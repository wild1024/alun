//! 迁移事务执行器。
//!
//! 把一条迁移的脚本拆成语句序列，按**事务模型三档**（设计文档 §4）执行：
//!
//! | 模式 | 行为 |
//! |---|---|
//! | 默认 | 全部语句 + 版本记录同事务，失败整体回滚、记录不落表 |
//! | `-- +no-transaction` | 语句逐条自动提交，结束后单独写记录；失败写 `success=false` 行 |
//!
//! 不使用驱动的 `execute_script`：它仅 PG / SQLite 实现、MySQL 返回
//! [`DbError::UnsupportedFeature`]，且无法进入迁移事务。这里改为在事务内**逐条**
//! 执行拆分器产出的语句（[`super::splitter`]）。

use std::time::Instant;

use alun_db_core::error::{DbError, Result};

use crate::db::Db;
use crate::tx::Transaction;

use super::MigratorOptions;
use super::history::{self, RecordWrite};
use super::kind::{AppliedVia, DialectKind};
use super::loader::LoadedMigration;
use super::splitter::{parse_directives, split_statements};

/// 一次迁移执行的统计结果。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExecutionOutcome {
    /// 实际执行的语句数。
    pub statement_count: usize,
    /// 执行耗时（毫秒）。
    pub execution_ms: i64,
}

/// 执行正向迁移（up），并在成功后写入记录。
///
/// # 参数
///
/// - `db`：数据库句柄。
/// - `migration`：待执行的迁移（已加载）。
/// - `options`：执行选项（是否拆分语句、默认是否包事务）。
/// - `seq`：本迁移的应用序号（回滚 LIFO 依据）。
/// - `checksum`：本次运算得到的 `up` 内容指纹。
///
/// # 返回
///
/// 执行统计（语句数与耗时）。
///
/// # 错误
///
/// 脚本无可执行语句、拆分失败、任一语句执行失败、写记录失败时返回
/// [`DbError`]。默认事务模式下失败整体回滚；`-- +no-transaction` 模式下已提交
/// 的语句无法回滚，失败会额外写一行 `success=false` 记录（尽力而为，写失败只告警）。
pub(crate) async fn apply_up(
    db: &Db,
    migration: &LoadedMigration,
    options: &MigratorOptions,
    seq: i64,
    checksum: &str,
) -> Result<ExecutionOutcome> {
    if is_transactional(migration, options, &migration.up) {
        // 默认事务模式：单迁移独立事务（失败隔离、断点续跑）。核心逻辑与
        // 「单事务整批」模式（`single_transaction`）共用 [`apply_up_in_tx`]。
        return db
            .transaction(|tx| async move {
                apply_up_in_tx(&tx, migration, options, seq, checksum).await
            })
            .await;
    }

    let dialect = DialectKind::from_dialect_name(db.dialect().name());
    let started = Instant::now();
    let statements = executable_statements(&migration.up, dialect, options, &migration.name)?;
    for sql in &statements {
        if let Err(e) = db.execute_with_params(sql, vec![]).await {
            // 非事务模式下已提交的语句无法回滚：写失败记录供 repair 清理，
            // 写记录本身失败只告警，绝不覆盖原始的迁移错误。
            let record = failure_record(migration, seq, checksum, elapsed_ms(started));
            if let Err(record_err) = write_record(db, &record).await {
                tracing::warn!(
                    migration = %migration.name,
                    "写入失败迁移记录时出错（原始错误仍会上抛）: {record_err}"
                );
            }
            return Err(e);
        }
    }
    let record = success_record(migration, seq, checksum, elapsed_ms(started));
    write_record(db, &record).await?;
    Ok(ExecutionOutcome {
        statement_count: statements.len(),
        execution_ms: elapsed_ms(started),
    })
}

/// 在**给定事务内**执行一条迁移的正向脚本并写入记录（不管理事务边界）。
///
/// 供两条路径复用：[`apply_up`] 的单迁移默认事务，以及 `single_transaction`
/// 模式下「多个迁移共用一个事务」的整批执行——两条路径共用同一套执行与
/// 记账逻辑，不存在「整批执行行为漂移」的可能。
///
/// # 错误
///
/// 脚本无可执行语句、拆分失败、任一语句执行失败、写记录失败时返回
/// [`DbError`]；由调用方决定回滚方式（单事务整批回滚 / 逐条模式由闭包回滚）。
pub(crate) async fn apply_up_in_tx(
    tx: &Transaction,
    migration: &LoadedMigration,
    options: &MigratorOptions,
    seq: i64,
    checksum: &str,
) -> Result<ExecutionOutcome> {
    let dialect = DialectKind::from_dialect_name(tx.dialect().name());
    // 迁移事务的同步提交设置（PG）：值会拼进 `SET LOCAL`（无法参数化），先严格白名单校验。
    let sc_setting: Option<String> = match options.synchronous_commit.as_deref() {
        Some(value) => {
            validate_synchronous_commit(value)?;
            Some(value.to_string())
        }
        None => None,
    };
    let started = Instant::now();

    // 语句拆分（完整词法分析）**延迟到实际需要时**：整段执行路径（驱动支持
    // multi-statement 协议，如 PG）根本不需要拆分，语句计数只进 DEBUG 日志——
    // 仅当日志启用时才补拆；逐条路径必须拆分。原先对所有迁移提前拆分的做法，
    // 在「整段执行 + info 日志」（生产默认）下是纯浪费。
    let statement_count = if tx.supports_script().await {
        // 优先整段执行（驱动支持 multi-statement 协议时，如 PG simple query）：
        // 迁移的语句互不相同、无参数，逐条 prepare+execute 会产生
        // `O(语句数 × 2)` 次往返；整段一次发送只有 O(1) 次（实测全量迁移减半）。
        // 驱动不支持时回退逐条——此时事务尚未执行任何语句，仍然有效。
        // 同步提交设置**拼进整段脚本**（而非单独一次 `SET LOCAL` 往返）：
        // 该设置在事务内生效、提交后自动恢复；每个迁移因此省一次往返。
        match &sc_setting {
            Some(sc) => {
                let script = format!("SET LOCAL synchronous_commit = {sc};\n{}", migration.up);
                tx.execute_script(&script).await?;
            }
            None => tx.execute_script(&migration.up).await?,
        }
        if tracing::enabled!(tracing::Level::DEBUG) {
            executable_statements(&migration.up, dialect, options, &migration.name)?.len()
        } else {
            0
        }
    } else {
        // 逐条路径无法拼脚本：`SET LOCAL` 单独执行（每迁移一次）。
        if let Some(sc) = &sc_setting {
            tx.execute(&format!("SET LOCAL synchronous_commit = {sc}"))
                .await?;
        }
        let statements = executable_statements(&migration.up, dialect, options, &migration.name)?;
        for sql in &statements {
            tx.execute_with_params(sql, vec![]).await?;
        }
        statements.len()
    };
    // 记录写入走 simple query 快路径（支持 script 的驱动省去 PREPARE + 参数绑定往返）。
    let record = success_record(migration, seq, checksum, elapsed_ms(started));
    history::write_apply_record(tx, &record).await?;
    Ok(ExecutionOutcome {
        statement_count,
        execution_ms: elapsed_ms(started),
    })
}

/// 执行回退迁移（down），并在成功后删除记录。
///
/// `down` 脚本为空时显式报错（单向迁移无法回滚），绝不静默跳过。
///
/// # 参数
///
/// - `db`：数据库句柄。
/// - `migration`：待回退的迁移（已加载）。
/// - `options`：执行选项。
///
/// # 返回
///
/// 执行统计（语句数与耗时）。
///
/// # 错误
///
/// 无 `down` 脚本、拆分失败、任一语句执行失败、删记录失败时返回 [`DbError`]。
pub(crate) async fn apply_down(
    db: &Db,
    migration: &LoadedMigration,
    options: &MigratorOptions,
) -> Result<ExecutionOutcome> {
    if migration.down.trim().is_empty() {
        return Err(DbError::Migration(format!(
            "migration '{}' has no down script; cannot roll back",
            migration.name
        )));
    }
    let dialect = DialectKind::from_dialect_name(db.dialect().name());
    let statements = executable_statements(&migration.down, dialect, options, &migration.name)?;
    let script = statements.as_slice();
    let statement_count = statements.len();
    let is_transaction = is_transactional(migration, options, &migration.down);
    let started = Instant::now();

    if is_transaction {
        db.transaction(|tx| async move {
            for sql in script {
                tx.execute_with_params(sql, vec![]).await?;
            }
            history::delete_in_tx(&tx, migration.kind, &migration.name).await
        })
        .await?;
    } else {
        for sql in script {
            db.execute_with_params(sql, vec![]).await?;
        }
        db.transaction(|tx| async move {
            history::delete_in_tx(&tx, migration.kind, &migration.name).await
        })
        .await?;
    }

    Ok(ExecutionOutcome {
        statement_count,
        execution_ms: elapsed_ms(started),
    })
}

/// 在给定事务内执行一条迁移的 up 语句（不写记录、不管理事务）。
///
/// 供 `dry_run` 使用：整批预演共用**一个**事务，因此后一条迁移能看到前一条
/// 建出的表——与真实执行时的可见性一致。
///
/// # 错误
///
/// 脚本无可执行语句、拆分失败或任一语句执行失败时返回 [`DbError`]。
pub(crate) async fn execute_up_in_tx(
    tx: &Transaction,
    migration: &LoadedMigration,
    options: &MigratorOptions,
) -> Result<ExecutionOutcome> {
    let dialect = DialectKind::from_dialect_name(tx.dialect().name());
    let statements = executable_statements(&migration.up, dialect, options, &migration.name)?;
    let statement_count = statements.len();
    let started = Instant::now();

    for sql in &statements {
        tx.execute_with_params(sql, vec![]).await.map_err(|e| {
            DbError::Migration(format!(
                "dry run failed on migration '{}': {e}",
                migration.name
            ))
        })?;
    }

    Ok(ExecutionOutcome {
        statement_count,
        execution_ms: elapsed_ms(started),
    })
}

/// 把脚本拆成可执行语句序列；拆分关闭时整段脚本作为一条语句。
///
/// 空脚本（仅注释 / 空白）视为配置错误显式报错：一条什么都不做的迁移几乎必然
/// 是文件写错，静默通过会掩盖问题。
fn executable_statements(
    sql: &str,
    dialect: DialectKind,
    options: &MigratorOptions,
    migration_name: &str,
) -> Result<Vec<String>> {
    let statements = if options.split_statements {
        // `-- +delimiter` 由拆分器内部就近解析并覆盖默认的 `;`
        split_statements(sql, dialect)?
    } else {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            Vec::new()
        } else {
            vec![trimmed.to_string()]
        }
    };

    if statements.is_empty() {
        return Err(DbError::Migration(format!(
            "migration '{migration_name}' contains no executable statements"
        )));
    }
    Ok(statements)
}

/// 本迁移是否应包裹事务。
///
/// 三处开关取合取：全局默认（`transaction_default`）、加载期记录的
/// `-- +no-transaction`、脚本内就近声明的 `-- +no-transaction`。
pub(crate) fn is_transactional(
    migration: &LoadedMigration,
    options: &MigratorOptions,
    script: &str,
) -> bool {
    options.transaction_default
        && !migration.is_no_transaction
        && !parse_directives(script).is_no_transaction
}

/// 构造一条成功记录。
fn success_record(
    migration: &LoadedMigration,
    seq: i64,
    checksum: &str,
    execution_ms: i64,
) -> RecordWrite {
    record(migration, seq, checksum, execution_ms, true)
}

/// 构造一条失败记录（仅非事务模式使用）。
fn failure_record(
    migration: &LoadedMigration,
    seq: i64,
    checksum: &str,
    execution_ms: i64,
) -> RecordWrite {
    record(migration, seq, checksum, execution_ms, false)
}

/// 构造记录写入字段集合。
fn record(
    migration: &LoadedMigration,
    seq: i64,
    checksum: &str,
    execution_ms: i64,
    is_success: bool,
) -> RecordWrite {
    RecordWrite {
        kind: migration.kind,
        name: migration.name.clone(),
        version: migration.version.clone(),
        checksum: Some(checksum.to_string()),
        // 本框架亲自执行的 `up`，指纹即所执行内容的哈希
        applied_via: AppliedVia::Executed,
        seq,
        execution_ms: Some(execution_ms),
        is_success,
        source: migration
            .source
            .as_ref()
            .map(|path| path.display().to_string()),
    }
}

/// 在独立事务中写入一条记录。
async fn write_record(db: &Db, record: &RecordWrite) -> Result<()> {
    db.transaction(|tx| async move { history::upsert_in_tx(&tx, record).await })
        .await
}

/// 从起点到当前的毫秒数（饱和到 `i64::MAX`，避免超长迁移溢出）。
fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}

/// 校验 `synchronous_commit` 配置值。
///
/// 该值会被拼进 `SET LOCAL synchronous_commit = <value>`（SET 语句无法参数化），
/// 因此只接受 ASCII 字母、数字与 `_` 的严格白名单——`off` / `on` / `remote_write`
/// 等合法取值全部通过，任何可构造注入的字符都会被拒绝。
fn validate_synchronous_commit(value: &str) -> Result<()> {
    if !value.is_empty() && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Ok(());
    }
    Err(DbError::InvalidValue(format!(
        "invalid synchronous_commit value '{value}': only ASCII letters, digits and '_' are allowed"
    )))
}

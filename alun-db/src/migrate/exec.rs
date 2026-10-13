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
    let dialect = DialectKind::from_dialect_name(db.dialect().name());
    let statements = executable_statements(&migration.up, dialect, options, &migration.name)?;
    // 以切片捕获进闭包（`&[String]` 是 Copy），避免整个 Vec 被 move 走
    let script = statements.as_slice();
    let statement_count = statements.len();
    let is_transaction = is_transactional(migration, options, &migration.up);
    let started = Instant::now();

    if is_transaction {
        db.transaction(|tx| async move {
            for sql in script {
                tx.execute_with_params(sql, vec![]).await?;
            }
            let record = success_record(migration, seq, checksum, elapsed_ms(started));
            history::upsert_in_tx(&tx, &record).await
        })
        .await?;
    } else {
        for sql in script {
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
    }

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

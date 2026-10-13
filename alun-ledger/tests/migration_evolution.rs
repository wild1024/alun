//! M8 Schema 演进（用例 MIG-001..MIG-012）。
//!
//! 与被测实现的对照（《测试项目测试规划.md》§3.2 / §6.8）：
//! - runner 只接受 `V{n}__Name.{up,down}.sql`（或单向 `V{n}__Name.sql`）、
//!   **每文件单语句**、**目录不递归**；
//! - 迁移目录被拆成 `<dialect>/01-base` → `common` → `<dialect>/02-evolve`
//!   三段，版本号全局唯一（1..=10 common、11..=20 base、21..=27 evolve）；
//! - 回滚（[`Migrator::rollback`]）的 LIFO 依据是「已应用的最大版本号」，而不是
//!   应用顺序（见 MIG-002 的说明与缺陷登记）。
//!
//! 临时文件一律落在 `CARGO_TARGET_TMPDIR`（`target/` 下），绝不改动 `migrations/`。

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use alun_ledger::services::rowx;

use common::assertions::count_by;
use common::harness::{Dialect, connect, ctx};

/// 全部版本号（三段目录合计 27 版）。
fn all_versions() -> BTreeSet<i64> {
    (1..=27).collect()
}

/// 方言 base DDL 建的表（`migrations/<dialect>/01-base`）。
const BASE_TABLES: [&str; 10] = [
    "tenants",
    "users",
    "accounts",
    "transfers",
    "ledger_entries",
    "invoices",
    "invoice_items",
    "subscriptions",
    "usage_records",
    "audit_logs",
];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn migrations_root() -> PathBuf {
    manifest_dir().join("migrations")
}

/// 三段迁移目录（顺序 = 应用顺序）。
fn dir_paths(d: Dialect) -> Vec<PathBuf> {
    let root = migrations_root();
    let dd = root.join(d.migration_dir());
    vec![
        dd.join("01-base"),
        root.join("common"),
        dd.join("02-evolve"),
    ]
}

/// 目标目录下的临时目录（先清空）。
fn tmp_dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create tmp dir");
    d
}

/// 已应用版本集合。
///
/// `schema_migrations.version` 为通用版本串（VARCHAR），本项目全部为纯数字版本，
/// 故在此解析回 `i64` 以便与 [`all_versions`] 等期望集合直接比较。
async fn applied_versions(db: &Db) -> BTreeSet<i64> {
    db.find("SELECT version FROM schema_migrations ORDER BY version")
        .await
        .expect("read schema_migrations")
        .iter()
        .map(|r| {
            let raw = String::from_value(r.values().next().expect("version column"))
                .expect("version is text");
            raw.parse::<i64>()
                .unwrap_or_else(|e| panic!("version '{raw}' is not numeric: {e}"))
        })
        .collect()
}

/// 方言 catalog：关系（表/视图）→ 列名（按 ordinal 序）。
async fn relation_columns(db: &Db, d: Dialect) -> BTreeMap<String, Vec<String>> {
    let sql = match d {
        Dialect::SqliteMem | Dialect::SqliteFile => {
            "SELECT m.name AS t, p.name AS c FROM sqlite_master m \
             JOIN pragma_table_info(m.name) p \
             WHERE m.type IN ('table', 'view') AND m.name NOT LIKE 'sqlite_%' \
             ORDER BY m.name, p.cid"
        }
        Dialect::Postgres => {
            "SELECT table_name AS t, column_name AS c FROM information_schema.columns \
             WHERE table_schema = current_schema() ORDER BY table_name, ordinal_position"
        }
        Dialect::Mysql => {
            "SELECT table_name AS t, column_name AS c FROM information_schema.columns \
             WHERE table_schema = DATABASE() ORDER BY table_name, ordinal_position"
        }
    };
    let rows = db.find(sql).await.expect("read catalog");
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in &rows {
        out.entry(catalog_text(r, "t"))
            .or_default()
            .push(catalog_text(r, "c"));
    }
    out
}

/// `information_schema` 的文本列在 MySQL 上以 `Value::Bytes` 返回，需容错解码。
fn catalog_text(row: &alun_db::prelude::Row, col: &str) -> String {
    use alun_db_core::stmt::Value;
    match rowx::value(row, col) {
        Ok(Value::Text(s)) => s.clone(),
        Ok(Value::Bytes(b)) => String::from_utf8_lossy(b).to_string(),
        Ok(Value::Enum(s)) => s.clone(),
        other => panic!("catalog 列 `{col}` 无法解码: {other:?}"),
    }
}

/// 表是否存在（按方言 catalog）。
async fn table_exists(db: &Db, d: Dialect, name: &str) -> bool {
    let sql = match d {
        Dialect::SqliteMem | Dialect::SqliteFile => {
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?"
        }
        Dialect::Postgres => {
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_name = ?"
        }
        Dialect::Mysql => {
            "SELECT table_name FROM information_schema.tables \
             WHERE table_schema = DATABASE() AND table_name = ?"
        }
    };
    !db.find_with_params(sql, vec![Value::Text(name.to_string())])
        .await
        .expect("table_exists")
        .is_empty()
}

/// 索引名集合。
async fn index_names(db: &Db, d: Dialect) -> BTreeSet<String> {
    let sql = match d {
        Dialect::SqliteMem | Dialect::SqliteFile => {
            "SELECT name FROM sqlite_master WHERE type = 'index'"
        }
        Dialect::Postgres => {
            "SELECT indexname AS name FROM pg_indexes WHERE schemaname = current_schema()"
        }
        Dialect::Mysql => {
            "SELECT DISTINCT index_name AS name FROM information_schema.statistics \
             WHERE table_schema = DATABASE()"
        }
    };
    db.find(sql)
        .await
        .expect("index names")
        .iter()
        .map(|r| rowx::text_of(r, "name").expect("index name"))
        .collect()
}

/// 取某表某列的类型名（SQLite `pragma_table_info`）。
async fn column_type(db: &Db, table: &str, column: &str) -> String {
    let rows = db
        .find_with_params(
            "SELECT p.type AS ty FROM sqlite_master m JOIN pragma_table_info(m.name) p \
             WHERE m.name = ? AND p.name = ?",
            vec![
                Value::Text(table.to_string()),
                Value::Text(column.to_string()),
            ],
        )
        .await
        .expect("column type");
    rowx::text_of(rows.first().expect("column exists"), "ty").expect("type text")
}

/// 按顶层分号切分 SQL（忽略字符串字面量内的分号；`--` 行注释先剔除）。
fn statements(sql: &str) -> Vec<String> {
    let cleaned: String = sql
        .lines()
        .filter(|l| !l.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    let mut chars = cleaned.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_str => {
                in_str = true;
                cur.push(ch);
            }
            '\'' => {
                // `''` 为转义引号，不结束字面量
                if chars.peek() == Some(&'\'') {
                    cur.push(ch);
                    cur.push(chars.next().expect("peeked"));
                } else {
                    in_str = false;
                    cur.push(ch);
                }
            }
            ';' if !in_str => {
                let seg = cur.trim();
                if !seg.is_empty() {
                    out.push(seg.to_string());
                }
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    let tail = cur.trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

/// 递归收集目录下的 `*.up.sql` / `*.sql` 迁移源文件。
fn collect_sql_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read dir") {
        let p = e.expect("dir entry").path();
        if p.is_dir() {
            collect_sql_files(&p, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some("sql") {
            out.push(p);
        }
    }
}

/// `02-evolve` 的静态迁移清单。
///
/// [`Migration`] 的 SQL 字段是 `String`，这里用 [`include_str!`] 在**编译期**
/// 嵌入 `migrations/<dialect>/02-evolve/*.sql`——保证与文件系统逐字一致（编译器
/// 保证），运行期由 [`Migration::new`] 拷贝为拥有型字符串。
macro_rules! evolve_manifest {
    ($dir:literal, $v26:literal, $v27:literal) => {
        vec![
            Migration::new(
                21,
                "users_split_name_backfill",
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0021__users_split_name_backfill.up.sql"
                )),
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0021__users_split_name_backfill.down.sql"
                )),
            ),
            Migration::new(
                22,
                "uq_subscriptions_period",
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0022__uq_subscriptions_period.up.sql"
                )),
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0022__uq_subscriptions_period.down.sql"
                )),
            ),
            Migration::new(
                23,
                "idx_usage_sub_date",
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0023__idx_usage_sub_date.up.sql"
                )),
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0023__idx_usage_sub_date.down.sql"
                )),
            ),
            Migration::new(
                24,
                "idx_audit_tenant_time",
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0024__idx_audit_tenant_time.up.sql"
                )),
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0024__idx_audit_tenant_time.down.sql"
                )),
            ),
            Migration::new(
                25,
                "view_v_mrr",
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0025__view_v_mrr.up.sql"
                )),
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0025__view_v_mrr.down.sql"
                )),
            ),
            Migration::new(
                26,
                $v26,
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0026__",
                    $v26,
                    ".up.sql"
                )),
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0026__",
                    $v26,
                    ".down.sql"
                )),
            ),
            Migration::new(
                27,
                $v27,
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0027__",
                    $v27,
                    ".up.sql"
                )),
                include_str!(concat!(
                    "../migrations/",
                    $dir,
                    "/02-evolve/V0027__",
                    $v27,
                    ".down.sql"
                )),
            ),
        ]
    };
}

/// 当前方言的 `02-evolve` 静态清单（与 [`Migrator::from_dir`] 的目录内容同源）。
fn static_migrations(d: Dialect) -> Vec<Migration> {
    let list = match d {
        Dialect::SqliteMem | Dialect::SqliteFile => {
            evolve_manifest!("sqlite", "audit_fts_table", "invoices_tenant_status_index")
        }
        Dialect::Postgres => evolve_manifest!("postgres", "citext_extension", "users_add_email_ci"),
        Dialect::Mysql => evolve_manifest!(
            "mysql",
            "audit_fulltext_index",
            "invoices_tenant_status_index"
        ),
    };
    // 与磁盘内容逐字一致（防止 include_str! 路径写错）
    let dir = migrations_root().join(d.migration_dir()).join("02-evolve");
    for m in &list {
        let on_disk = std::fs::read_to_string(dir.join(format!(
            "V{:0>4}__{}.up.sql",
            m.version.as_str(),
            m.name
        )));
        if let Ok(on_disk) = on_disk {
            assert_eq!(m.up.trim(), on_disk.trim(), "v{} 内容漂移", m.version);
        }
    }
    list
}

// ─────────────────────────── MIG-001 ───────────────────────────

/// MIG-001：三段目录全量 apply（27 版），`schema_migrations` 行数与版本集合一致；
/// 重跑为 no-op（返回 0）。
#[tokio::test]
async fn mig_001_full_apply_27_versions_then_noop() {
    let Some(c) = ctx().await else { return };

    // ctx() 已完成「01-base → common → 02-evolve」全量 apply
    let set = applied_versions(c.db()).await;
    assert_eq!(set, all_versions(), "三段目录应合计 27 版且版本号连续");
    let rows = count_by(c.db(), "SELECT COUNT(*) FROM schema_migrations", vec![]).await;
    assert_eq!(rows, 27, "schema_migrations 行数");

    // 重跑：无新版本
    for dir in dir_paths(c.dialect) {
        let n = Migrator::from_dir(&dir)
            .run(c.db())
            .await
            .unwrap_or_else(|e| panic!("重跑 {} 失败: {e}", dir.display()));
        assert_eq!(n, 0, "{} 重跑应为 no-op", dir.display());
    }

    // 全新库上的「全量 apply」计数只在 sqlite-mem 可测（内存库按连接隔离）
    if c.dialect == Dialect::SqliteMem {
        let fresh = connect(Dialect::SqliteMem).await.expect("fresh sqlite-mem");
        let mut counts = Vec::new();
        for dir in dir_paths(c.dialect) {
            counts.push(Migrator::from_dir(&dir).run(&fresh).await.expect("apply"));
        }
        assert_eq!(
            counts,
            vec![10, 10, 7],
            "01-base / common / 02-evolve 应用数"
        );
        assert_eq!(applied_versions(&fresh).await, all_versions());
        for dir in dir_paths(c.dialect) {
            assert_eq!(
                Migrator::from_dir(&dir)
                    .run(&fresh)
                    .await
                    .expect("re-apply"),
                0
            );
        }
    } else {
        eprintln!(
            "[MIG-001] {} 为共享库，跳过「全新库应用计数」分支（其余断言已覆盖）",
            c.dialect.name()
        );
    }
}

// ─────────────────────────── MIG-002 ───────────────────────────

/// MIG-002：[`Migrator::rollback`] 的 LIFO 回滚 + 重新 apply 往返，
/// 表/列集合必须完全一致。
#[tokio::test]
async fn mig_002_down_up_roundtrip_preserves_schema() {
    let Some(c) = ctx().await else { return };
    let evolve = migrations_root()
        .join(c.dialect.migration_dir())
        .join("02-evolve");

    let before = relation_columns(c.db(), c.dialect).await;
    assert!(before.contains_key("invoices"), "前置：表结构应已就绪");

    // 逐版本回滚：返回的版本号必须严格递减（LIFO = 已应用最大版本）
    let mut rolled = Vec::new();
    for _ in 0..7 {
        let v = Migrator::from_dir(&evolve)
            .rollback(c.db())
            .await
            .expect("down")
            .expect("还应有一版可回滚");
        rolled.push(v.to_string());
    }
    assert_eq!(
        rolled.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["27", "26", "25", "24", "23", "22", "21"],
        "LIFO 应按版本号递减"
    );
    assert_eq!(
        applied_versions(c.db()).await,
        (1..=20).collect::<BTreeSet<_>>(),
        "回滚 7 版后仅剩 base + common"
    );

    let mid = relation_columns(c.db(), c.dialect).await;
    assert_ne!(mid, before, "回滚 v_mrr / audit_fts 等对象后表列集合应变化");

    // 重新 apply 后与回滚前完全一致
    let n = Migrator::from_dir(&evolve)
        .run(c.db())
        .await
        .expect("re-apply");
    assert_eq!(n, 7);
    assert_eq!(
        relation_columns(c.db(), c.dialect).await,
        before,
        "down → up 往返后表/列集合必须一致"
    );

    // 静态清单路径：`Migrator::rollback` / `Migrator::run`
    let list = static_migrations(c.dialect);
    assert_eq!(list.len(), 7);
    let down = Migrator::from_migrations(list.clone())
        .rollback(c.db())
        .await
        .expect("rollback")
        .expect("应回滚 1 版");
    assert_eq!(down.as_str(), "27");
    assert_eq!(
        Migrator::from_migrations(list)
            .run(c.db())
            .await
            .expect("run"),
        1
    );
    assert_eq!(
        relation_columns(c.db(), c.dialect).await,
        before,
        "静态清单往返后表/列集合必须一致"
    );

    // 空表语义：回滚到没有可回滚版本时返回 Ok(None)
    let empty = tmp_dir("mig002_empty");
    let fresh = connect(Dialect::SqliteMem).await;
    if let Some(fresh) = fresh {
        assert!(
            Migrator::from_dir(&empty)
                .rollback(&fresh)
                .await
                .expect("empty dir")
                .is_none()
        );
    }

    // 跨目录回滚的边界（缺陷登记）：LIFO 按**版本号**而非**应用顺序**，
    // 因此「回滚到 0」在目录拆分下不可达——common 的 down 会打在已被
    // base down 删除的表上。用独立内存库复现，不动用例其余状态。
    if c.dialect == Dialect::SqliteMem {
        let db = connect(Dialect::SqliteMem).await.expect("fresh sqlite-mem");
        let root = migrations_root();
        let base = root.join(c.dialect.migration_dir()).join("01-base");
        let common = root.join("common");
        for dir in dir_paths(c.dialect) {
            Migrator::from_dir(&dir).run(&db).await.expect("apply");
        }
        for _ in 0..7 {
            Migrator::from_dir(&evolve)
                .rollback(&db)
                .await
                .expect("down evolve")
                .expect("evolve 应有可回滚版本");
        }

        // 逆序之外的目录不得回滚：common 仍应用时 down base 必须被拒绝/无操作，
        // 且不得按版本号硬删（否则会把 common 依赖的表删掉）
        let out_of_order = Migrator::from_dir(&base).rollback(&db).await;
        assert!(
            !matches!(&out_of_order, Ok(Some(v)) if v.as_str().parse::<i64>().is_ok_and(|n| n <= 10)),
            "common 仍应用时不得回滚 base（应用顺序 = base→common→evolve，D022 已修复）：{out_of_order:?}"
        );
        assert!(
            table_exists(&db, Dialect::SqliteMem, "tenants").await,
            "越序回滚不得破坏仍被依赖的表"
        );

        // 严格逆序：evolve → common → base，可回滚到 0
        for _ in 0..10 {
            Migrator::from_dir(&common)
                .rollback(&db)
                .await
                .expect("down common")
                .expect("common 应有可回滚版本");
        }
        for _ in 0..10 {
            Migrator::from_dir(&base)
                .rollback(&db)
                .await
                .expect("down base")
                .expect("base 应有可回滚版本");
        }
        assert_eq!(
            applied_versions(&db).await,
            BTreeSet::new(),
            "按应用顺序逆序回滚应到达 0 版"
        );
        eprintln!(
            "[MIG-002] 跨目录回滚按应用顺序（base→common→evolve 逆序）到达 0 版；\
             越序回滚被拒绝且不破坏依赖表（D022 已修复）"
        );
    } else {
        eprintln!(
            "[MIG-002] {} 为共享库，跳过跨目录回滚边界探针（避免破坏共享 schema）",
            c.dialect.name()
        );
    }
}

// ─────────────────────────── MIG-003 ───────────────────────────

/// MIG-003：篡改**已应用**的迁移文件后重跑 → 现状静默通过（无 checksum）。
/// 用例断言「观察到的行为」（`Ok(0)` 且改动从未生效），并登记缺陷 G6。
#[tokio::test]
async fn mig_003_tampered_applied_migration_is_detected() {
    let Some(c) = ctx().await else { return };
    let src = migrations_root()
        .join(c.dialect.migration_dir())
        .join("01-base");
    let dir = tmp_dir("mig003_tampered");
    // 复制一个**已应用**的版本（V0011 tenants）
    for suffix in ["up", "down"] {
        std::fs::copy(
            src.join(format!("V0011__create_tenants.{suffix}.sql")),
            dir.join(format!("V0011__create_tenants.{suffix}.sql")),
        )
        .expect("copy migration");
    }

    // 未篡改：已在 schema_migrations 中 → 0
    assert_eq!(
        Migrator::from_dir(&dir).run(c.db()).await.expect("no-op"),
        0,
        "已应用版本应被跳过"
    );

    // 篡改 up 内容（改成建一张完全不同的表）后重跑
    std::fs::write(
        dir.join("V0011__create_tenants.up.sql"),
        "CREATE TABLE mig003_tampered_probe (id BIGINT NOT NULL PRIMARY KEY)",
    )
    .expect("tamper");
    // 已应用迁移的内容被改写 → 必须显式报错（D011 已修复：schema_migrations 增 checksum）
    let tampered = Migrator::from_dir(&dir).run(c.db()).await;
    let msg = match &tampered {
        Ok(v) => panic!("已应用迁移被篡改必须报错，实际返回 Ok({v})"),
        Err(e) => e.to_string(),
    };
    assert!(
        ["checksum", "校验和", "篡改", "内容不一致", "不一致"]
            .iter()
            .any(|k| msg.contains(k)),
        "错误文案应指明内容指纹不一致，实际 `{msg}`"
    );
    assert!(
        !table_exists(c.db(), c.dialect, "mig003_tampered_probe").await,
        "篡改后的 DDL 不应被执行"
    );
    let recorded: String = db_scalar_text(
        c.db(),
        "SELECT name FROM schema_migrations WHERE version = 11",
    )
    .await;
    assert_eq!(
        recorded, "create_tenants",
        "库中记录的名字仍是原文件（篡改不可见）"
    );

    eprintln!(
        "[DEFECT G6] 迁移无校验和：篡改已应用迁移文件后 `Migrator::run` 返回 Ok(0)、静默通过，\
         库内既未发现篡改也未重放（`schema_migrations(version,name,applied_at)` 无 checksum 列）。\
         期望：内容指纹不一致时报错并要求人工介入。"
    );
}

/// 取单个文本标量。
async fn db_scalar_text(db: &Db, sql: &str) -> String {
    db.scalar::<String>(sql).await.expect("scalar text")
}

// ─────────────────────────── MIG-004 ───────────────────────────

/// MIG-004：单文件单语句约束——每个 up 文件恰含 1 条语句，且三段目录 ≥ 26 个文件。
#[test]
fn mig_004_each_up_file_holds_exactly_one_statement() {
    let root = migrations_root();
    let mut files = Vec::new();
    collect_sql_files(&root, &mut files);
    assert!(!files.is_empty(), "未扫到迁移文件: {}", root.display());

    let mut up_files = 0usize;
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        if !name.ends_with(".up.sql") {
            continue;
        }
        up_files += 1;
        let sql = std::fs::read_to_string(f).expect("read migration");
        let stmts = statements(&sql);
        assert_eq!(
            stmts.len(),
            1,
            "{} 含 {} 条语句（runner 要求每文件单语句）",
            f.display(),
            stmts.len()
        );
    }
    assert!(up_files >= 26, "up 文件数 {up_files} < 26");

    // 当前方言目录 + common 的版本数（= 27）
    let mut own = Vec::new();
    for dir in dir_paths(dialect_of_test()) {
        collect_sql_files(&dir, &mut own);
    }
    let own_up: Vec<&PathBuf> = own
        .iter()
        .filter(|p| p.to_string_lossy().ends_with(".up.sql"))
        .collect();
    assert_eq!(own_up.len(), 27, "三段目录应共 27 个 up 文件");
    eprintln!(
        "[MIG-004] up 文件：全仓 {up_files}，当前方言+common {}；单语句约束全部成立",
        own_up.len()
    );
}

/// 测试进程的目标方言（与 harness 一致）。
fn dialect_of_test() -> Dialect {
    common::harness::dialect()
}

// ─────────────────────────── MIG-005 ───────────────────────────

/// MIG-005：重复版本号 / 非法文件名 / 缺配 down → 显式报错。
#[tokio::test]
async fn mig_005_duplicate_version_and_bad_filenames() {
    let Some(c) = ctx().await else { return };
    let db = c.db();

    // A. 静态清单重复版本 → InvalidValue
    let dup = [
        Migration::new(9001, "a", "SELECT 1", ""),
        Migration::new(9001, "b", "SELECT 1", ""),
    ];
    let err = Migrator::from_migrations(dup)
        .run(db)
        .await
        .expect_err("重复版本必须报错");
    assert!(matches!(err, DbError::InvalidValue(_)), "实际 {err:?}");

    // B. 非法文件名 → Migration
    let d = tmp_dir("mig005_badname");
    std::fs::write(d.join("not_a_migration.sql"), "SELECT 1").unwrap();
    let err = Migrator::from_dir(&d)
        .run(db)
        .await
        .expect_err("非法文件名必须报错");
    assert!(matches!(err, DbError::Migration(_)), "实际 {err:?}");
    assert!(err.to_string().contains("unparseable"), "实际 {err}");

    // C. 目录内重复版本 → Migration
    let d = tmp_dir("mig005_dup");
    std::fs::write(d.join("V9001__a.up.sql"), "SELECT 1").unwrap();
    std::fs::write(d.join("V9001__b.up.sql"), "SELECT 1").unwrap();
    let err = Migrator::from_dir(&d)
        .run(db)
        .await
        .expect_err("重复版本必须报错");
    assert!(matches!(err, DbError::Migration(_)), "实际 {err:?}");
    assert!(err.to_string().contains("duplicate"), "实际 {err}");

    // D. 只有 down 没有 up → Migration（绝不静默跳过）
    let d = tmp_dir("mig005_down_only");
    std::fs::write(d.join("V9002__only.down.sql"), "SELECT 1").unwrap();
    let err = Migrator::from_dir(&d)
        .run(db)
        .await
        .expect_err("缺配 up 必须报错");
    assert!(matches!(err, DbError::Migration(_)), "实际 {err:?}");

    // E. 非 .sql 文件被跳过；合法单向文件可正常执行（用例自身清理，避免污染共享库）
    let d = tmp_dir("mig005_skip");
    std::fs::write(d.join("README.md"), "not sql").unwrap();
    let ddl = "CREATE TABLE mig005_probe (id INTEGER NOT NULL PRIMARY KEY)";
    std::fs::write(d.join("V9003__ok.up.sql"), ddl).unwrap();
    std::fs::write(d.join("V9003__ok.down.sql"), "DROP TABLE mig005_probe").unwrap();
    assert_eq!(Migrator::from_dir(&d).run(db).await.expect("apply"), 1);
    assert!(table_exists(db, c.dialect, "mig005_probe").await);
    // 单向缺失 down 时无法回滚（runner 显式报错而非静默）
    std::fs::remove_file(d.join("V9003__ok.down.sql")).unwrap();
    let err = Migrator::from_dir(&d)
        .rollback(db)
        .await
        .expect_err("缺 down 必须报错");
    assert!(matches!(err, DbError::Migration(_)), "实际 {err:?}");
    // 清理：删除版本记录 + 探针表
    db.execute_with_params(
        "DELETE FROM schema_migrations WHERE version = ?",
        vec![Value::I64(9003)],
    )
    .await
    .expect("cleanup version");
    db.execute("DROP TABLE mig005_probe")
        .await
        .expect("cleanup table");
    assert_eq!(applied_versions(db).await, all_versions());
}

// ─────────────────────────── MIG-006 ───────────────────────────

/// MIG-006：4 个并发 [`Migrator::run`] 任务无 panic，且版本不被重复应用。
/// 现状无咨询锁（G6 同类）→ 允许部分任务因锁冲突报错，断言收敛后的最终集合。
#[tokio::test]
async fn mig_006_concurrent_migrate_from_dir_applies_each_version_once() {
    let Some(c) = ctx().await else { return };
    let dirs = dir_paths(c.dialect);

    // sqlite 用文件库（内存库池容量为 1，无法真并发）
    let db = if c.dialect.is_sqlite() {
        let path = tmp_dir("mig006").join("concurrent.db");
        let url = format!("sqlite://{}", path.display());
        Db::connect(&url).await.expect("file db")
    } else {
        c.db().clone()
    };

    let mut handles = Vec::new();
    for i in 0..4usize {
        let db = db.clone();
        let dirs = dirs.clone();
        handles.push(tokio::spawn(async move {
            let mut applied = 0usize;
            let mut errs: Vec<String> = Vec::new();
            for d in &dirs {
                match Migrator::from_dir(d).run(&db).await {
                    Ok(n) => applied += n,
                    Err(e) => errs.push(format!("{}: {e}", d.display())),
                }
            }
            (i, applied, errs)
        }));
    }

    let mut total_applied = 0usize;
    let mut failed_tasks = 0usize;
    for h in handles {
        let (i, applied, errs) = h.await.expect("任务不得 panic");
        total_applied += applied;
        if !errs.is_empty() {
            failed_tasks += 1;
            eprintln!(
                "[MIG-006] 任务 {i} 报错 {} 次：{}",
                errs.len(),
                errs.join(" | ")
            );
        } else {
            eprintln!("[MIG-006] 任务 {i} 应用 {applied} 版");
        }
    }

    // 咨询锁（PG advisory / MySQL GET_LOCK / SQLite 进程内互斥）串行化后：
    // 四个任务都应成功，且同一版本恰好被应用一次（D011 已修复）
    assert_eq!(failed_tasks, 0, "并发迁移不应报错（咨询锁应串行化）");
    // 全新库（sqlite 文件库）上应恰好应用 27 版一次；共享库（PG/MySQL）上此前用例
    // 已迁移完毕 → 合计为 0 也正确。关键不变量是「不超过 27」且最终恰好 27 版。
    assert!(
        total_applied <= 27,
        "同一版本被重复应用：合计 {total_applied} > 27"
    );
    let after_wave = applied_versions(&db).await;
    assert!(after_wave.len() <= 27);
    let rows = count_by(&db, "SELECT COUNT(*) FROM schema_migrations", vec![]).await;
    assert_eq!(rows, after_wave.len() as i64, "版本表内不得出现重复行");

    // 串行补跑必须收敛到 27 版（并发期间丢失的版本由此补齐）
    for d in &dirs {
        Migrator::from_dir(d).run(&db).await.expect("串行补跑");
    }
    assert_eq!(
        applied_versions(&db).await.len(),
        27,
        "并发 + 补跑后必须恰好 27 版"
    );
    assert_eq!(applied_versions(&db).await, all_versions());

    if failed_tasks > 0 || total_applied < 27 {
        eprintln!(
            "[DEFECT 迁移无咨询锁] 4 个并发任务中 {failed_tasks} 个报错，\
             并发窗口内只应用 {total_applied}/27 版；必须外部串行或补跑才收敛。\
             期望：runner 内置 advisory lock / 唯一键重试。"
        );
    }
}

// ─────────────────────────── MIG-007 ───────────────────────────

/// MIG-007：快照解析器忽略 `DROP COLUMN`（G15）→ `#[model(strict)]` 假通过。
#[tokio::test]
async fn mig_007_snapshot_applies_dropped_column() {
    let Some(c) = ctx().await else { return };

    let snapshot_path = env!("ALUN_DB_SCHEMA_SNAPSHOT");
    let snapshot = std::fs::read_to_string(snapshot_path)
        .unwrap_or_else(|e| panic!("读取快照 {snapshot_path} 失败: {e}"));
    assert!(
        !snapshot.contains("legacy_id"),
        "快照必须反映已应用的 DROP COLUMN（D012 已修复），实际仍含 legacy_id：\n{snapshot}"
    );

    // 真实库中该列已被 V0005 删除 → 快照与真实 schema 不一致
    let users = relation_columns(c.db(), c.dialect).await;
    let cols = users.get("users").expect("users 表");
    assert!(
        !cols.iter().any(|c| c == "legacy_id"),
        "真实库 users 不应还有 legacy_id: {cols:?}"
    );

    eprintln!(
        "[MIG-007] 快照已按 V0005 的 `DROP COLUMN legacy_id` 同步（{snapshot_path}），\
         真实库同样无该列 → `#[model(strict)]` 不再对已删除列假通过；\
         反向验证：`cargo check -p alun-ledger --features strict_probe` 现在**必须编译失败**（悬空列被检出）。"
    );
}

// ─────────────────────────── MIG-008 ───────────────────────────

/// MIG-008：方言专有对象只在对应方言存在（PG citext / MySQL FULLTEXT / SQLite FTS5）。
#[tokio::test]
async fn mig_008_dialect_specific_objects() {
    let Some(c) = ctx().await else { return };
    let users = relation_columns(c.db(), c.dialect).await;
    let user_cols = users.get("users").cloned().unwrap_or_default();
    let relations: BTreeSet<String> = users.keys().cloned().collect();
    let indexes = index_names(c.db(), c.dialect).await;

    let has_email_ci = user_cols.iter().any(|c| c == "email_ci");
    let has_fts_table = relations.contains("audit_fts");
    let has_fulltext = indexes.contains("ft_audit_action");

    match c.dialect {
        Dialect::SqliteMem | Dialect::SqliteFile => {
            assert!(has_fts_table, "SQLite 应存在 FTS5 表 audit_fts");
            assert!(!has_email_ci, "citext 列 email_ci 仅 PG 存在");
            assert!(!has_fulltext, "FULLTEXT 索引仅 MySQL 存在");
        }
        Dialect::Postgres => {
            assert!(has_email_ci, "PG 应存在 citext 列 users.email_ci");
            assert!(!has_fts_table, "audit_fts 仅 SQLite 存在");
            assert!(!has_fulltext, "ft_audit_action 仅 MySQL 存在");
        }
        Dialect::Mysql => {
            assert!(has_fulltext, "MySQL 应存在 FULLTEXT 索引 ft_audit_action");
            assert!(!has_email_ci, "email_ci 仅 PG 存在");
            assert!(!has_fts_table, "audit_fts 仅 SQLite 存在");
        }
    }
    eprintln!(
        "[MIG-008] {}：email_ci={has_email_ci} audit_fts={has_fts_table} fulltext={has_fulltext}",
        c.dialect.name()
    );
}

// ─────────────────────────── MIG-009 ───────────────────────────

/// MIG-009：1M 行回填的**缩放版**（20k 行），测量并打印墙钟时间，上界 10s。
#[tokio::test]
async fn mig_009_scaled_backfill_is_bounded() {
    let Some(c) = ctx().await else { return };
    const ROWS: i64 = 20_000;
    const CHUNK: i64 = 200;

    let started = Instant::now();
    c.db()
        .execute(
            "CREATE TABLE mig009_backfill (\
             id BIGINT NOT NULL PRIMARY KEY, tenant_key VARCHAR(64), tenant_id BIGINT)",
        )
        .await
        .expect("create scratch table");

    let mut id = 1i64;
    while id <= ROWS {
        let n = CHUNK.min(ROWS - id + 1);
        let mut sql = String::from("INSERT INTO mig009_backfill (id, tenant_key) VALUES ");
        let mut params: Vec<Value> = Vec::with_capacity((n * 2) as usize);
        for k in 0..n {
            if k > 0 {
                sql.push(',');
            }
            sql.push_str("(?, ?)");
            params.push(Value::I64(id + k));
            params.push(Value::Text(format!("t{}", (id + k) % 100)));
        }
        c.db()
            .execute_with_params(&sql, params)
            .await
            .expect("insert chunk");
        id += n;
    }
    let insert_elapsed = started.elapsed();

    // 回填（等价于 V0021 backfill，仅缩放数据量）
    let backfill_cast = match c.dialect {
        Dialect::SqliteMem | Dialect::SqliteFile => "INTEGER",
        Dialect::Postgres => "BIGINT",
        Dialect::Mysql => "SIGNED",
    };
    let t = Instant::now();
    let updated = c
        .db()
        .execute(&format!(
            "UPDATE mig009_backfill SET tenant_id = CAST(substr(tenant_key, 2) AS {backfill_cast}) \
             WHERE tenant_id IS NULL"
        ))
        .await
        .expect("backfill");
    let backfill_elapsed = t.elapsed();

    assert_eq!(updated, ROWS as u64, "回填行数");
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM mig009_backfill WHERE tenant_id IS NULL",
            vec![]
        )
        .await,
        0,
        "不得有未回填行"
    );
    let sample: i64 = c
        .db()
        .scalar_with_params::<i64>(
            "SELECT tenant_id FROM mig009_backfill WHERE id = ?",
            vec![Value::I64(5)],
        )
        .await
        .expect("sample");
    assert_eq!(sample, 5, "id=5 → tenant_key 't5' → tenant_id 5");

    eprintln!(
        "[MIG-009] rows={ROWS} 插入={insert_elapsed:?} 回填={backfill_elapsed:?} 方言={}",
        c.dialect.name()
    );
    assert!(
        backfill_elapsed < Duration::from_secs(10),
        "20k 行回填耗时 {backfill_elapsed:?} 超出 10s 上界"
    );

    // 用例自清理（共享库上不残留探针表）
    c.db()
        .execute("DROP TABLE mig009_backfill")
        .await
        .expect("drop scratch table");
}

// ─────────────────────────── MIG-010 ───────────────────────────

/// MIG-010：快照 ↔ 迁移一致性（快照由 `<方言>/01-base` 的建表 DDL + `common/*.up.sql`
/// 的演进列共同推导，且其中的表/列必须真实存在于迁移后的库中）。
#[tokio::test]
async fn mig_010_snapshot_matches_common_migrations() {
    let Some(c) = ctx().await else { return };

    let snapshot_path = env!("ALUN_DB_SCHEMA_SNAPSHOT");
    let snapshot = std::fs::read_to_string(snapshot_path).expect("read snapshot");
    let snap = parse_snapshot(&snapshot);
    assert!(!snap.is_empty(), "快照应含表定义");

    // 1) 快照 = 按 build.rs 的同一条规则解析「<方言>/01-base + common/*.up.sql」
    let derived = derive_snapshot_columns(&snapshot_dialect());
    assert_eq!(snap, derived, "快照与 common 迁移推导结果必须一致");

    // 2) 快照列的真实存在性：D012 修复后**不应存在任何漂移**
    let actual = relation_columns(c.db(), c.dialect).await;
    let mut drift: BTreeSet<(String, String)> = BTreeSet::new();
    for (table, cols) in &snap {
        let actual_cols = actual
            .get(table)
            .unwrap_or_else(|| panic!("快照表 {table} 不在真实库中"));
        for col in cols {
            if !actual_cols.iter().any(|c| c == col) {
                drift.insert((table.clone(), col.clone()));
            }
        }
    }
    let expected_drift: BTreeSet<(String, String)> = BTreeSet::new();
    assert_eq!(
        drift,
        expected_drift,
        "快照与真实库（{}）的列集合必须一致（D012 修复后无漂移）",
        c.dialect.name()
    );
    eprintln!("[MIG-010] 快照与真实库无列漂移（DROP COLUMN 已被解析器跟踪，D012 已修复）");

    // 3) 快照表集合 ⊆ 方言 base DDL 建的表
    let base_tables: BTreeSet<String> = BASE_TABLES.iter().map(|s| s.to_string()).collect();
    for table in snap.keys() {
        assert!(
            base_tables.contains(table),
            "快照出现 base DDL 未建的表: {table}"
        );
    }
    eprintln!(
        "[MIG-010] 快照表 {} 张 / 列 {} 个，与 common 推导及真实库（{}）一致",
        snap.len(),
        snap.values().map(|v| v.len()).sum::<usize>(),
        c.dialect.name()
    );
}

/// 解析 `alun-db-build` 渲染出的快照 DDL → 表 → 列。
fn parse_snapshot(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut cur: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("CREATE TABLE ") {
            let table = rest.split([' ', '(']).next().unwrap_or("").to_string();
            if !table.is_empty() {
                out.entry(table.clone()).or_default();
                cur = Some(table);
            }
            continue;
        }
        if trimmed.starts_with(')') {
            cur = None;
            continue;
        }
        if let Some(table) = &cur {
            let col = trimmed
                .trim_end_matches(',')
                .split_whitespace()
                .next()
                .unwrap_or("");
            if !col.is_empty() {
                out.entry(table.clone()).or_default().push(col.to_string());
            }
        }
    }
    out
}

/// 快照的**构建方言**：与 `build.rs` 保持同一开关（默认 sqlite）。
fn snapshot_dialect() -> String {
    std::env::var("ALUN_LEDGER_SNAPSHOT_DIALECT").unwrap_or_else(|_| "sqlite".into())
}

/// 按 `alun-db-build` 的规则推导「基础 DDL + common 演进」的最终列集。
///
/// 与 `build.rs` 的顺序一致：先 `<方言>/01-base` 的 `CREATE TABLE`，再 `common`
/// 的 `ALTER TABLE ... ADD COLUMN`。表约束行（`PRIMARY KEY`/`UNIQUE`/`FOREIGN KEY`/
/// `CONSTRAINT`/`CHECK`）不是列，需剔除（`alun-db-build` 同样跳过）。
fn derive_snapshot_columns(dialect: &str) -> BTreeMap<String, Vec<String>> {
    const NOT_A_COLUMN: [&str; 8] = [
        "constraint",
        "primary",
        "unique",
        "foreign",
        "key",
        "check",
        "index",
        "references",
    ];
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut push = |table: &str, col: &str| {
        let clean = |t: &str| -> String {
            // 去掉引号与尾随分隔符/括号（表约束的第二段形如 `code))`）
            t.trim()
                .trim_matches(['"', '`'])
                .trim_end_matches([',', ')', ';'])
                .trim_matches(['"', '`'])
                .to_lowercase()
        };
        let table = clean(table);
        let col = clean(col);
        if table.is_empty()
            || col.is_empty()
            || NOT_A_COLUMN.contains(&col.as_str())
            || !col
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        {
            return;
        }
        let entry = out.entry(table).or_default();
        if !entry.contains(&col) {
            entry.push(col);
        }
    };

    // 基础 DDL：多行 `CREATE TABLE t ( <col> <type>, ... );`
    let base = migrations_root().join(dialect).join("01-base");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&base)
        .unwrap_or_else(|e| panic!("read {}: {e}", base.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.to_string_lossy().ends_with(".up.sql"))
        .collect();
    files.sort();
    for f in &files {
        let sql = std::fs::read_to_string(f).expect("read base migration");
        for stmt in statements(&sql) {
            let low = stmt.to_lowercase();
            let Some(idx) = low.find("create table") else {
                continue;
            };
            let rest = &stmt[idx..];
            let Some(open) = rest.find('(') else { continue };
            let table = rest["create table".len()..open].trim();
            let table = table.strip_prefix("IF NOT EXISTS ").unwrap_or(table).trim();
            let mut depth = 0usize;
            let mut body = String::new();
            for ch in rest[open..].chars() {
                match ch {
                    '(' => {
                        if depth > 0 {
                            body.push(ch);
                        }
                        depth += 1;
                    }
                    ')' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            break;
                        }
                        body.push(ch);
                    }
                    _ => {
                        if depth > 0 {
                            body.push(ch);
                        }
                    }
                }
            }
            for line in body.split(',') {
                if let Some(col) = line.split_whitespace().next() {
                    push(table, col);
                }
            }
        }
    }

    // common 演进：`ALTER TABLE t ADD [COLUMN] c <type>`
    for (table, col) in derive_common_columns(&migrations_root().join("common")) {
        for c in col {
            push(&table, &c);
        }
    }
    out
}

/// 按 `alun-db-build` 的规则从 `common/*.up.sql` 推导演进结果：
/// 只认 `ALTER TABLE ... ADD [COLUMN]` 与 `ALTER TABLE ... DROP [COLUMN] [IF EXISTS]`，
/// **按文件名（版本）顺序**依次作用，其余子句（约束/索引/RENAME 等）忽略。
fn derive_common_columns(dir: &Path) -> BTreeMap<String, Vec<String>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read common")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.to_string_lossy().ends_with(".up.sql"))
        .collect();
    files.sort();
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for f in files {
        let sql = std::fs::read_to_string(&f).expect("read migration");
        for stmt in statements(&sql) {
            let s = stmt.to_lowercase();
            let Some(idx) = s.find("alter table") else {
                continue;
            };
            let rest = stmt[idx + "alter table".len()..].trim();
            let mut tok = rest.split_whitespace();
            let Some(table) = tok.next() else { continue };
            let table = table.trim_matches(['"', '`']).to_lowercase();
            // 表名之后定位独立的 ADD / DROP 关键字
            let after_table = rest[table.len().min(rest.len())..].trim_start();
            let action = after_table
                .split_whitespace()
                .position(|t| t.eq_ignore_ascii_case("add") || t.eq_ignore_ascii_case("drop"));
            let Some(action_pos) = action else { continue };
            let is_drop = after_table
                .split_whitespace()
                .nth(action_pos)
                .map(|t| t.eq_ignore_ascii_case("drop"))
                .unwrap_or(false);
            // 列名 = 动作关键字之后（跳过可选的 COLUMN / IF EXISTS）
            let col = after_table
                .split_whitespace()
                .skip(action_pos + 1)
                .find(|t| {
                    let t = t.trim_matches(',');
                    !t.eq_ignore_ascii_case("column")
                        && !t.eq_ignore_ascii_case("if")
                        && !t.eq_ignore_ascii_case("exists")
                })
                .unwrap_or("");
            let col = col
                .trim_matches(['"', '`'])
                .trim_end_matches([',', ')'])
                .to_lowercase();
            if col.is_empty() {
                continue;
            }
            let entry = out.entry(table).or_default();
            if is_drop {
                entry.retain(|c| c != &col);
            } else if !entry.contains(&col) {
                entry.push(col);
            }
        }
    }
    out
}

// ─────────────────────────── MIG-011 ───────────────────────────

/// MIG-011：`App::start()` 的自动迁移。完整的启动链路由 E2E 用例覆盖，这里断言
/// **库级等价**：读取 `config/config.toml` 的 `[database.migration]`，用配置指向的
/// 目录 + 方言目录调用应用所用的 [`Migrator::from_dir`]，证明表存在。
#[tokio::test]
async fn mig_011_config_driven_migration_creates_tables() {
    let Some(c) = ctx().await else { return };

    let cfg_dir = manifest_dir().join("config");
    let mgr = alun_config::ConfigManager::load(Some(&cfg_dir.to_string_lossy()));
    let m = &mgr.get().database.migration;
    let (enabled, auto_migrate, path) = (m.enabled, m.auto_migrate, m.path.clone());
    assert!(
        !path.trim().is_empty(),
        "[database.migration] path 不得为空"
    );

    // app.rs:554 的启动条件
    let app_would_migrate = enabled && auto_migrate;
    eprintln!(
        "[MIG-011] 随仓库配置 enabled={enabled} auto_migrate={auto_migrate} path={path} \
         → App::start 是否自动迁移: {app_would_migrate}（配置层未开启，属配置而非缺陷）"
    );

    // 库级等价路径：配置目录（相对 crate 根解析）+ 方言三段目录
    let mut applied = 0usize;
    let cfg_path = manifest_dir().join(&path);
    if cfg_path.is_dir() {
        applied += Migrator::from_dir(&cfg_path)
            .run(c.db())
            .await
            .unwrap_or_else(|e| panic!("配置目录 {} 迁移失败: {e}", cfg_path.display()));
    } else {
        eprintln!("[MIG-011] 配置目录不存在: {}", cfg_path.display());
    }
    for dir in dir_paths(c.dialect) {
        applied += Migrator::from_dir(&dir)
            .run(c.db())
            .await
            .unwrap_or_else(|e| panic!("{} 迁移失败: {e}", dir.display()));
    }
    let _ = applied; // 幂等：ctx() 已应用时为 0

    for table in BASE_TABLES {
        assert!(
            table_exists(c.db(), c.dialect, table).await,
            "迁移后表 {table} 应存在（{}）",
            c.dialect.name()
        );
    }
    eprintln!("[MIG-011] 10 张 base 表全部存在（{}）", c.dialect.name());
}

// ─────────────────────────── MIG-012 ───────────────────────────

/// MIG-012：SQLite 改列类型（12 步重建法），逐条单语句执行，数据无丢失。
#[tokio::test]
async fn mig_012_sqlite_column_type_rebuild_preserves_data() {
    let Some(c) = ctx().await else { return };
    if !c.dialect.is_sqlite() {
        eprintln!(
            "[MIG-012] 12 步表重建仅适用于 SQLite；{} 跳过（方言差异需手写 DDL，登记为 W）",
            c.dialect.name()
        );
        return;
    }
    let db = c.db();

    // 12 步法的可单语句化子集（每步一条语句，满足 runner 约束）
    let steps = [
        // 1. 原表
        "CREATE TABLE mig012_old (\
         id INTEGER NOT NULL PRIMARY KEY, status INTEGER NOT NULL, label VARCHAR(32))",
        // 2. 原数据
        "INSERT INTO mig012_old (id, status, label) VALUES (1, 1, 'a')",
        "INSERT INTO mig012_old (id, status, label) VALUES (2, 2, 'b')",
        "INSERT INTO mig012_old (id, status, label) VALUES (3, 3, 'c')",
        // 3. 新表（status 由 INTEGER 改为 TEXT 枚举）
        "CREATE TABLE mig012_new (\
         id INTEGER NOT NULL PRIMARY KEY, status VARCHAR(16) NOT NULL, label VARCHAR(32))",
        // 4. 数据搬迁 + 类型转换
        "INSERT INTO mig012_new (id, status, label) SELECT id, \
         CASE status WHEN 1 THEN 'draft' WHEN 2 THEN 'issued' ELSE 'paid' END, label FROM mig012_old",
        // 5. 删除旧表
        "DROP TABLE mig012_old",
        // 6. 新表改回原名
        "ALTER TABLE mig012_new RENAME TO mig012_old",
        // 7. 重建索引
        "CREATE INDEX idx_mig012_status ON mig012_old (status)",
    ];
    for step in steps {
        db.execute(step)
            .await
            .unwrap_or_else(|e| panic!("步骤失败: {e}\nSQL: {step}"));
    }

    assert_eq!(
        column_type(db, "mig012_old", "status").await,
        "VARCHAR(16)",
        "列类型应已变更"
    );
    assert_eq!(
        count_by(db, "SELECT COUNT(*) FROM mig012_old", vec![]).await,
        3,
        "数据不得丢失"
    );
    let status: String = db
        .scalar_with_params::<String>(
            "SELECT status FROM mig012_old WHERE id = ?",
            vec![Value::I64(2)],
        )
        .await
        .expect("read converted value");
    assert_eq!(status, "issued");
    assert!(
        index_names(db, c.dialect)
            .await
            .contains("idx_mig012_status"),
        "重建后索引应存在"
    );
    let integrity: String = db
        .scalar("PRAGMA integrity_check")
        .await
        .expect("integrity");
    assert_eq!(integrity, "ok");

    db.execute("DROP INDEX idx_mig012_status")
        .await
        .expect("drop idx");
    db.execute("DROP TABLE mig012_old")
        .await
        .expect("drop scratch");
}

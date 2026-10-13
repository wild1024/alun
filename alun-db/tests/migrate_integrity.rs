//! 迁移完整性与回滚顺序测试。
//!
//! 覆盖：
//! - D011：已应用迁移的内容指纹校验（篡改报错）、旧表补列兼容、并发迁移互斥；
//! - D022：回滚按**应用顺序**（`applied_seq`）而非版本号大小，且跨目录越序回滚
//!   被显式拒绝。

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use alun_db::prelude::*;
use alun_db_core::error::DbError;
use alun_db_core::stmt::Value;

// ==================== 临时目录 ====================

/// 并行测试各用独立临时目录（位于包内 `target/`，与 ledger harness 同约定）。
fn temp_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = PathBuf::from("target")
        .join("migrate_integrity")
        .join(format!("{tag}_{}_{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// 写入一个迁移文件。
fn write(dir: &Path, name: &str, sql: &str) {
    fs::write(dir.join(name), sql).expect("write migration file");
}

/// 写入一对 up/down 迁移文件。
fn write_pair(dir: &Path, version: i64, name: &str, table: &str) {
    write(
        dir,
        &format!("V{version}__{name}.up.sql"),
        &format!("CREATE TABLE {table} (id INTEGER NOT NULL PRIMARY KEY)"),
    );
    write(
        dir,
        &format!("V{version}__{name}.down.sql"),
        &format!("DROP TABLE {table}"),
    );
}

/// 表是否存在。
async fn table_exists(db: &Db, table: &str) -> bool {
    db.find_first_with_params(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?",
        vec![Value::Text(table.to_string())],
    )
    .await
    .expect("sqlite_master")
    .is_some()
}

/// `schema_migrations` 当前记录数。
async fn records(db: &Db) -> i64 {
    db.scalar("SELECT COUNT(*) FROM schema_migrations")
        .await
        .expect("count")
}

// ==================== D011：内容指纹 ====================

#[tokio::test]
async fn tampered_applied_migration_is_rejected() {
    let db = common::setup_db().await;
    let dir = temp_dir("checksum");
    write_pair(&dir, 1, "create_probe", "checksum_probe");
    assert_eq!(Migrator::from_dir(&dir).run(&db).await.expect("apply"), 1);

    // 改写**已应用**迁移的内容（复现 DEFECTS D011 / ledger mig_003）
    write(
        &dir,
        "V1__create_probe.up.sql",
        "CREATE TABLE checksum_probe_tampered (id INTEGER NOT NULL PRIMARY KEY)",
    );

    let err = Migrator::from_dir(&dir)
        .run(&db)
        .await
        .expect_err("篡改必须报错");
    match &err {
        DbError::MigrationChecksum {
            version,
            applied,
            current,
            ..
        } => {
            assert_eq!(version, "1");
            assert_ne!(applied, current, "两枚指纹必须不同");
        }
        other => panic!("期望 MigrationChecksum，实际 {other:?}"),
    }
    let text = err.to_string();
    assert!(
        text.contains("checksum") && text.contains("tampered"),
        "文案需说明「已应用迁移被篡改」：{text}"
    );

    // 篡改后的 DDL 不得执行，记录也不得被改写
    assert!(!table_exists(&db, "checksum_probe_tampered").await);
    assert!(table_exists(&db, "checksum_probe").await);
    assert_eq!(records(&db).await, 1);

    // 回滚同样受指纹保护（不可信状态不许执行 down）
    let err = Migrator::from_dir(&dir)
        .rollback(&db)
        .await
        .expect_err("篡改时不得回滚");
    assert!(
        matches!(err, DbError::MigrationChecksum { .. }),
        "got {err:?}"
    );
    assert_eq!(records(&db).await, 1);

    // 恢复原内容 → 不再报错（指纹只反映语义内容，无假阳性）
    write_pair(&dir, 1, "create_probe", "checksum_probe");
    assert_eq!(Migrator::from_dir(&dir).run(&db).await.expect("no-op"), 0);
    assert_eq!(
        Migrator::from_dir(&dir)
            .rollback(&db)
            .await
            .expect("down")
            .map(|v| v.to_string()),
        Some("1".to_string())
    );
}

#[tokio::test]
async fn history_row_without_checksum_is_backfilled() {
    let db = common::setup_db().await;
    let dir = temp_dir("checksum_backfill");
    write_pair(&dir, 1, "create_probe", "legacy_probe");
    assert_eq!(Migrator::from_dir(&dir).run(&db).await.expect("apply"), 1);

    // 模拟旧框架导入行：历史行没有内容指纹（compat 导入 / 手工写入）
    db.execute("UPDATE schema_migrations SET checksum = NULL")
        .await
        .expect("clear checksum");

    // 首次运行补录指纹（不重复执行已应用迁移）
    assert_eq!(
        Migrator::from_dir(&dir).run(&db).await.expect("backfill"),
        0
    );
    let checksum: Option<String> = db
        .scalar("SELECT checksum FROM schema_migrations WHERE version = '1'")
        .await
        .expect("checksum");
    let checksum = checksum.expect("首次运行必须补录指纹");
    assert_eq!(checksum.len(), 64, "补录 SHA-256 指纹");

    // 补录之后即可发现篡改
    write(
        &dir,
        "V1__create_probe.up.sql",
        "CREATE TABLE legacy_probe_tampered (id INTEGER NOT NULL PRIMARY KEY)",
    );
    let err = Migrator::from_dir(&dir)
        .run(&db)
        .await
        .expect_err("补录后应能发现篡改");
    assert!(
        matches!(err, DbError::MigrationChecksum { .. }),
        "got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_migrations_apply_each_version_once() {
    // 文件库：每次迁移走不同连接，进程内互斥锁负责串行化
    let dir_db = temp_dir("concurrent_db");
    let db = Db::connect(&format!("sqlite://{}", dir_db.join("lock.db").display()))
        .await
        .expect("file db");

    let dir = temp_dir("concurrent_migrations");
    for v in 1..=6 {
        write_pair(&dir, v, &format!("create_t{v}"), &format!("conc_t{v}"));
    }

    let mut handles = Vec::new();
    for _ in 0..4 {
        let db = db.clone();
        let dir = dir.clone();
        handles.push(tokio::spawn(async move {
            Migrator::from_dir(&dir)
                .run(&db)
                .await
                .map_err(|e| e.to_string())
        }));
    }

    let mut applied_total = 0;
    for handle in handles {
        let applied = handle
            .await
            .expect("task must not panic")
            .expect("并发迁移不得失败（有迁移互斥）");
        applied_total += applied;
    }
    assert_eq!(applied_total, 6, "每个版本恰好应用一次");
    assert_eq!(records(&db).await, 6, "不得出现重复记录");

    // 收敛后再次迁移是 no-op
    assert_eq!(Migrator::from_dir(&dir).run(&db).await.expect("re-run"), 0);
    for v in 1..=6 {
        assert!(table_exists(&db, &format!("conc_t{v}")).await);
    }
}

// ==================== D022：回滚顺序 ====================

#[tokio::test]
async fn rollback_follows_application_order_not_version_order() {
    let db = common::setup_db().await;
    // 版本号大、应用更早
    let late = temp_dir("order_late");
    // 版本号小、应用更晚
    let early = temp_dir("order_early");
    write_pair(&late, 9, "create_late", "order_late");
    write_pair(&early, 2, "create_early", "order_early");

    assert_eq!(
        Migrator::from_dir(&late).run(&db).await.expect("apply 9"),
        1
    );
    assert_eq!(
        Migrator::from_dir(&early).run(&db).await.expect("apply 2"),
        1
    );

    // 最近**应用**的是版本 2（版本号更小）→ 先回滚它；按版本号取最大会误回滚 9
    assert_eq!(
        Migrator::from_dir(&early)
            .rollback(&db)
            .await
            .expect("down 2")
            .map(|v| v.to_string()),
        Some("2".to_string()),
        "回滚必须按应用顺序"
    );
    assert!(!table_exists(&db, "order_early").await);
    assert!(table_exists(&db, "order_late").await);

    assert_eq!(
        Migrator::from_dir(&late)
            .rollback(&db)
            .await
            .expect("down 9")
            .map(|v| v.to_string()),
        Some("9".to_string())
    );
    assert!(
        Migrator::from_dir(&late)
            .rollback(&db)
            .await
            .expect("empty")
            .is_none()
    );
}

#[tokio::test]
async fn rollback_refuses_latest_applied_from_another_directory() {
    let db = common::setup_db().await;
    let first = temp_dir("scope_first");
    let second = temp_dir("scope_second");
    write_pair(&first, 1, "create_first", "scope_first_probe");
    write_pair(&second, 5, "create_second", "scope_second_probe");

    assert_eq!(
        Migrator::from_dir(&first).run(&db).await.expect("apply 1"),
        1
    );
    assert_eq!(
        Migrator::from_dir(&second).run(&db).await.expect("apply 5"),
        1
    );

    // 全局最近应用的是版本 5，不属于 first 目录 → 显式拒绝，不拆后续迁移依赖的对象
    let err = Migrator::from_dir(&first)
        .rollback(&db)
        .await
        .expect_err("越序回滚必须报错");
    assert!(matches!(err, DbError::Migration(_)), "got {err:?}");
    let text = err.to_string();
    assert!(text.contains("version 5"), "文案需指明待回滚版本：{text}");

    assert!(
        table_exists(&db, "scope_first_probe").await,
        "不得拆掉已应用对象"
    );
    assert!(table_exists(&db, "scope_second_probe").await);
    assert_eq!(records(&db).await, 2, "失败的回滚不得删除记录");

    // 从它自己的目录回滚即可
    assert_eq!(
        Migrator::from_dir(&second)
            .rollback(&db)
            .await
            .expect("down 5")
            .map(|v| v.to_string()),
        Some("5".to_string())
    );
    assert_eq!(
        Migrator::from_dir(&first)
            .rollback(&db)
            .await
            .expect("down 1")
            .map(|v| v.to_string()),
        Some("1".to_string())
    );
}

#[tokio::test]
async fn gap_backfill_rolls_back_in_application_order() {
    let db = common::setup_db().await;

    // 先应用 {1, 5}（制造 gap）。
    // 记录表以 `(kind, name)` 为主键，故静态清单与目录文件必须**同名同内容**：
    // 同名版本被视为同一条迁移，内容不一致会被指纹校验拦下。
    let n = Migrator::from_migrations([
        Migration::new(
            1,
            "gap1",
            "CREATE TABLE gap_t1 (id INTEGER NOT NULL PRIMARY KEY)",
            "DROP TABLE gap_t1",
        ),
        Migration::new(
            5,
            "gap5",
            "CREATE TABLE gap_t5 (id INTEGER NOT NULL PRIMARY KEY)",
            "DROP TABLE gap_t5",
        ),
    ])
    .run(&db)
    .await
    .expect("static migrate");
    assert_eq!(n, 2);

    // 目录含 1..5：补迁 2、3、4（应用顺序变为 1,5,2,3,4）
    let dir = temp_dir("gap_order");
    for v in 1..=5 {
        write_pair(&dir, v, &format!("gap{v}"), &format!("gap_t{v}"));
    }
    assert_eq!(
        Migrator::from_dir(&dir).run(&db).await.expect("backfill"),
        3
    );

    // 逆序回滚：4 → 3 → 2 → 5 → 1（按版本号取最大会先回 5）
    let mut rolled = Vec::new();
    while let Some(version) = Migrator::from_dir(&dir).rollback(&db).await.expect("down") {
        rolled.push(version.to_string());
    }
    assert_eq!(
        rolled,
        vec!["4", "3", "2", "5", "1"],
        "回滚顺序 = 应用顺序的逆序"
    );
    assert_eq!(records(&db).await, 0);
}

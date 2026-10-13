//! `migrate_from_dir` 的集成测试：目录扫描、up/down 双向、幂等、gap 补迁、
//! 版本重复、非法文件名、缺配 down 报错与从目录回滚；以及单文件入口
//! `run_file` / `run_file_as` 与清单视图 `status` / `plan`。

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use alun_db::prelude::*;

/// 目录序号，确保并行测试各用独立临时目录。
static DIR_SEQ: AtomicU32 = AtomicU32::new(0);

/// 创建唯一临时目录用于存放迁移文件。
fn temp_migrations_dir(test: &str) -> PathBuf {
    let seq = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "alundb_migrate_{}_{}_{}",
        test,
        std::process::id(),
        seq
    ));
    fs::create_dir_all(&dir).expect("create migration temp dir");
    dir
}

/// 写入单个迁移文件。
fn write_migration(file: &Path, content: &str) {
    fs::write(file, content).expect("write migration file");
}

#[tokio::test]
async fn dir_migrations_apply_and_are_idempotent() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("idempotent");

    write_migration(
        &dir.join("V1__create_t1.sql"),
        "CREATE TABLE t1 (id INTEGER PRIMARY KEY)",
    );
    write_migration(
        &dir.join("V2__create_t2.sql"),
        "CREATE TABLE t2 (id INTEGER PRIMARY KEY)",
    );

    let n = Migrator::from_dir(&dir).run(&db).await.expect("first run");
    assert_eq!(n, 2, "applies both pending migrations");

    db.execute("INSERT INTO t1 (id) VALUES (1)")
        .await
        .expect("t1 exists");

    // 幂等：再次运行不重复应用
    let again = Migrator::from_dir(&dir).run(&db).await.expect("second run");
    assert_eq!(again, 0, "no-op when already applied");
}

#[tokio::test]
async fn non_sql_files_are_ignored() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("non_sql");

    write_migration(
        &dir.join("V1__create_t1.sql"),
        "CREATE TABLE t1 (id INTEGER PRIMARY KEY)",
    );
    write_migration(&dir.join("README.md"), "docs only, should be skipped");

    let n = Migrator::from_dir(&dir).run(&db).await.expect("run");
    assert_eq!(n, 1, "README.md is ignored");
    db.execute("INSERT INTO t1 (id) VALUES (1)")
        .await
        .expect("t1 exists");
}

#[tokio::test]
async fn paired_up_down_and_rollback_from_dir() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("pair");

    write_migration(
        &dir.join("V1__create_t1.up.sql"),
        "CREATE TABLE t1 (id INTEGER PRIMARY KEY)",
    );
    write_migration(&dir.join("V1__create_t1.down.sql"), "DROP TABLE t1");

    let n = Migrator::from_dir(&dir).run(&db).await.expect("migrate up");
    assert_eq!(n, 1);
    db.execute("INSERT INTO t1 (id) VALUES (1)")
        .await
        .expect("t1 exists");

    // 从目录清单回滚：执行 down 并删除版本记录
    let rolled = Migrator::from_dir(&dir)
        .rollback(&db)
        .await
        .expect("migrate down");
    assert_eq!(rolled.map(|v| v.to_string()), Some("1".to_string()));

    // 表已删除，可再次迁移
    let n = Migrator::from_dir(&dir)
        .run(&db)
        .await
        .expect("migrate up again");
    assert_eq!(n, 1, "re-applies after rollback");

    // 再回滚后无迁移可回滚
    Migrator::from_dir(&dir).rollback(&db).await.expect("down");
    assert!(
        Migrator::from_dir(&dir)
            .rollback(&db)
            .await
            .expect("down on empty")
            .is_none()
    );
}

#[tokio::test]
async fn gap_versions_are_backfilled() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("gap");

    // 先用静态清单应用版本 1 与 5（制造 gap：{1,5}）。
    // 注意：内容必须与下面目录里的同名版本文件**逐字一致**——D011 修复后
    // `schema_migrations.checksum` 会校验已应用版本的内容指纹。
    let n = Migrator::from_migrations([
        Migration::new(
            1,
            "create_t1",
            "CREATE TABLE t1 (id INTEGER PRIMARY KEY)",
            "DROP TABLE t1",
        ),
        Migration::new(
            5,
            "create_t5",
            "CREATE TABLE t5 (id INTEGER PRIMARY KEY)",
            "DROP TABLE t5",
        ),
    ])
    .run(&db)
    .await
    .expect("static migrate");
    assert_eq!(n, 2);

    // 目录含 1..5，其中 1、5 已应用，2、3、4 应被补迁（集合语义，非 MAX(version)）
    for (v, name) in [(1, "t1"), (2, "t2"), (3, "t3"), (4, "t4"), (5, "t5")] {
        write_migration(
            &dir.join(format!("V{v}__create_{name}.sql")),
            &format!("CREATE TABLE {name} (id INTEGER PRIMARY KEY)"),
        );
    }

    let applied = Migrator::from_dir(&dir)
        .run(&db)
        .await
        .expect("gap migrate");
    assert_eq!(applied, 3, "backfills versions 2,3,4");

    db.execute("INSERT INTO t2 (id) VALUES (1)")
        .await
        .expect("t2 exists");
    db.execute("INSERT INTO t3 (id) VALUES (1)")
        .await
        .expect("t3 exists");
    db.execute("INSERT INTO t4 (id) VALUES (1)")
        .await
        .expect("t4 exists");
}

#[tokio::test]
async fn duplicate_version_is_rejected() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("dup");

    write_migration(
        &dir.join("V1__create_t1.sql"),
        "CREATE TABLE t1 (id INTEGER PRIMARY KEY)",
    );
    write_migration(
        &dir.join("V1__create_t1_dup.sql"),
        "CREATE TABLE t1 (id INTEGER PRIMARY KEY)",
    );

    let err = Migrator::from_dir(&dir)
        .run(&db)
        .await
        .expect_err("duplicate version must fail");
    assert!(matches!(err, DbError::Migration(_)), "got {err:?}");
}

#[tokio::test]
async fn unparseable_sql_file_is_rejected() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("bad_name");

    // 缺少默认前缀：文件名不以 `V{n}__` 开头 → 报错，而非静默跳过
    write_migration(&dir.join("notes.sql"), "SELECT 1");

    let err = Migrator::from_dir(&dir)
        .run(&db)
        .await
        .expect_err("unparseable must fail");
    assert!(matches!(err, DbError::Migration(_)), "got {err:?}");
}

#[tokio::test]
async fn down_without_up_is_rejected() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("down_only");

    write_migration(&dir.join("V2__create_t2.down.sql"), "DROP TABLE t2");

    let err = Migrator::from_dir(&dir)
        .run(&db)
        .await
        .expect_err("down without up must fail");
    assert!(matches!(err, DbError::Migration(_)), "got {err:?}");
}

// ==================== 单文件入口与清单视图 ====================

#[tokio::test]
async fn run_file_applies_and_is_idempotent() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("run_file");
    let file = dir.join("V7__create_probe.sql");
    write_migration(
        &file,
        "CREATE TABLE run_file_probe (id INTEGER PRIMARY KEY)",
    );

    let migrator = Migrator::new();
    assert_eq!(migrator.run_file(&db, &file).await.expect("run file"), 1);
    db.execute("INSERT INTO run_file_probe (id) VALUES (1)")
        .await
        .expect("表已建");

    // 同版本已成功应用 → 跳过（幂等），不重复执行脚本
    assert_eq!(migrator.run_file(&db, &file).await.expect("re-run"), 0);
}

#[tokio::test]
async fn run_file_rejects_down_suffix() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("run_file_down");
    let file = dir.join("V7__create_probe.down.sql");
    write_migration(&file, "DROP TABLE run_file_probe");

    // `.down.sql` 不是可执行的迁移，必须报错而非当成 up 执行
    let err = Migrator::new()
        .run_file(&db, &file)
        .await
        .expect_err("down 文件不得作为 up 执行");
    assert!(matches!(err, DbError::Migration(_)), "got {err:?}");
}

#[tokio::test]
async fn run_file_as_registers_explicit_identity() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("run_file_as");
    // 文件名不符合任何命名风格，须由调用方显式给出身份
    let file = dir.join("hotfix.sql");
    write_migration(&file, "CREATE TABLE hotfix_probe (id INTEGER PRIMARY KEY)");

    let migrator = Migrator::new();
    let explicit = Version::new("20250101_120000");
    assert_eq!(
        migrator
            .run_file_as(&db, &file, explicit.clone(), "hotfix")
            .await
            .expect("run file as"),
        1
    );
    // 记录使用调用方给定的身份，而非文件名（`hotfix.sql` 本不符合任何风格）
    let name: String = db
        .scalar("SELECT name FROM schema_migrations WHERE version = '20250101_120000'")
        .await
        .expect("recorded name");
    assert_eq!(name, "hotfix");
    // 显式身份注册后重复运行是幂等的
    assert_eq!(
        migrator
            .run_file_as(&db, &file, explicit, "hotfix")
            .await
            .expect("re-run"),
        0
    );

    // 空版本 / 空名称属于调用方输入错误，须显式拒绝
    assert!(matches!(
        migrator
            .run_file_as(&db, &file, Version::new(""), "hotfix")
            .await,
        Err(DbError::InvalidValue(_))
    ));
    assert!(matches!(
        migrator
            .run_file_as(&db, &file, Version::new("1"), "  ")
            .await,
        Err(DbError::InvalidValue(_))
    ));
}

#[tokio::test]
async fn status_and_plan_reflect_pending_then_applied() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("status");
    write_migration(
        &dir.join("V1__create_t1.sql"),
        "CREATE TABLE t1 (id INTEGER PRIMARY KEY)",
    );
    write_migration(
        &dir.join("V2__create_t2.sql"),
        "CREATE TABLE t2 (id INTEGER PRIMARY KEY)",
    );

    let migrator = Migrator::from_dir(&dir);

    // 裸库（记录表都不存在）时 status 不得报错，而是「全部待应用」
    let statuses = migrator.status(&db).await.expect("status on bare db");
    assert_eq!(statuses.len(), 2);
    assert!(
        statuses
            .iter()
            .all(|s| s.state == MigrationState::Pending && !s.is_applied),
        "{statuses:?}"
    );
    assert_eq!(migrator.plan(&db).await.expect("plan").len(), 2);

    assert_eq!(migrator.run(&db).await.expect("apply"), 2);

    let statuses = migrator.status(&db).await.expect("status after apply");
    assert_eq!(statuses.len(), 2);
    assert!(statuses.iter().all(|s| s.state == MigrationState::Applied));
    assert!(statuses.iter().all(|s| s.is_applied));
    assert!(
        statuses
            .iter()
            .all(|s| s.applied_checksum == Some(s.current_checksum.clone())),
        "已应用迁移的指纹应与当前文件一致：{statuses:?}"
    );
    assert!(
        migrator
            .plan(&db)
            .await
            .expect("plan after apply")
            .is_empty()
    );
}

#[tokio::test]
async fn status_reports_missing_when_file_is_removed() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("status_missing");
    write_migration(
        &dir.join("V1__create_t1.sql"),
        "CREATE TABLE t1 (id INTEGER PRIMARY KEY)",
    );
    write_migration(
        &dir.join("V2__create_t2.sql"),
        "CREATE TABLE t2 (id INTEGER PRIMARY KEY)",
    );
    assert_eq!(Migrator::from_dir(&dir).run(&db).await.expect("apply"), 2);

    // 删掉 v2 的文件 → 库内记录在清单外，报 Missing 而非静默忽略
    fs::remove_file(dir.join("V2__create_t2.sql")).expect("remove file");

    let statuses = Migrator::from_dir(&dir).status(&db).await.expect("status");
    let missing: Vec<_> = statuses
        .iter()
        .filter(|s| s.state == MigrationState::Missing)
        .collect();
    assert_eq!(missing.len(), 1, "{statuses:?}");
    assert_eq!(missing[0].name, "create_t2");
    assert_eq!(
        missing[0]
            .version
            .as_ref()
            .map(|v| v.to_string())
            .as_deref(),
        Some("2")
    );
}

#[tokio::test]
async fn status_reports_divergent_when_applied_file_is_edited() {
    let db = common::setup_db().await;
    let dir = temp_migrations_dir("status_divergent");
    write_migration(
        &dir.join("V1__create_t1.sql"),
        "CREATE TABLE t1 (id INTEGER PRIMARY KEY)",
    );
    assert_eq!(Migrator::from_dir(&dir).run(&db).await.expect("apply"), 1);

    // 改写已应用迁移的内容 → 状态为 Divergent（执行入口会在 apply 前报错）
    write_migration(
        &dir.join("V1__create_t1.sql"),
        "CREATE TABLE t1_tampered (id INTEGER PRIMARY KEY)",
    );
    let statuses = Migrator::from_dir(&dir).status(&db).await.expect("status");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].state, MigrationState::Divergent, "{statuses:?}");

    // plan 只列 Pending，不把 Divergent 当成待应用
    assert!(
        Migrator::from_dir(&dir)
            .plan(&db)
            .await
            .expect("plan")
            .is_empty()
    );
}

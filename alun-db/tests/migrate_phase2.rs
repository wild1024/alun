//! 第二期能力测试：可重复 / 种子迁移、基线、repair、dry_run、旧表导入开关。
//!
//! 对照设计文档 §6（错误与一致性）、§7（baseline / repair / dry_run）与 §10 第二期。

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use alun_db::prelude::*;
use alun_db_core::error::DbError;
use alun_db_core::stmt::Value;

// ==================== 临时目录与断言辅助 ====================

/// 并行测试各用独立临时目录（位于包内 `target/`，与其余迁移测试同约定）。
fn temp_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = PathBuf::from("target")
        .join("migrate_phase2")
        .join(format!("{tag}_{}_{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// 写入一个迁移文件（自动建父目录）。
fn write(dir: &Path, name: &str, sql: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dir");
    }
    fs::write(&path, sql).expect("write migration file");
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

/// 记录表当前行数。
async fn record_count(db: &Db) -> i64 {
    db.scalar("SELECT COUNT(*) FROM schema_migrations")
        .await
        .expect("count records")
}

/// 指定 `(kind, name)` 的记录数。
async fn record_count_of(db: &Db, kind: &str, name: &str) -> i64 {
    let count: i64 = db
        .scalar_with_params(
            "SELECT COUNT(*) FROM schema_migrations WHERE kind = ? AND name = ?",
            vec![Value::Text(kind.to_string()), Value::Text(name.to_string())],
        )
        .await
        .expect("count records");
    count
}

// ==================== 可重复 / 种子迁移 ====================

/// `R__` 可重复迁移：无记录时执行，内容不变时跳过，内容变化时重跑。
#[tokio::test]
async fn repeatable_migration_reruns_when_content_changes() {
    let db = common::setup_db().await;
    let dir = temp_dir("repeatable");
    write(
        &dir,
        "R__create_view.sql",
        "CREATE VIEW rp_view AS SELECT 1 AS n",
    );

    let migrator = Migrator::from_dir(&dir);
    assert_eq!(migrator.run(&db).await.expect("first run"), 1, "首次执行");
    assert_eq!(record_count_of(&db, "repeatable", "create_view").await, 1);

    // 内容相同 → 幂等跳过
    assert_eq!(migrator.run(&db).await.expect("no-op"), 0);

    // 内容变化 → 重跑（可重复迁移的预期行为，不是篡改）
    write(
        &dir,
        "R__create_view.sql",
        "DROP VIEW rp_view; CREATE VIEW rp_view AS SELECT 1 AS n, 2 AS m",
    );
    assert_eq!(
        migrator.run(&db).await.expect("rerun"),
        1,
        "内容变化必须重跑而非报 MigrationChecksum"
    );
    let cols: i64 = db
        .scalar("SELECT COUNT(*) FROM pragma_table_info('rp_view')")
        .await
        .expect("pragma_table_info");
    assert_eq!(cols, 2, "重跑后的视图应含新列");

    // 重跑后内容又一致 → 再次幂等
    assert_eq!(migrator.run(&db).await.expect("no-op again"), 0);
}

/// 种子目录：其中每个 `.sql` 按 Seed 装载，文件名为种子名，内容变化即重灌。
#[tokio::test]
async fn seed_source_directory_is_loaded_and_rerun_on_change() {
    let db = common::setup_db().await;
    let dir = temp_dir("seed_dir");
    let seed_dir = temp_dir("seed_data");
    write(
        &dir,
        "V1__create_dict.sql",
        "CREATE TABLE dict (code TEXT NOT NULL PRIMARY KEY, label TEXT NOT NULL)",
    );
    write(
        &seed_dir,
        "dict_data.sql",
        "INSERT INTO dict (code, label) VALUES ('a', 'A')",
    );

    let migrator = Migrator::from_dir(&dir).source_seed(&seed_dir);
    // 版本化迁移先执行、种子后执行（同一批内也要满足依赖顺序）
    assert_eq!(migrator.run(&db).await.expect("apply"), 2);
    assert_eq!(record_count_of(&db, "seed", "dict_data").await, 1);

    let label: String = db
        .scalar("SELECT label FROM dict WHERE code = 'a'")
        .await
        .expect("seeded row");
    assert_eq!(label, "A");

    // 种子内容变化 → 幂等重灌（用覆盖写验证「重跑」而非「重复插入报错」）
    write(
        &seed_dir,
        "dict_data.sql",
        "INSERT INTO dict (code, label) VALUES ('a', 'A2') ON CONFLICT(code) DO UPDATE SET label = 'A2'",
    );
    assert_eq!(migrator.run(&db).await.expect("re-seed"), 1);
    let label: String = db
        .scalar("SELECT label FROM dict WHERE code = 'a'")
        .await
        .expect("re-seeded row");
    assert_eq!(label, "A2", "种子内容变化后应重灌");
}

/// `status` 对内容变化的可重复迁移报 `Pending`（而非版本化迁移的 `Divergent`）。
#[tokio::test]
async fn status_marks_changed_repeatable_as_pending_not_divergent() {
    let db = common::setup_db().await;
    let dir = temp_dir("status_rerunnable");
    write(
        &dir,
        "V1__create_t1.sql",
        "CREATE TABLE p2_t1 (id INTEGER NOT NULL PRIMARY KEY, n INTEGER NOT NULL DEFAULT 0)",
    );
    write(&dir, "S__dict.sql", "INSERT INTO p2_t1 (id) VALUES (1)");

    let migrator = Migrator::from_dir(&dir);
    assert_eq!(migrator.run(&db).await.expect("apply"), 2);

    // 改写种子的内容
    write(&dir, "S__dict.sql", "INSERT INTO p2_t1 (id) VALUES (2)");
    let statuses = migrator.status(&db).await.expect("status");
    let seed = statuses
        .iter()
        .find(|s| s.kind == MigrationKind::Seed)
        .expect("seed status");
    assert_eq!(seed.state, MigrationState::Pending, "{seed:?}");

    // 而版本化迁移被改写仍是 Divergent
    write(
        &dir,
        "V1__create_t1.sql",
        "CREATE TABLE p2_t1_tampered (id INTEGER NOT NULL PRIMARY KEY)",
    );
    let statuses = migrator.status(&db).await.expect("status");
    let versioned = statuses
        .iter()
        .find(|s| s.kind == MigrationKind::Versioned)
        .expect("versioned status");
    assert_eq!(versioned.state, MigrationState::Divergent, "{versioned:?}");
    assert_eq!(
        migrator.plan(&db).await.expect("plan").len(),
        1,
        "只有种子待重跑"
    );
}

// ==================== 基线 ====================

/// `baseline(n)`：`<= n` 的迁移被直接标记为已应用，其 SQL 不执行。
#[tokio::test]
async fn baseline_marks_versions_without_executing_them() {
    let db = common::setup_db().await;
    let dir = temp_dir("baseline");
    for v in 1..=4 {
        write(
            &dir,
            &format!("V{v}__create_t{v}.sql"),
            &format!("CREATE TABLE bl_t{v} (id INTEGER NOT NULL PRIMARY KEY)"),
        );
    }

    let migrator = Migrator::from_dir(&dir);
    // 存量库：结构已有 v1..=v2，接入时跳过它们
    assert_eq!(
        migrator
            .baseline(&db, Version::new("2"))
            .await
            .expect("baseline"),
        2
    );

    // 基线标记的迁移 SQL **未执行**：表都不存在
    assert!(!table_exists(&db, "bl_t1").await, "基线不得执行 SQL");
    assert!(!table_exists(&db, "bl_t2").await);
    assert_eq!(record_count(&db).await, 2, "仅 v1、v2 落地标记");

    // 随后的 run 只应用基线之后的版本
    assert_eq!(migrator.run(&db).await.expect("apply"), 2, "只应用 v3、v4");
    assert!(table_exists(&db, "bl_t3").await);
    assert!(table_exists(&db, "bl_t4").await);

    // 重复基线无副作用（不覆盖既有记录、不新增行）
    assert_eq!(
        migrator
            .baseline(&db, Version::new("2"))
            .await
            .expect("baseline again"),
        0
    );
    assert_eq!(record_count(&db).await, 4);

    // 空版本是调用方输入错误
    assert!(matches!(
        migrator.baseline(&db, Version::new("")).await,
        Err(DbError::InvalidValue(_))
    ));
}

/// `baseline_on_migrate` + `baseline_version`：启动即跳过历史版本。
#[tokio::test]
async fn configured_baseline_runs_before_versioned_migrations() {
    let db = common::setup_db().await;
    let dir = temp_dir("baseline_config");
    for v in 1..=3 {
        write(
            &dir,
            &format!("V{v}__create_c{v}.sql"),
            &format!("CREATE TABLE bc_t{v} (id INTEGER NOT NULL PRIMARY KEY)"),
        );
    }

    let migrator = Migrator::from_dir(&dir).options(MigratorOptions {
        baseline_on_migrate: true,
        baseline_version: Some(Version::new("2")),
        ..MigratorOptions::default()
    });
    assert_eq!(migrator.run(&db).await.expect("apply"), 1, "仅应用 v3");
    assert!(!table_exists(&db, "bc_t1").await);
    assert!(!table_exists(&db, "bc_t2").await);
    assert!(table_exists(&db, "bc_t3").await);

    // 只开开关不给版本 → 显式报错，而不是静默地什么都不标记
    let bad = Migrator::from_dir(&dir).options(MigratorOptions {
        baseline_on_migrate: true,
        ..MigratorOptions::default()
    });
    assert!(matches!(bad.run(&db).await, Err(DbError::InvalidValue(_))));
}

/// `B{n}__` 文件：声明式基线标记，不执行其 SQL；含语句时显式报错。
#[tokio::test]
async fn baseline_marker_file_declares_skipped_versions() {
    let db = common::setup_db().await;
    let dir = temp_dir("baseline_marker");
    for v in 1..=3 {
        write(
            &dir,
            &format!("V{v}__create_m{v}.sql"),
            &format!("CREATE TABLE bm_t{v} (id INTEGER NOT NULL PRIMARY KEY)"),
        );
    }
    // 基线标记文件：只写注释，不写 DDL
    write(&dir, "B2__baseline.sql", "-- 存量库结构已到 v2\n");

    let migrator = Migrator::from_dir(&dir);
    // B 文件本身不产生记录，但 baseline 语义由 `baseline_on_migrate` 或 API 触发；
    // 这里显式用 API 配合目录，验证「标记只在清单内落地」
    assert_eq!(
        migrator
            .baseline(&db, Version::new("2"))
            .await
            .expect("baseline"),
        2
    );
    assert_eq!(migrator.run(&db).await.expect("apply"), 1, "仅应用 v3");
    assert!(!table_exists(&db, "bm_t1").await);
    assert!(table_exists(&db, "bm_t3").await);

    // 基线标记文件里写 DDL → 显式报错（绝不静默忽略内容）
    let bad_dir = temp_dir("baseline_marker_bad");
    write(
        &bad_dir,
        "B9__baseline.sql",
        "CREATE TABLE should_not_run (id INTEGER)",
    );
    let err = Migrator::from_dir(&bad_dir)
        .run(&db)
        .await
        .expect_err("基线标记内不得写 DDL");
    assert!(matches!(err, DbError::Migration(_)), "got {err:?}");
    assert!(err.to_string().contains("baseline marker"), "{err}");
    assert!(!table_exists(&db, "should_not_run").await);
}

// ==================== repair ====================

/// `repair` 三件事：接受当前指纹、清理已删除的可重复 / 种子记录、删除失败行。
#[tokio::test]
async fn repair_fixes_checksum_cleans_deleted_and_failed_rows() {
    let db = common::setup_db().await;
    let dir = temp_dir("repair");
    write(
        &dir,
        "V1__create_t1.sql",
        "CREATE TABLE rp_t1 (id INTEGER NOT NULL PRIMARY KEY)",
    );
    write(&dir, "S__seed_a.sql", "INSERT INTO rp_t1 (id) VALUES (1)");

    let migrator = Migrator::from_dir(&dir);
    assert_eq!(migrator.run(&db).await.expect("apply"), 2);

    // 1) 已应用版本化迁移被改写 → 状态 Divergent，run 会中止
    write(
        &dir,
        "V1__create_t1.sql",
        "CREATE TABLE rp_t1 (id INTEGER NOT NULL PRIMARY KEY, extra TEXT)",
    );
    assert!(matches!(
        migrator.run(&db).await,
        Err(DbError::MigrationChecksum { .. })
    ));

    // repair 接受当前内容 → 指纹被重写
    let report = migrator.repair(&db).await.expect("repair");
    assert_eq!(report.repaired_checksums, 1, "{report:?}");
    assert_eq!(
        migrator
            .status(&db)
            .await
            .expect("status")
            .iter()
            .find(|s| s.kind == MigrationKind::Versioned)
            .expect("versioned status")
            .state,
        MigrationState::Applied,
        "repair 后不再 Divergent"
    );
    // 篡改过的 DDL 从未执行：表结构仍是旧的
    let cols: i64 = db
        .scalar("SELECT COUNT(*) FROM pragma_table_info('rp_t1')")
        .await
        .expect("pragma_table_info");
    assert_eq!(cols, 1, "repair 只修记录，不回放 SQL");

    // 2) 删掉种子文件 → 记录变 Deleted，repair 清理
    fs::remove_file(dir.join("S__seed_a.sql")).expect("remove seed file");
    let statuses = migrator.status(&db).await.expect("status");
    assert!(
        statuses.iter().any(|s| s.state == MigrationState::Deleted),
        "{statuses:?}"
    );
    let report = migrator.repair(&db).await.expect("repair deleted");
    assert_eq!(report.deleted_records, 1, "{report:?}");
    assert_eq!(record_count_of(&db, "seed", "seed_a").await, 0);

    // 3) 手工塞一行 success=false（模拟非事务模式失败残留）→ repair 清理
    db.execute(
        "INSERT INTO schema_migrations \
         (kind, name, version, checksum, applied_via, applied_at, applied_seq, execution_ms, success, source) \
         VALUES ('versioned', 'ghost_failed', '99', NULL, 'executed', 0, 999, 1, 0, NULL)",
    )
    .await
    .expect("insert failed row");
    let report = migrator.repair(&db).await.expect("repair failed rows");
    assert_eq!(report.removed_failed_records, 1, "{report:?}");
    assert_eq!(record_count_of(&db, "versioned", "ghost_failed").await, 0);
}

/// `repair` 不清理版本化迁移的 `Missing`（清单外版本属其他来源）。
#[tokio::test]
async fn repair_keeps_missing_versioned_records() {
    let db = common::setup_db().await;
    let dir = temp_dir("repair_missing");
    write(
        &dir,
        "V1__create_keep.sql",
        "CREATE TABLE rk_t1 (id INTEGER NOT NULL PRIMARY KEY)",
    );
    assert_eq!(Migrator::from_dir(&dir).run(&db).await.expect("apply"), 1);

    // 塞一条清单外的高版本记录（模拟多来源中的另一个目录）
    db.execute(
        "INSERT INTO schema_migrations \
         (kind, name, version, checksum, applied_via, applied_at, applied_seq, execution_ms, success, source) \
         VALUES ('versioned', 'from_other_source', '77', NULL, 'executed', 0, 77, 1, 1, NULL)",
    )
    .await
    .expect("insert external row");

    let report = Migrator::from_dir(&dir).repair(&db).await.expect("repair");
    assert_eq!(
        report.deleted_records, 0,
        "版本化 Missing 不得被清理：{report:?}"
    );
    assert_eq!(
        record_count_of(&db, "versioned", "from_other_source").await,
        1
    );
}

// ==================== dry_run ====================

/// `dry_run` 真实执行后回滚：语句错误会被暴露，但表与记录都不落库。
#[tokio::test]
async fn dry_run_previews_without_persisting() {
    let db = common::setup_db().await;
    let dir = temp_dir("dry_run");
    write(
        &dir,
        "V1__create_dr.sql",
        "CREATE TABLE dr_t1 (id INTEGER NOT NULL PRIMARY KEY)",
    );
    write(
        &dir,
        "V2__insert_dr.sql",
        "INSERT INTO dr_t1 (id) VALUES (1)",
    );

    let migrator = Migrator::from_dir(&dir);
    let report = migrator.dry_run(&db).await.expect("dry run");
    assert_eq!(report.previewed, 2, "{report:?}");
    assert!(report.skipped_non_transactional.is_empty());

    // 预演不留痕：表不存在、记录表为空
    assert!(!table_exists(&db, "dr_t1").await, "预演不得建表");
    assert_eq!(record_count(&db).await, 0, "预演不得写记录");
    // 预演也不影响后续真实迁移
    assert_eq!(migrator.run(&db).await.expect("real run"), 2);
    assert!(table_exists(&db, "dr_t1").await);

    // 脚本本身有错（表不存在）→ 预演显式报错，且不留痕
    let bad = temp_dir("dry_run_bad");
    write(
        &bad,
        "V1__bad.sql",
        "INSERT INTO nonexistent_table (id) VALUES (1)",
    );
    let fresh = common::setup_db().await;
    assert!(Migrator::from_dir(&bad).dry_run(&fresh).await.is_err());
    assert_eq!(record_count(&fresh).await, 0);
}

/// 标了 `-- +no-transaction` 的迁移无法预演，被显式跳过并记录名称。
#[tokio::test]
async fn dry_run_skips_non_transactional_migrations() {
    let db = common::setup_db().await;
    let dir = temp_dir("dry_run_no_tx");
    write(
        &dir,
        "V1__no_tx.sql",
        "-- +no-transaction\nCREATE TABLE drnt_t1 (id INTEGER NOT NULL PRIMARY KEY)",
    );
    write(
        &dir,
        "V2__tx.sql",
        "CREATE TABLE drnt_t2 (id INTEGER NOT NULL PRIMARY KEY)",
    );

    let report = Migrator::from_dir(&dir)
        .dry_run(&db)
        .await
        .expect("dry run");
    assert_eq!(report.previewed, 1, "{report:?}");
    assert_eq!(
        report.skipped_non_transactional,
        vec!["no_tx".to_string()],
        "{report:?}"
    );
    assert!(!table_exists(&db, "drnt_t1").await, "跳过的迁移未执行");
    assert!(
        !table_exists(&db, "drnt_t2").await,
        "事务性迁移回滚后不留表"
    );
}

// ==================== 可配置前缀 ====================

/// 自定义种类前缀端到端生效：`Fn__` 可重复、`Dict__` 种子，均能被识别并执行。
#[tokio::test]
async fn custom_kind_prefixes_apply_end_to_end() {
    let db = common::setup_db().await;
    let dir = temp_dir("custom_prefix");
    write(
        &dir,
        "V1__create_base.sql",
        "CREATE TABLE cp_base (id INTEGER NOT NULL PRIMARY KEY, n INTEGER NOT NULL DEFAULT 0)",
    );
    write(
        &dir,
        "Fn__custom_view.sql",
        "CREATE VIEW cp_view AS SELECT 1 AS n",
    );
    write(
        &dir,
        "Dict__seed_row.sql",
        "INSERT INTO cp_base (id, n) VALUES (1, 42)",
    );

    let migrator = Migrator::from_dir(&dir).options(MigratorOptions {
        repeatable_prefix: "Fn".into(),
        seed_prefix: "Dict".into(),
        ..MigratorOptions::default()
    });

    // 三个文件全部被正确归类：版本化 1 + 可重复 1 + 种子 1
    assert_eq!(migrator.run(&db).await.expect("apply"), 3);
    let statuses = migrator.status(&db).await.expect("status");
    let kinds: Vec<&str> = statuses.iter().map(|s| s.kind.as_str()).collect();
    assert!(kinds.contains(&"repeatable"), "{statuses:?}");
    assert!(kinds.contains(&"seed"), "{statuses:?}");
    assert!(
        statuses.iter().all(|s| s.state == MigrationState::Applied),
        "{statuses:?}"
    );

    assert!(
        db.find_first("SELECT name FROM sqlite_master WHERE type = 'view' AND name = 'cp_view'")
            .await
            .expect("sqlite_master")
            .is_some(),
        "自定义前缀的可重复迁移已执行"
    );
    let n: i64 = db
        .scalar("SELECT n FROM cp_base WHERE id = 1")
        .await
        .expect("seed row");
    assert_eq!(n, 42, "自定义前缀的种子迁移已执行");

    // 内容变化后仍按「可重复」语义重跑
    write(
        &dir,
        "Dict__seed_row.sql",
        "UPDATE cp_base SET n = 43 WHERE id = 1",
    );
    assert_eq!(migrator.run(&db).await.expect("re-seed"), 1);
    let n: i64 = db
        .scalar("SELECT n FROM cp_base WHERE id = 1")
        .await
        .expect("re-seeded row");
    assert_eq!(n, 43);
}

/// 前缀配置冲突必须在**执行前**中止，且不产生任何副作用。
#[tokio::test]
async fn conflicting_prefix_configuration_aborts_before_touching_the_database() {
    let db = common::setup_db().await;
    let dir = temp_dir("bad_prefix");
    write(
        &dir,
        "V1__create_t1.sql",
        "CREATE TABLE bp_t1 (id INTEGER NOT NULL PRIMARY KEY)",
    );

    let migrator = Migrator::from_dir(&dir).options(MigratorOptions {
        repeatable_prefix: "X".into(),
        seed_prefix: "X".into(),
        ..MigratorOptions::default()
    });

    let err = migrator.run(&db).await.expect_err("重复前缀必须报错");
    assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
    assert!(!table_exists(&db, "bp_t1").await, "配置错误不得部分应用");
    // status / dry_run 走同一加载路径，同样被拦下
    assert!(migrator.status(&db).await.is_err());
    assert!(migrator.dry_run(&db).await.is_err());
}

// ==================== 旧表导入开关 ====================

/// `import_legacy` 默认关闭：即便库里有 `_sqlx_migrations`，也不会被读取。
#[tokio::test]
async fn legacy_import_is_opt_in() {
    let db = common::setup_db().await;
    seed_legacy_table(&db).await;

    let dir = temp_dir("legacy_off");
    write(
        &dir,
        "V1__create_li.sql",
        "CREATE TABLE li_t1 (id INTEGER NOT NULL PRIMARY KEY)",
    );

    // 默认（import_legacy = false）：不看旧表 → v1 被真正执行
    assert_eq!(
        Migrator::from_dir(&dir).run(&db).await.expect("apply"),
        1,
        "默认不导入旧表，v1 应被执行"
    );
    assert!(table_exists(&db, "li_t1").await);
}

/// `import_legacy = true`：旧表记录被还原，已应用版本不再重复执行。
#[tokio::test]
async fn legacy_import_when_enabled_skips_applied_versions() {
    let db = common::setup_db().await;
    seed_legacy_table(&db).await;
    // 存量库中 v1 的产物已存在
    db.execute("CREATE TABLE li_t1 (id INTEGER NOT NULL PRIMARY KEY)")
        .await
        .expect("historical table");

    let dir = temp_dir("legacy_on");
    write(
        &dir,
        "V1__create_li.sql",
        "CREATE TABLE li_t1 (id INTEGER NOT NULL PRIMARY KEY)",
    );
    write(
        &dir,
        "V2__create_li2.sql",
        "CREATE TABLE li_t2 (id INTEGER NOT NULL PRIMARY KEY)",
    );

    let migrator = Migrator::from_dir(&dir).options(MigratorOptions {
        import_legacy: true,
        ..MigratorOptions::default()
    });
    assert_eq!(migrator.run(&db).await.expect("apply"), 1, "仅应用 v2");
    assert!(table_exists(&db, "li_t2").await);

    let name: String = db
        .scalar("SELECT name FROM schema_migrations WHERE kind = 'versioned' AND version = '1'")
        .await
        .expect("imported record");
    assert_eq!(name, "create_li", "旧表描述还原为迁移名");
}

/// 在库里造一张旧框架的 `_sqlx_migrations` 并写入 v1。
async fn seed_legacy_table(db: &Db) {
    db.execute(
        "CREATE TABLE _sqlx_migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            installed_on TEXT NOT NULL,
            success BOOLEAN NOT NULL,
            checksum BLOB NOT NULL
        )",
    )
    .await
    .expect("create legacy table");
    db.execute(
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum) \
         VALUES (1, 'create_li', '2024-01-01 00:00:00', TRUE, X'00')",
    )
    .await
    .expect("seed legacy row");
}

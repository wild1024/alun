//! 第三期能力测试：编译期嵌入迁移（`migrate!`）、占位符替换、`reset` / `fresh`。
//!
//! 对照设计文档 §10 第三期。

mod common;

use std::collections::HashMap;
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
        .join("migrate_phase3")
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

/// 写入一对 up/down 迁移文件（`reset` / `fresh` 需要可逆迁移）。
fn write_pair(dir: &Path, version: u32, name: &str, table: &str) {
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

/// 对象（表 / 视图 / 索引）是否存在。
async fn object_exists(db: &Db, kind: &str, name: &str) -> bool {
    db.find_first_with_params(
        "SELECT name FROM sqlite_master WHERE type = ? AND name = ?",
        vec![Value::Text(kind.to_string()), Value::Text(name.to_string())],
    )
    .await
    .expect("sqlite_master")
    .is_some()
}

/// 记录表当前行数；记录表尚未创建（校验期即中止，未触碰库）时视为 0 行。
async fn record_count(db: &Db) -> i64 {
    if !table_exists(db, "schema_migrations").await {
        return 0;
    }
    db.scalar("SELECT COUNT(*) FROM schema_migrations")
        .await
        .expect("count records")
}

// ==================== 编译期嵌入迁移（`migrate!`） ====================

/// `migrate!` 把 `tests/fixtures/embedded` 编入二进制，分类与目录来源完全一致。
///
/// 夹具目录含：`V1__create_emb.sql`、`sub/V2__add_emb_col.sql`（子目录）、
/// `R__emb_view.sql`（可重复）。
#[tokio::test]
async fn embedded_migrations_apply_without_touching_disk_at_runtime() {
    let db = common::setup_db().await;

    // 编译期已 include_str!，此处不读任何文件
    let migrator = alun_db::migrate!("tests/fixtures/embedded");
    assert_eq!(migrator.run(&db).await.expect("apply embedded"), 3);

    assert!(table_exists(&db, "emb_t1").await, "V1 已执行");
    // V2 的 ALTER 生效 → 列存在且可写
    db.execute("INSERT INTO emb_t1 (id, n) VALUES (1, 7)")
        .await
        .expect("V2 的列已建");
    assert!(object_exists(&db, "view", "emb_view").await, "R__ 已执行");

    // 分类正确：版本化 2 条 + 可重复 1 条
    let statuses = migrator.status(&db).await.expect("status");
    let repeatable = statuses
        .iter()
        .filter(|s| s.kind == MigrationKind::Repeatable)
        .count();
    let versioned = statuses
        .iter()
        .filter(|s| s.kind == MigrationKind::Versioned)
        .count();
    assert_eq!((versioned, repeatable), (2, 1), "{statuses:?}");
    assert!(
        statuses.iter().all(|s| s.state == MigrationState::Applied),
        "{statuses:?}"
    );

    // 幂等
    assert_eq!(migrator.run(&db).await.expect("re-run"), 0);
}

/// 嵌入来源可与目录来源叠加；两侧版本号冲突时显式报错。
#[tokio::test]
async fn embedded_and_directory_sources_share_one_version_space() {
    let db = common::setup_db().await;
    let dir = temp_dir("embedded_conflict");
    // 与夹具里的 V1 撞版本号，但名称不同 → 版本号必须全局唯一
    write(
        &dir,
        "V1__create_other.sql",
        "CREATE TABLE other_t1 (id INTEGER NOT NULL PRIMARY KEY)",
    );

    let err = alun_db::migrate!("tests/fixtures/embedded")
        .source_dir(&dir)
        .run(&db)
        .await
        .expect_err("跨来源重复版本必须报错");
    assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
    assert!(
        err.to_string().contains("duplicate migration version 1"),
        "{err}"
    );
    assert!(!table_exists(&db, "emb_t1").await, "冲突清单不得部分应用");
}

/// 嵌入来源可继续链式追加种子目录（`migrate!` 返回的就是普通 `Migrator`）。
#[tokio::test]
async fn embedded_migrator_accepts_seed_sources() {
    let db = common::setup_db().await;
    let seed_dir = temp_dir("embedded_seed");
    write(
        &seed_dir,
        "emb_seed.sql",
        "INSERT INTO emb_t1 (id, n) VALUES (2, 9)",
    );

    let migrator = alun_db::migrate!("tests/fixtures/embedded").source_seed(&seed_dir);
    assert_eq!(
        migrator.run(&db).await.expect("apply"),
        4,
        "3 条嵌入 + 1 条种子"
    );

    let n: i64 = db
        .scalar("SELECT n FROM emb_t1 WHERE id = 2")
        .await
        .expect("seed row");
    assert_eq!(n, 9);
}

// ==================== 占位符替换 ====================

/// 占位符替换端到端生效，且**指纹覆盖替换后的 SQL**。
#[tokio::test]
async fn placeholders_are_substituted_and_covered_by_checksum() {
    let db = common::setup_db().await;
    let dir = temp_dir("placeholders");
    write(
        &dir,
        "V1__create_widgets.sql",
        "CREATE TABLE ${table_prefix}_widgets (id INTEGER NOT NULL PRIMARY KEY)",
    );

    let mut options = MigratorOptions::default();
    options
        .placeholders
        .insert("table_prefix".into(), "ph".into());

    let migrator = Migrator::from_dir(&dir).options(options);
    assert_eq!(migrator.run(&db).await.expect("apply"), 1);
    assert!(
        table_exists(&db, "ph_widgets").await,
        "替换后的表名才是真正执行的 DDL"
    );

    // 幂等：同一取值重复运行不受影响
    assert_eq!(migrator.run(&db).await.expect("re-run"), 0);

    // 改动占位符取值 = 实际执行的 SQL 变了 → 必须被指纹拦下，而不是静默跑新的 SQL
    let mut changed = MigratorOptions::default();
    changed
        .placeholders
        .insert("table_prefix".into(), "other".into());
    let err = Migrator::from_dir(&dir)
        .options(changed)
        .run(&db)
        .await
        .expect_err("占位符取值变化必须触发指纹校验失败");
    assert!(
        matches!(err, DbError::MigrationChecksum { .. }),
        "got {err:?}"
    );
    assert!(
        !table_exists(&db, "other_widgets").await,
        "不得执行改动后的 SQL"
    );
}

/// 引用未配置的占位符 → 执行前显式报错，不留任何痕迹。
#[tokio::test]
async fn unknown_placeholder_aborts_before_touching_the_database() {
    let db = common::setup_db().await;
    let dir = temp_dir("placeholder_unknown");
    write(
        &dir,
        "V1__create_t1.sql",
        "CREATE TABLE ${nope}_t1 (id INTEGER NOT NULL PRIMARY KEY)",
    );

    let mut options = MigratorOptions::default();
    options.placeholders.insert("known".into(), "k".into());

    let err = Migrator::from_dir(&dir)
        .options(options)
        .run(&db)
        .await
        .expect_err("未知占位符必须报错");
    assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
    let text = err.to_string();
    assert!(text.contains("nope") && text.contains("known"), "{text}");
    assert!(!table_exists(&db, "k_t1").await, "不得部分应用");
    assert_eq!(record_count(&db).await, 0);
}

/// 未配置占位符时不扫描：含 `${` 的字符串字面量照常入库。
#[tokio::test]
async fn placeholder_syntax_inside_string_literals_is_left_alone_by_default() {
    let db = common::setup_db().await;
    let dir = temp_dir("placeholder_literal");
    write(
        &dir,
        "V1__create_template.sql",
        "CREATE TABLE tpl (body TEXT NOT NULL);\n\
         INSERT INTO tpl (body) VALUES ('hello ${name}');",
    );

    // 默认（无占位符配置）→ 整段照常执行
    assert_eq!(Migrator::from_dir(&dir).run(&db).await.expect("apply"), 1);
    let body: String = db
        .scalar("SELECT body FROM tpl")
        .await
        .expect("seeded literal");
    assert_eq!(body, "hello ${name}", "空占位符表不得改写字面量");
}

// ==================== reset / fresh ====================

/// `reset` / `fresh` 默认被安全门拦住，且不留任何副作用。
#[tokio::test]
async fn destructive_operations_are_gated_by_default() {
    let db = common::setup_db().await;
    let dir = temp_dir("gate");
    write_pair(&dir, 1, "create_t1", "gate_t1");

    let migrator = Migrator::from_dir(&dir);
    assert_eq!(migrator.run(&db).await.expect("apply"), 1);

    for err in [
        migrator.reset(&db).await.expect_err("reset 默认必须被拒绝"),
        migrator.fresh(&db).await.expect_err("fresh 默认必须被拒绝"),
    ] {
        assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
        assert!(err.to_string().contains("is_destructive_allowed"), "{err}");
    }
    assert!(table_exists(&db, "gate_t1").await, "被拒绝的操作不得改动库");
    assert_eq!(record_count(&db).await, 1);
}

/// `reset` 逆序回滚全部迁移；`fresh` 随后重建。
#[tokio::test]
async fn reset_rolls_back_everything_and_fresh_rebuilds() {
    let db = common::setup_db().await;
    let dir = temp_dir("reset_fresh");
    write_pair(&dir, 1, "create_t1", "rf_t1");
    write_pair(&dir, 2, "create_t2", "rf_t2");
    write_pair(&dir, 3, "create_t3", "rf_t3");

    let migrator = Migrator::from_dir(&dir).options(MigratorOptions {
        is_destructive_allowed: true,
        ..MigratorOptions::default()
    });
    assert_eq!(migrator.run(&db).await.expect("apply"), 3);

    // reset：三条全部回滚（V3 的 down 先执行，故 V2 的 down 仍安全）
    let report = migrator.reset(&db).await.expect("reset");
    assert_eq!(report.rolled_back, 3, "{report:?}");
    assert_eq!(report.cleared_records, 0, "{report:?}");
    assert_eq!(record_count(&db).await, 0);
    assert!(!table_exists(&db, "rf_t1").await);
    assert!(!table_exists(&db, "rf_t3").await);

    // reset 之后可再次重置（幂等，无记录即无操作）
    let report = migrator.reset(&db).await.expect("reset again");
    assert_eq!(report.rolled_back, 0);

    // 重新应用，再验证 fresh 的「重置 + 重建」
    assert_eq!(migrator.run(&db).await.expect("re-apply"), 3);

    // fresh：先逆序回滚全部，再重建
    let report = migrator.fresh(&db).await.expect("fresh");
    assert_eq!(report.reset.rolled_back, 3, "{report:?}");
    assert_eq!(report.applied, 3, "{report:?}");
    assert!(table_exists(&db, "rf_t1").await);
    assert!(table_exists(&db, "rf_t3").await);
    assert_eq!(record_count(&db).await, 3);
}

/// `reset` 不执行**无指纹记录**（基线标记 / 旧框架导入）的 `down`，只清记录。
#[tokio::test]
async fn reset_clears_baseline_records_without_running_their_down() {
    let db = common::setup_db().await;
    let dir = temp_dir("reset_baseline");
    write_pair(&dir, 1, "create_b1", "rb_t1");
    write_pair(&dir, 2, "create_b2", "rb_t2");
    write_pair(&dir, 3, "create_b3", "rb_t3");

    let migrator = Migrator::from_dir(&dir).options(MigratorOptions {
        is_destructive_allowed: true,
        ..MigratorOptions::default()
    });
    // 存量库接入：v1 视为已应用（从未执行），随后只跑 v2、v3
    assert_eq!(
        migrator
            .baseline(&db, Version::new("1"))
            .await
            .expect("baseline"),
        1
    );
    assert_eq!(migrator.run(&db).await.expect("apply"), 2);
    assert!(!table_exists(&db, "rb_t1").await, "基线不执行 SQL");
    assert!(table_exists(&db, "rb_t2").await);

    // reset：v3、v2 执行 down；v1 只清记录——它对 rb_t1 的 down 会被跳过，
    // 否则就会去 DROP 一张从未建过的表
    let report = migrator.reset(&db).await.expect("reset");
    assert_eq!(report.rolled_back, 2, "{report:?}");
    assert_eq!(report.cleared_records, 1, "{report:?}");
    assert_eq!(record_count(&db).await, 0);
}

/// `reset` 遇到无 `down` 的迁移时显式报错，而不是静默跳过留下半截状态。
#[tokio::test]
async fn reset_reports_migrations_without_down_scripts() {
    let db = common::setup_db().await;
    let dir = temp_dir("reset_no_down");
    write(
        &dir,
        "V1__create_t1.sql",
        "CREATE TABLE rnd_t1 (id INTEGER NOT NULL PRIMARY KEY)",
    );

    let migrator = Migrator::from_dir(&dir).options(MigratorOptions {
        is_destructive_allowed: true,
        ..MigratorOptions::default()
    });
    assert_eq!(migrator.run(&db).await.expect("apply"), 1);

    let err = migrator.reset(&db).await.expect_err("缺 down 必须报错");
    assert!(matches!(err, DbError::Migration(_)), "got {err:?}");
    assert!(err.to_string().contains("no down script"), "{err}");
    assert!(table_exists(&db, "rnd_t1").await, "报错时不得改动库");
    assert_eq!(record_count(&db).await, 1, "记录不得被删");
}

/// 占位符表与破坏性开关都能从 `MigratorOptions` 直接配置（编译期覆盖）。
#[tokio::test]
async fn options_are_composable_for_phase_three_features() {
    let db = common::setup_db().await;
    let dir = temp_dir("options_compose");
    write_pair(&dir, 1, "create_t1", "oc_t1");
    write(
        &dir,
        "V2__create_prefixed.sql",
        "CREATE TABLE ${prefix}_extra (id INTEGER NOT NULL PRIMARY KEY)",
    );

    let mut placeholders = HashMap::new();
    placeholders.insert("prefix".to_string(), "oc".to_string());

    let migrator = Migrator::from_dir(&dir).options(MigratorOptions {
        placeholders,
        is_destructive_allowed: true,
        ..MigratorOptions::default()
    });

    assert_eq!(migrator.run(&db).await.expect("apply"), 2);
    assert!(table_exists(&db, "oc_extra").await);

    // V2（最近应用的一条）无 down 脚本 → reset 立即报错，报错前不得有任何改动
    let err = migrator
        .reset(&db)
        .await
        .expect_err("缺 down 的迁移必须让 reset 报错");
    assert!(matches!(err, DbError::Migration(_)), "got {err:?}");
    assert!(err.to_string().contains("no down script"), "{err}");
    assert!(
        table_exists(&db, "oc_t1").await,
        "报错时不得回滚已应用的迁移"
    );
    assert!(
        table_exists(&db, "oc_extra").await,
        "报错时不得回滚已应用的迁移"
    );
    assert_eq!(record_count(&db).await, 2, "记录不得被删");
}

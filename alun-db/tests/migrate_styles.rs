//! 真实项目迁移风格的端到端用例（设计文档 §10 第一期）。
//!
//! 三组风格来自真实项目，命名与目录布局**不做任何改名**即须可加载：
//!
//! | 组 | 来源项目 | 命名 / 布局 | 关键能力 |
//! |---|---|---|---|
//! | A | jinhu | 扁平目录、`NN_name.sql`、单文件数百条语句、无 down | 多语句拆分 + 宽松前缀解析 |
//! | B | facai | `NN_name.up.sql`（仅部分版本配 `.down.sql`）、up-only 合法 | up-only 回滚须显式报错 |
//! | C | x | `V{NNNN}__name.up.sql` + `system/`、`support/` 子目录 | 多来源 + 目录递归 + 跨目录版本唯一 |
//! | D | —— | PostgreSQL `$tag$…$tag$` 函数体 | `$$` 内的 `;` 不是语句边界 |

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use alun_db::migrate::{DialectKind, parse_directives};
use alun_db::prelude::*;
use alun_db_core::error::DbError;

// ==================== 临时目录与断言辅助 ====================

/// 并行测试各用独立临时目录（位于包内 `target/`，与其余迁移测试同约定）。
fn temp_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = PathBuf::from("target")
        .join("migrate_styles")
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

/// 表是否存在（SQLite catalog）。
async fn table_exists(db: &Db, table: &str) -> bool {
    db.find_first_with_params(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?",
        vec![Value::Text(table.to_string())],
    )
    .await
    .expect("sqlite_master")
    .is_some()
}

/// 元数据对象（表 / 视图 / 索引）是否存在。
async fn object_exists(db: &Db, kind: &str, name: &str) -> bool {
    db.find_first_with_params(
        "SELECT name FROM sqlite_master WHERE type = ? AND name = ?",
        vec![Value::Text(kind.to_string()), Value::Text(name.to_string())],
    )
    .await
    .expect("sqlite_master")
    .is_some()
}

/// 记录表中某条迁移的名称与版本（以 PRAGMA 风格的动态条件查询）。
async fn recorded_name(db: &Db, version: &str) -> Option<String> {
    db.find_first_with_params(
        "SELECT name FROM schema_migrations WHERE version = ?",
        vec![Value::Text(version.to_string())],
    )
    .await
    .expect("schema_migrations")
    .map(|row| String::from_value(row.values().next().expect("name")).expect("name is text"))
}

// ==================== A 组：jinhu 风格 ====================

/// jinhu 风格：扁平目录、纯数字前缀 + 单下划线、**单文件多语句**、无 down 文件。
///
/// 这一组是「拆分器是第一期硬需求」的直接证据：`01_base_tables_init.sql`
/// 单文件含数百条语句，不拆分则无法执行。
#[tokio::test]
async fn jinhu_style_flat_single_file_with_many_statements() {
    let db = common::setup_db().await;
    let dir = temp_dir("jinhu");

    let mut sql = String::from("-- 基础表初始化（jinhu 风格：单文件多语句、无 down）\n\n");
    for i in 0..30 {
        sql.push_str(&format!(
            "CREATE TABLE jh_t{i} (\n  id INTEGER NOT NULL PRIMARY KEY,\n  name TEXT NOT NULL\n);\n"
        ));
    }
    // 注释、空行与跨行语句混排，均不得干扰拆分
    sql.push_str("\n-- 索引与视图\n");
    for i in 0..30 {
        sql.push_str(&format!("CREATE INDEX jh_idx_t{i}\n  ON jh_t{i} (name);\n"));
    }
    sql.push_str("CREATE VIEW jh_visible AS SELECT id, name FROM jh_t0;\n");
    write(&dir, "01_base_tables_init.sql", &sql);

    let migrator = Migrator::from_dir(&dir);
    let applied = migrator.run(&db).await.expect("jinhu 风格必须能加载并应用");
    assert_eq!(applied, 1, "单文件即一条迁移");

    // 文件名解析结果：纯数字前缀 + 单下划线，无版本前缀要求
    assert_eq!(
        recorded_name(&db, "1").await.as_deref(),
        Some("base_tables_init")
    );

    assert!(
        table_exists(&db, "jh_t29").await,
        "最后一条 CREATE TABLE 必须执行"
    );
    assert!(object_exists(&db, "index", "jh_idx_t29").await, "索引已建");
    assert!(object_exists(&db, "view", "jh_visible").await, "视图已建");
    db.execute("INSERT INTO jh_t0 (id, name) VALUES (1, 'a')")
        .await
        .expect("多语句文件建出的表可写");

    // 幂等：再跑不重复应用（拆分结果不落库，重跑靠记录表判定）
    assert_eq!(migrator.run(&db).await.expect("re-run"), 0);
}

/// jinhu 风格的多文件演进：`NN_name.sql` 之间按**数值**升序（`10` 在 `9` 之后）。
#[tokio::test]
async fn jinhu_style_numeric_prefix_orders_before_lexicographic() {
    let db = common::setup_db().await;
    let dir = temp_dir("jinhu_order");

    // 版本 9 先建表，版本 10 用 ALTER 依赖它——按字典序 `10` 会排在 `9` 前而失败
    write(
        &dir,
        "09_inventory.sql",
        "CREATE TABLE jh_inv (id INTEGER NOT NULL PRIMARY KEY)",
    );
    write(
        &dir,
        "10_inventory_ext.sql",
        "ALTER TABLE jh_inv ADD COLUMN qty INTEGER NOT NULL DEFAULT 0",
    );

    assert_eq!(Migrator::from_dir(&dir).run(&db).await.expect("apply"), 2);
    db.execute("INSERT INTO jh_inv (id, qty) VALUES (1, 5)")
        .await
        .expect("版本 10 的 ALTER 必须已应用");
}

// ==================== B 组：facai 风格 ====================

/// facai 风格：`.up.sql` 后缀、**up-only 版本合法**，回滚到 up-only 版本须显式报错。
///
/// 真实目录中仅 `31/32/33` 配有 `.down.sql`，`34`–`44` 全是 up-only。
#[tokio::test]
async fn facai_style_paired_and_up_only_versions() {
    let db = common::setup_db().await;
    let dir = temp_dir("facai");

    write(
        &dir,
        "01_system.up.sql",
        "CREATE TABLE fc_dict (id INTEGER NOT NULL PRIMARY KEY, code TEXT NOT NULL)",
    );
    // 配对版本：up + down 同名同版本
    write(
        &dir,
        "31_storage_backend.up.sql",
        "CREATE TABLE fc_storage (id INTEGER NOT NULL PRIMARY KEY)",
    );
    write(&dir, "31_storage_backend.down.sql", "DROP TABLE fc_storage");
    // up-only 版本（真实目录里占多数）
    write(
        &dir,
        "34_admin_optimize.up.sql",
        "ALTER TABLE fc_dict ADD COLUMN label TEXT NOT NULL DEFAULT ''",
    );

    let migrator = Migrator::from_dir(&dir);
    assert_eq!(
        migrator.run(&db).await.expect("apply"),
        3,
        "三条迁移全部可加载"
    );

    // 记录的身份：名称取自文件名，不因 up/down 后缀而改变
    assert_eq!(
        recorded_name(&db, "31").await.as_deref(),
        Some("storage_backend")
    );
    assert_eq!(
        recorded_name(&db, "34").await.as_deref(),
        Some("admin_optimize")
    );
    // up + down 两个文件是**一条**迁移，不得记成两条
    let paired_rows: i64 = db
        .scalar("SELECT COUNT(*) FROM schema_migrations WHERE version = '31'")
        .await
        .expect("count version 31");
    assert_eq!(paired_rows, 1);

    // 最近应用的是 34（up-only）→ 回滚必须显式报错，且不动任何状态
    let err = migrator
        .rollback(&db)
        .await
        .expect_err("up-only 版本回滚必须报错");
    assert!(matches!(err, DbError::Migration(_)), "got {err:?}");
    assert!(err.to_string().contains("no down script"), "{err}");
    assert_eq!(
        recorded_name(&db, "34").await.as_deref(),
        Some("admin_optimize")
    );

    // LIFO 严格逆序：34 挡在最前面且不可回滚 → 配对版本 31 也无法被越过回滚
    let err = migrator.rollback(&db).await.expect_err("仍须报错");
    assert!(err.to_string().contains("no down script"), "{err}");
    assert!(table_exists(&db, "fc_storage").await, "不得越过 34 回滚 31");
    assert!(table_exists(&db, "fc_dict").await);
    assert_eq!(
        recorded_name(&db, "31").await.as_deref(),
        Some("storage_backend")
    );
}

/// facai 风格的局部可逆：up-only 版本可用单文件 `-- +migrate Up/Down` 双段式自带回退。
#[tokio::test]
async fn facai_style_two_section_single_file_is_reversible() {
    let db = common::setup_db().await;
    let dir = temp_dir("facai_sections");

    write(
        &dir,
        "40_two_sections.sql",
        "-- +migrate Up\n\
         CREATE TABLE fc_two_section (id INTEGER NOT NULL PRIMARY KEY);\n\
         \n\
         -- +migrate Down\n\
         DROP TABLE fc_two_section;\n",
    );

    let migrator = Migrator::from_dir(&dir);
    assert_eq!(migrator.run(&db).await.expect("apply"), 1);
    assert!(table_exists(&db, "fc_two_section").await, "Up 段已执行");

    // 双段式的 Down 段被识别 → 可回滚（与「无 down」的 up-only 版本行为不同）
    assert_eq!(
        migrator
            .rollback(&db)
            .await
            .expect("down")
            .map(|v| v.to_string()),
        Some("40".into())
    );
    assert!(!table_exists(&db, "fc_two_section").await, "Down 段已执行");
}

// ==================== C 组：x 风格（子目录 + 多来源） ====================

/// x 风格：`V{NNNN}__name.up.sql`、按业务域分子目录、多来源合并。
#[tokio::test]
async fn x_style_subdirectories_and_multiple_sources() {
    let db = common::setup_db().await;
    let system = temp_dir("x_system");
    let support = temp_dir("x_support");

    // system 来源：根目录 + **嵌套子目录**（真实项目按业务域分目录）
    write(
        &system,
        "V0002__create_sys_role.up.sql",
        "CREATE TABLE sys_role (id INTEGER NOT NULL PRIMARY KEY, code TEXT NOT NULL)",
    );
    write(
        &system,
        "V0002__create_sys_role.down.sql",
        "DROP TABLE sys_role",
    );
    write(
        &system.join("system"),
        "V0003__create_sys_user.up.sql",
        "CREATE TABLE sys_user (id INTEGER NOT NULL PRIMARY KEY)",
    );
    // support 来源：另一业务域，版本号须与 system **全局互斥**
    write(
        &support,
        "V0003__create_notice.up.sql",
        "CREATE TABLE sup_notice (id INTEGER NOT NULL PRIMARY KEY)",
    );

    let migrator = Migrator::new()
        .source_dir(&system)
        .source_dir(&support)
        .options(MigratorOptions {
            version_prefix: "V".into(),
            ..MigratorOptions::default()
        });

    // 跨来源版本重复：加载阶段即显式拒绝，且不得「部分应用」
    let err = migrator.run(&db).await.expect_err("跨来源版本重复必须报错");
    assert!(matches!(err, DbError::Migration(_)), "got {err:?}");
    let text = err.to_string();
    assert!(text.contains("version 3"), "文案需指明冲突版本：{text}");
    assert!(
        text.contains("V0003__create_sys_user.up.sql")
            && text.contains("V0003__create_notice.up.sql"),
        "文案需同时给出两个冲突文件路径：{text}"
    );
    assert!(!table_exists(&db, "sys_role").await, "冲突清单不得部分应用");

    // 冲突版本让位后正常合并：版本升序跨目录生效
    fs::remove_file(support.join("V0003__create_notice.up.sql")).expect("remove conflicting file");
    write(
        &support,
        "V0004__create_notice.up.sql",
        "CREATE TABLE sup_notice (id INTEGER NOT NULL PRIMARY KEY)",
    );

    assert_eq!(migrator.run(&db).await.expect("apply"), 3);
    assert!(table_exists(&db, "sys_role").await);
    assert!(
        table_exists(&db, "sys_user").await,
        "嵌套子目录中的迁移已加载"
    );
    assert!(table_exists(&db, "sup_notice").await);

    // plan：全部已应用时为空；status：三条均为 Applied
    assert!(migrator.plan(&db).await.expect("plan").is_empty());
    let statuses = migrator.status(&db).await.expect("status");
    assert_eq!(statuses.len(), 3);
    assert!(
        statuses.iter().all(|s| s.state == MigrationState::Applied),
        "{statuses:?}"
    );
}

// ==================== D 组：`$$` 美元引用拆分 ====================

/// PostgreSQL 美元引用：`$tag$…$tag$` 整体是一个词法单元，内部的 `;` 不是语句边界。
///
/// 拆分器是公开 API，此处在**调用方视角**验证 plpgsql 函数体（jinhu 实测必需）。
#[test]
fn dollar_quoted_function_body_is_not_split() {
    let sql = "\
CREATE FUNCTION jh_touch() RETURNS trigger AS $$\n\
BEGIN\n\
  NEW.updated_at := now();\n\
  RETURN NEW;\n\
END;\n\
$$ LANGUAGE plpgsql;\n\
\n\
CREATE TABLE jh_touched (id BIGINT NOT NULL PRIMARY KEY);\n";

    let statements = split_statements(sql, DialectKind::Postgres).expect("split");
    assert_eq!(
        statements.len(),
        2,
        "函数体内部的 `;` 不得切分：{statements:#?}"
    );
    assert!(
        statements[0].contains("NEW.updated_at := now();"),
        "{}",
        statements[0]
    );
    assert!(
        statements[0].contains("END;\n$$ LANGUAGE plpgsql"),
        "{}",
        statements[0]
    );
    assert!(statements[1].starts_with("CREATE TABLE jh_touched"));

    // 具名标签（`$body$`）与嵌套使用的字符串同样受保护
    let tagged =
        "CREATE FUNCTION jh_f() RETURNS int AS $body$ SELECT length('a;b'); $body$ LANGUAGE sql;";
    let statements = split_statements(tagged, DialectKind::Postgres).expect("split");
    assert_eq!(statements.len(), 1, "{statements:#?}");

    // 单引号与双引号内的 `;`（另一种常见陷阱）在各方言下都不切分
    let quoted = "INSERT INTO jh_t0 (name) VALUES ('a;b'); UPDATE jh_t0 SET name = \"x;y\";";
    let statements = split_statements(quoted, DialectKind::Postgres).expect("split");
    assert_eq!(statements.len(), 2, "{statements:#?}");
}

/// `-- +delimiter` 指令覆盖默认的 `;`（MySQL 存储过程等场景的逃生门）。
#[test]
fn custom_delimiter_directive_overrides_semicolon() {
    let sql = "\
-- +delimiter //
CREATE PROCEDURE fc_noop() BEGIN SELECT 1; END //
CREATE TABLE fc_after (id INTEGER NOT NULL PRIMARY KEY)//
";
    let statements = split_statements(sql, DialectKind::Mysql).expect("split");
    assert_eq!(statements.len(), 2, "{statements:#?}");
    assert!(statements[0].contains("SELECT 1; END"), "{}", statements[0]);
}

/// `-- +no-transaction` 指令被解析（`CREATE INDEX CONCURRENTLY` 等无法进事务的语句）。
#[test]
fn no_transaction_directive_is_parsed() {
    let ok = parse_directives("-- +no-transaction\nCREATE INDEX CONCURRENTLY idx ON jh_t0 (name);");
    assert!(ok.is_no_transaction);
    assert!(ok.delimiter.is_none());

    let plain = parse_directives("CREATE INDEX idx ON jh_t0 (name);");
    assert!(!plain.is_no_transaction, "无指令时保持默认（事务包装）");
}

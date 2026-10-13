//! 存量兼容（旧框架 sqlx 迁移记录导入）与全新库迁移测试。
//!
//! 覆盖 D023（「表不存在」必须穿透驱动包装的 `source` 链判定）与
//! D024（导入语句的参数类型必须与列类型匹配）。
//!
//! PostgreSQL / MySQL 分支各自在**独立库**上做破坏性探测（建/删迁移记录表），
//! 不污染共享测试库；无法创建独立库或连不上服务器时跳过。

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use alun_db::prelude::*;
use alun_db_core::stmt::Value;

/// 写入一对 up/down 迁移文件。
fn write_pair(dir: &Path, version: i64, name: &str, table: &str) {
    fs::write(
        dir.join(format!("V{version}__{name}.up.sql")),
        format!("CREATE TABLE {table} (id BIGINT NOT NULL PRIMARY KEY)"),
    )
    .expect("write up");
    fs::write(
        dir.join(format!("V{version}__{name}.down.sql")),
        format!("DROP TABLE {table}"),
    )
    .expect("write down");
}

/// 唯一临时迁移目录。
fn temp_dir(tag: &str) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = PathBuf::from("target")
        .join("migrate_legacy")
        .join(format!("{tag}_{}_{seq}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// 把连接串末尾的库名替换为 `db_name`（保留查询参数）。
fn swap_db_name(url: &str, db_name: &str) -> String {
    let (head, query) = match url.split_once('?') {
        Some((head, query)) => (head, Some(query)),
        None => (url, None),
    };
    let swapped = match head.rfind('/') {
        Some(idx) => format!("{}{db_name}", &head[..=idx]),
        None => head.to_string(),
    };
    match query {
        Some(query) => format!("{swapped}?{query}"),
        None => swapped,
    }
}

/// 外部数据库连接上限：服务器接受 TCP 却不应答时不能无限阻塞（会挂住整个 `cargo test`）。
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 有界连接：超时与连接错误都折叠为 `None`。
async fn connect_bounded(url: &str) -> Option<Db> {
    match tokio::time::timeout(PROBE_TIMEOUT, Db::connect(url)).await {
        Ok(Ok(db)) => Some(db),
        Ok(Err(e)) => {
            eprintln!("跳过（无法连接 {url}）：{e}");
            None
        }
        Err(_) => {
            eprintln!("跳过（连接 {url} 超时 >{}s）", PROBE_TIMEOUT.as_secs());
            None
        }
    }
}

/// 在独立库上连接（用于破坏性探测）；库不存在则尝试创建，失败即跳过。
async fn isolated_db(url: &str, db_name: &str) -> Option<Db> {
    let admin = connect_bounded(url).await?;
    let _ = admin.execute(&format!("CREATE DATABASE {db_name}")).await;
    let target = swap_db_name(url, db_name);
    connect_bounded(&target).await
}

// ==================== SQLite ====================

#[tokio::test]
async fn legacy_sqlx_rows_are_imported_once_on_sqlite() {
    let db = common::setup_db().await;

    // 旧框架留下的存量迁移表 + 历史版本（含空描述与超长描述）
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
    .expect("legacy table");
    let long_description = "超长描述".repeat(40);
    for (version, description) in [
        (1_i64, "create_legacy_probe".to_string()),
        (2, String::new()),
        (3, long_description),
    ] {
        db.execute_with_params(
            "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum) \
             VALUES (?, ?, '2024-01-01 00:00:00', TRUE, ?)",
            vec![
                Value::I64(version),
                Value::Text(description),
                Value::Bytes(vec![0]),
            ],
        )
        .await
        .expect("legacy row");
    }
    // 存量库中历史迁移已建好的表
    db.execute("CREATE TABLE legacy_probe (id INTEGER NOT NULL PRIMARY KEY)")
        .await
        .expect("historical table");

    let dir = temp_dir("sqlite_legacy");
    write_pair(&dir, 1, "create_legacy_probe", "legacy_probe");

    let migrator = Migrator::from_dir(&dir).options(MigratorOptions {
        import_legacy: true,
        ..MigratorOptions::default()
    });

    // 版本 1 已导入 → 不重复执行；2、3 无对应文件不影响导入
    let applied = migrator
        .run(&db)
        .await
        .expect("导入后跳过历史版本（D023/D024）");
    assert_eq!(applied, 0);

    let count: i64 = db
        .scalar("SELECT COUNT(*) FROM schema_migrations")
        .await
        .expect("count");
    assert_eq!(count, 3, "三条历史版本一次性导入");
    let name: String = db
        .scalar("SELECT name FROM schema_migrations WHERE version = '2'")
        .await
        .expect("name");
    assert_eq!(name, "legacy", "空描述回退为 legacy");
    let long_name_len: i64 = db
        .scalar("SELECT LENGTH(name) FROM schema_migrations WHERE version = '3'")
        .await
        .expect("length");
    assert_eq!(long_name_len, 100, "超长名称按列宽截断");
    let seq: i64 = db
        .scalar("SELECT applied_seq FROM schema_migrations WHERE version = '1'")
        .await
        .expect("seq");
    assert_eq!(seq, 1, "导入行按版本号保持升序");
    // 历史行导入时无内容指纹（`NULL`）；**首次运行**按目录中的同名版本补录指纹，
    // 因此此处应已补录且为 SHA-256 十六进制（后续运行据此检出篡改，D011）。
    let checksum: Option<String> = db
        .scalar("SELECT checksum FROM schema_migrations WHERE version = '1'")
        .await
        .expect("checksum");
    let checksum = checksum.expect("历史行应在首次运行时补录内容指纹");
    assert_eq!(checksum.len(), 64, "SHA-256 十六进制长度：{checksum}");

    // 幂等：再次迁移仍是 no-op
    assert_eq!(migrator.run(&db).await.expect("again"), 0);
}

// ==================== PostgreSQL ====================

#[cfg(feature = "postgres")]
const DEFAULT_PG_URL: &str = "postgres://alun:alun@localhost:5432/alun_test";

#[cfg(feature = "postgres")]
async fn pg_url() -> String {
    std::env::var("ALUN_TEST_PG_URL").unwrap_or_else(|_| DEFAULT_PG_URL.to_string())
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn fresh_postgres_database_migrates_without_legacy_table() {
    let Some(db) = isolated_db(&pg_url().await, "alun_db_probe_fresh").await else {
        return;
    };
    // 全新库：schema_migrations 与 _sqlx_migrations 都不存在
    let _ = db.execute("DROP TABLE IF EXISTS pg_probe").await;
    let _ = db.execute("DROP TABLE IF EXISTS schema_migrations").await;
    let _ = db.execute("DROP TABLE IF EXISTS _sqlx_migrations").await;

    let dir = temp_dir("pg_fresh");
    write_pair(&dir, 1, "create_pg_probe", "pg_probe");

    let applied = Migrator::from_dir(&dir)
        .run(&db)
        .await
        .expect("全新 PG 库必须能迁移（D023：表缺失不得被判成真实错误）");
    assert_eq!(applied, 1);
    let seq: i64 = db
        .scalar("SELECT applied_seq FROM schema_migrations WHERE version = '1'")
        .await
        .expect("applied_seq 列存在");
    assert_eq!(seq, 1);
    let checksum: String = db
        .scalar("SELECT checksum FROM schema_migrations WHERE version = '1'")
        .await
        .expect("checksum 列存在");
    assert_eq!(checksum.len(), 64);
    assert_eq!(Migrator::from_dir(&dir).run(&db).await.expect("re-run"), 0);
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn legacy_sqlx_rows_are_imported_on_postgres() {
    let Some(db) = isolated_db(&pg_url().await, "alun_db_probe_legacy").await else {
        return;
    };
    let _ = db.execute("DROP TABLE IF EXISTS pg_probe").await;
    let _ = db.execute("DROP TABLE IF EXISTS schema_migrations").await;
    let _ = db.execute("DROP TABLE IF EXISTS _sqlx_migrations").await;

    db.execute(
        "CREATE TABLE _sqlx_migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            installed_on TIMESTAMPTZ NOT NULL,
            success BOOLEAN NOT NULL,
            checksum BYTEA NOT NULL
        )",
    )
    .await
    .expect("legacy table");
    db.execute_with_params(
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum) \
         VALUES (?, ?, NOW(), TRUE, ?)",
        vec![
            Value::I64(1),
            Value::Text("create_pg_probe".into()),
            Value::Bytes(vec![0]),
        ],
    )
    .await
    .expect("legacy row");
    db.execute("CREATE TABLE pg_probe (id BIGINT NOT NULL PRIMARY KEY)")
        .await
        .expect("historical table");

    let dir = temp_dir("pg_legacy");
    write_pair(&dir, 1, "create_pg_probe", "pg_probe");

    // D024：导入语句的参数类型不再被 PG 推断为 text
    let migrator = Migrator::from_dir(&dir).options(MigratorOptions {
        import_legacy: true,
        ..MigratorOptions::default()
    });
    let applied = migrator.run(&db).await.expect("导入后跳过历史版本（D024）");
    assert_eq!(applied, 0);
    let name: String = db
        .scalar("SELECT name FROM schema_migrations WHERE version = '1'")
        .await
        .expect("imported row");
    assert_eq!(name, "create_pg_probe");
}

// ==================== MySQL ====================

#[cfg(feature = "mysql")]
const DEFAULT_MYSQL_URL: &str = "mysql://alun:alun@localhost:3306/alun_test";

#[cfg(feature = "mysql")]
async fn mysql_url() -> String {
    std::env::var("ALUN_TEST_MYSQL_URL").unwrap_or_else(|_| DEFAULT_MYSQL_URL.to_string())
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn fresh_mysql_database_migrates_without_legacy_table() {
    let Some(db) = isolated_db(&mysql_url().await, "alun_db_probe_fresh").await else {
        return;
    };
    let _ = db.execute("DROP TABLE IF EXISTS my_probe").await;
    let _ = db.execute("DROP TABLE IF EXISTS schema_migrations").await;
    let _ = db.execute("DROP TABLE IF EXISTS _sqlx_migrations").await;

    let dir = temp_dir("mysql_fresh");
    write_pair(&dir, 1, "create_my_probe", "my_probe");

    let applied = Migrator::from_dir(&dir)
        .run(&db)
        .await
        .expect("全新 MySQL 库必须能迁移（D023）");
    assert_eq!(applied, 1);
    let checksum: String = db
        .scalar("SELECT checksum FROM schema_migrations WHERE version = '1'")
        .await
        .expect("checksum 列存在");
    assert_eq!(checksum.len(), 64);
    assert_eq!(Migrator::from_dir(&dir).run(&db).await.expect("re-run"), 0);
}

#[cfg(feature = "mysql")]
#[tokio::test]
async fn legacy_sqlx_rows_are_imported_on_mysql() {
    let Some(db) = isolated_db(&mysql_url().await, "alun_db_probe_legacy").await else {
        return;
    };
    let _ = db.execute("DROP TABLE IF EXISTS my_probe").await;
    let _ = db.execute("DROP TABLE IF EXISTS schema_migrations").await;
    let _ = db.execute("DROP TABLE IF EXISTS _sqlx_migrations").await;

    db.execute(
        "CREATE TABLE _sqlx_migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            success BOOLEAN NOT NULL,
            checksum BLOB NOT NULL
        )",
    )
    .await
    .expect("legacy table");
    db.execute_with_params(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum) VALUES (?, ?, TRUE, ?)",
        vec![
            Value::I64(1),
            Value::Text("create_my_probe".into()),
            Value::Bytes(vec![0]),
        ],
    )
    .await
    .expect("legacy row");
    db.execute("CREATE TABLE my_probe (id BIGINT NOT NULL PRIMARY KEY)")
        .await
        .expect("historical table");

    let dir = temp_dir("mysql_legacy");
    write_pair(&dir, 1, "create_my_probe", "my_probe");

    let migrator = Migrator::from_dir(&dir).options(MigratorOptions {
        import_legacy: true,
        ..MigratorOptions::default()
    });
    let applied = migrator.run(&db).await.expect("导入后跳过历史版本（D024）");
    assert_eq!(applied, 0);
    let name: String = db
        .scalar("SELECT name FROM schema_migrations WHERE version = '1'")
        .await
        .expect("imported row");
    assert_eq!(name, "create_my_probe");
}

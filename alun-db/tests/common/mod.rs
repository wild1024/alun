//! 测试辅助工具。
//!
//! 共享辅助集：各测试二进制按需取用，未引用的函数不视为死代码。
#![allow(dead_code)]

use alun_db::prelude::*;

pub mod models;

/// 创建内存数据库用于测试。
///
/// 每个测试使用独立的 `:memory:` 数据库，通过 `Db::connect` 直接构造，
/// 不污染全局单例。
pub async fn setup_db() -> Db {
    Db::connect("sqlite::memory:").await.expect("create db")
}

/// 执行建表 SQL。
pub async fn create_users_table(db: &Db) {
    db.execute(
        "CREATE TABLE users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            email TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            status INTEGER NOT NULL DEFAULT 1,
            profile TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .await
    .expect("create users table");
}

/// 创建订单表。
pub async fn create_orders_table(db: &Db) {
    db.execute(
        "CREATE TABLE orders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            amount TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .await
    .expect("create orders table");
}

/// 初始化完整测试环境。
pub async fn setup() -> Db {
    let db = setup_db().await;
    create_users_table(&db).await;
    create_orders_table(&db).await;
    db
}

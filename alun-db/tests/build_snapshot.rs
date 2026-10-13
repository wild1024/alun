//! build.rs 快照方案测试。
//!
//! 本测试文件能编译即证明：`#[model(strict)]` 未显式 `schema` 时，能通过
//! build.rs（`alun_db_build::build_snapshot("tests/migrations")`）产出的
//! `schema_snapshot.sql`（经环境变量 `ALUN_DB_SCHEMA_SNAPSHOT` 注入）完成
//! 编译期列校验。若迁移文件中的表/列与模型不匹配，将在此处编译失败。
//!
//! 迁移目录 `tests/migrations/` 分两个文件：`0001` 创建 `users`，`0002` 以
//! `ALTER TABLE users ADD COLUMN age` 增量演进——`age` 字段的存在即验证了
//! 快照合并逻辑确实跨越多个迁移文件。

mod common;
use alun_db::prelude::*;
use common::*;

/// 快照驱动校验模型：字段列必须全部出现在 build.rs 快照对应表中。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("users")]
#[model(strict)]
pub struct User {
    /// 主键
    #[key]
    pub id: i64,
    /// 名称
    pub name: String,
    /// 邮箱
    pub email: String,
    /// 年龄（由 `0002_add_users_age.sql` 的 ALTER 演进而来）
    pub age: i64,
    /// 软删除时间戳
    #[soft_delete]
    pub deleted_at: Option<String>,
}

#[tokio::test]
async fn snapshot_strict_model_is_usable_at_runtime() {
    let db = setup_db().await;
    db.execute(
        "CREATE TABLE users (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT NOT NULL UNIQUE,
            age INTEGER NOT NULL DEFAULT 0,
            deleted_at TEXT
        )",
    )
    .await
    .expect("create users from migration-equivalent DDL");

    let mut u = User {
        id: 1,
        name: "alice".into(),
        email: "a@x.io".into(),
        age: 30,
        deleted_at: None,
    };
    u.insert_or_update(&db).await.expect("insert user");

    let fetched = User::find(&db, 1).await.expect("find user");
    assert_eq!(fetched, u);
}

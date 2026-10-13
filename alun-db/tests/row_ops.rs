//! 动态行写操作（`Db::insert_row` / `update_row` / `delete_row`）与
//! 模型部分更新（`ChangeSet`）的集成测试。

mod common;

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use common::models::User;

fn str_value(v: &Value) -> String {
    match v {
        Value::Text(s) => s.clone(),
        _ => format!("{v:?}"),
    }
}

fn i64_value(v: &Value) -> i64 {
    match v {
        Value::I64(i) => *i,
        Value::I32(i) => *i as i64,
        _ => panic!("not an int: {v:?}"),
    }
}

#[tokio::test]
async fn insert_row_roundtrip() {
    let db = common::setup().await;

    // 携带实体元数据：表名 users、主键字段 id；自增主键留空
    let mut row = Row::new().with_entity("users", "id");
    row.insert("email", "a@example.com");
    row.insert("name", "Alice");
    row.insert("status", 1);
    row.insert("created_at", "2026-01-01T00:00:00Z");
    row.insert("updated_at", "2026-01-01T00:00:00Z");
    let inserted = db.insert_row(&row).await.expect("insert row");
    // 自增主键回填 + 字段保留
    assert!(inserted.get("id").is_some());
    assert_eq!(inserted.get("name").map(str_value), Some("Alice".into()));
    assert_eq!(inserted.table(), Some("users"));
}

#[tokio::test]
async fn row_ops_require_entity_metadata() {
    let db = common::setup().await;

    // 缺少表名元数据时插入应报错而非 panic
    let mut row = Row::new();
    row.insert("email", "x@example.com");
    row.insert("name", "X");
    let err = db.insert_row(&row).await.unwrap_err();
    assert!(err.to_string().contains("表名"));
}

#[tokio::test]
async fn update_row_and_delete_row() {
    let db = common::setup().await;

    let mut row = Row::new().with_entity("users", "id");
    row.insert("email", "b@example.com");
    row.insert("name", "Bob");
    row.insert("status", 1);
    row.insert("created_at", "2026-01-01T00:00:00Z");
    row.insert("updated_at", "2026-01-01T00:00:00Z");
    let inserted = db.insert_row(&row).await.expect("insert");
    let id = inserted.get("id").map(i64_value).unwrap();

    // 更新：改 name，带主键值（复用实体元数据）
    let mut row = Row::new().with_entity("users", "id");
    row.insert("id", id);
    row.insert("name", "Bobby");
    let affected = db.update_row(&row).await.expect("update");
    assert_eq!(affected, 1);

    let fresh = db
        .find_row_by_pk("users", "id", Value::I64(id))
        .await
        .expect("find")
        .unwrap();
    assert_eq!(fresh.get("name").map(str_value), Some("Bobby".into()));

    // 删除
    let affected = db.delete_row(&row).await.expect("delete");
    assert_eq!(affected, 1);
    let gone = db
        .find_row_by_pk("users", "id", Value::I64(id))
        .await
        .expect("find");
    assert!(gone.is_none());
}

#[tokio::test]
async fn execute_script_works() {
    let db = common::setup().await;

    // 多语句脚本：建临时表 + 插入，验证 execute_batch 路径
    db.execute_script(
        "CREATE TABLE script_t(id INTEGER PRIMARY KEY); \
         INSERT INTO script_t(id) VALUES (1); \
         INSERT INTO script_t(id) VALUES (2);",
    )
    .await
    .expect("execute_script");

    let cnt: i64 = db
        .scalar("SELECT COUNT(*) FROM script_t")
        .await
        .expect("count");
    assert_eq!(cnt, 2);
}

#[tokio::test]
async fn changeset_updates_only_selected_fields() {
    let db = common::setup_db().await;
    common::create_users_table(&db).await;

    let now = "2026-01-01T00:00:00Z";
    let mut user = User {
        id: 0,
        email: "u@example.com".into(),
        name: "Ursula".into(),
        status: 1,
        profile: None,
        created_at: now.parse().unwrap(),
        updated_at: now.parse().unwrap(),
    };
    user.insert_or_update(&db).await.expect("save");
    let id = user.id;
    user.status = 2;

    // 仅更新 status，name 保持不变
    let affected = User::of(id)
        .set(User::status(), user.status)
        .update(&db)
        .await
        .expect("update");
    assert_eq!(affected, 1);

    let fresh = User::find(&db, id).await.expect("find");
    assert_eq!(fresh.status, 2);
    assert_eq!(fresh.name, "Ursula");
}

//! Builder 参数化与注入防护测试。

mod common;
use alun_db::prelude::*;
use common::*;

/// Builder 查询走参数绑定，值中的 SQL 特殊字符不影响语义。
#[tokio::test]
async fn builder_filter_binds_params_safely() {
    let db = setup().await;
    db.execute_with_params(
        "INSERT INTO users (email, name, status, created_at, updated_at) VALUES (?, ?, 1, '2026-01-01', '2026-01-01')",
        vec![
            Value::Text("bob@x.com".into()),
            Value::Text("Bob'; DROP TABLE users; --".into()),
        ],
    )
    .await
    .unwrap();

    let rows = db
        .query()
        .table("users")
        .filter(Expr::Binary {
            left: Box::new(Expr::Column {
                table: "",
                column: "email",
            }),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Value(Value::Text("bob@x.com".into()))),
        })
        .exec(&db)
        .await
        .unwrap();

    assert_eq!(rows.len(), 1);
    let name: String = rows[0].get_typed("name").unwrap();
    // 恶意 payload 作为字面值完整存储，未被解释执行
    assert_eq!(name, "Bob'; DROP TABLE users; --");
    // 表未被删除，可继续查询
    assert!(!db.find("SELECT * FROM users").await.unwrap().is_empty());
}

/// 非法表名被标识符校验拒绝。
#[tokio::test]
async fn builder_rejects_invalid_table() {
    let db = setup().await;
    let result = db.query().table("users; DROP TABLE users").exec(&db).await;
    assert!(matches!(result, Err(DbError::InvalidValue(_))));
}

/// 非法排序列名被拒绝。
#[tokio::test]
async fn builder_rejects_invalid_order_column() {
    let db = setup().await;
    let result = db
        .query()
        .table("users")
        .order_by("id; DROP TABLE users", alun_db_core::statement::Order::Asc)
        .exec(&db)
        .await;
    assert!(matches!(result, Err(DbError::InvalidValue(_))));
}

/// 未设置 limit 时 first 自动追加 LIMIT 1。
#[tokio::test]
async fn builder_first_applies_limit_one() {
    let db = setup().await;
    for i in 0..5 {
        db.execute_with_params(
            "INSERT INTO users (email, name, status, created_at, updated_at) VALUES (?, ?, 1, '2026-01-01', '2026-01-01')",
            vec![
                Value::Text(format!("u{i}@x.com")),
                Value::Text(format!("User{i}")),
            ],
        )
        .await
        .unwrap();
    }

    let first = db.query().table("users").first(&db).await.unwrap();
    assert!(first.is_some());
}

/// CountBuilder 条件计数走参数绑定。
#[tokio::test]
async fn count_builder_with_filter() {
    let db = setup().await;
    for i in 0..3 {
        db.execute_with_params(
            "INSERT INTO users (email, name, status, created_at, updated_at) VALUES (?, ?, ?, '2026-01-01', '2026-01-01')",
            vec![
                Value::Text(format!("c{i}@x.com")),
                Value::Text(format!("User{i}")),
                Value::I32(i),
            ],
        )
        .await
        .unwrap();
    }

    let count = db
        .count()
        .table("users")
        .filter(Expr::Binary {
            left: Box::new(Expr::Column {
                table: "",
                column: "status",
            }),
            op: BinaryOp::Ge,
            right: Box::new(Expr::Value(Value::I32(1))),
        })
        .exec(&db)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

/// PageBuilder 分页带条件。
#[tokio::test]
async fn page_builder_with_filter() {
    let db = setup().await;
    for i in 0..7 {
        db.execute_with_params(
            "INSERT INTO users (email, name, status, created_at, updated_at) VALUES (?, ?, 1, '2026-01-01', '2026-01-01')",
            vec![
                Value::Text(format!("p{i}@x.com")),
                Value::Text(format!("User{i}")),
            ],
        )
        .await
        .unwrap();
    }

    let page = db
        .page(2, 3)
        .table("users")
        .filter(Expr::Binary {
            left: Box::new(Expr::Column {
                table: "",
                column: "status",
            }),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Value(Value::I32(1))),
        })
        .exec(&db)
        .await
        .unwrap();

    assert_eq!(page.total, 7);
    assert_eq!(page.pages, 3);
    assert_eq!(page.list.len(), 3);
}

//! 非自增主键（自然键）模型的 save 语义测试。
//!
//! String 主键（无 `#[auto]`）：首次 save 走 INSERT（含主键列），
//! 同主键再次 save 走 UPDATE，不产生重复行或唯一约束冲突。

mod common;
use alun_db::prelude::*;
use common::*;

/// String 自然键模型：主键非自增，由业务侧赋值。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("products")]
pub struct Product {
    /// 自然主键：产品编码
    #[key]
    pub code: String,
    /// 产品名称
    pub name: String,
    /// 价格（分）
    pub price: i64,
}

/// 建表：TEXT 主键，非自增。
async fn create_products_table(db: &Db) {
    db.execute(
        "CREATE TABLE products (
            code TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            price INTEGER NOT NULL
        )",
    )
    .await
    .expect("create products table");
}

#[tokio::test]
async fn natural_key_save_inserts_then_updates() {
    let db = setup_db().await;
    create_products_table(&db).await;

    // 第一次 save：主键不存在，走 INSERT（主键列必须写入）
    let mut product = Product {
        code: "P-001".into(),
        name: "keyboard".into(),
        price: 199,
    };
    product
        .insert_or_update(&db)
        .await
        .expect("first save should insert");

    let count: i64 = db
        .scalar("SELECT COUNT(*) FROM products")
        .await
        .expect("count after insert");
    assert_eq!(count, 1, "insert must write exactly one row");

    let price: i64 = db
        .scalar_with_params(
            "SELECT price FROM products WHERE code = ?",
            vec![Value::Text("P-001".into())],
        )
        .await
        .expect("select price");
    assert_eq!(price, 199);

    // 第二次 save：同主键已存在，走 UPDATE
    product.price = 299;
    product
        .insert_or_update(&db)
        .await
        .expect("second save should update");

    let count: i64 = db
        .scalar("SELECT COUNT(*) FROM products")
        .await
        .expect("count after update");
    assert_eq!(count, 1, "second save must update, not insert a duplicate");

    let price: i64 = db
        .scalar_with_params(
            "SELECT price FROM products WHERE code = ?",
            vec![Value::Text("P-001".into())],
        )
        .await
        .expect("select price after update");
    assert_eq!(price, 299);
}

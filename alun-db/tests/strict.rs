//! `#[model(strict)]` 编译期 schema 校验测试。
//!
//! 本文件能编译即证明模型列与 `tests/strict_schema.sql` 中的 `CREATE TABLE`
//! 一致（列缺失会在编译期报错）。此处再截运行期查询佐证可用。

mod common;
use alun_db::prelude::*;
use common::*;

/// 严格校验模型：字段列必须全部出现在 schema 文件对应表中。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("widgets")]
#[model(strict, schema = "tests/strict_schema.sql")]
pub struct Widget {
    /// 主键，自增
    #[key]
    #[auto]
    pub id: i64,
    /// 名称
    pub name: String,
    /// 数量
    pub qty: i64,
}

#[tokio::test]
async fn strict_model_is_usable_at_runtime() {
    let db = setup_db().await;
    db.execute(
        "CREATE TABLE widgets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            qty INTEGER NOT NULL DEFAULT 0
        )",
    )
    .await
    .expect("create widgets");

    let mut w = Widget {
        id: 0,
        name: "bolt".into(),
        qty: 5,
    };
    w.insert_or_update(&db).await.expect("insert widget");
    assert!(w.id > 0, "auto id backfilled");

    let fetched = Widget::find(&db, w.id).await.expect("find widget");
    assert_eq!(fetched, w);
}

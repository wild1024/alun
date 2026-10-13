//! `#[view]` 只读视图模型测试：视图模型可查询，但任何写操作返回
//! [`DbError::Readonly`]。

mod common;
use alun_db::prelude::*;
use common::*;

/// 只读视图模型：映射到数据库视图 `vw_notes`。
#[derive(Model, Debug, Clone, PartialEq)]
#[view("vw_notes")]
pub struct VwNote {
    /// 主键
    #[key]
    pub id: i64,
    /// 标题
    pub title: String,
}

#[tokio::test]
async fn view_is_queryable_but_rejects_writes() {
    let db = setup_db().await;
    // 底层表 + 视图
    db.execute("CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL)")
        .await
        .expect("create notes");
    db.execute("INSERT INTO notes (id, title) VALUES (1, 'a'), (2, 'b')")
        .await
        .expect("seed notes");
    db.execute("CREATE VIEW vw_notes AS SELECT id, title FROM notes")
        .await
        .expect("create view");

    // 查询可用
    let all = VwNote::query().find(&db).await.expect("view query");
    assert_eq!(all.len(), 2);
    assert_eq!(VwNote::count(&db).await.expect("view count"), 2);

    // 写操作被拒绝，且不触碰数据库
    let mut v = VwNote {
        id: 3,
        title: "c".into(),
    };
    let save_err = v
        .insert_or_update(&db)
        .await
        .expect_err("save must be rejected");
    assert!(matches!(save_err, DbError::Readonly(_)), "got {save_err:?}");

    let del_err = v.delete(&db).await.expect_err("delete must be rejected");
    assert!(matches!(del_err, DbError::Readonly(_)), "got {del_err:?}");

    let pb_err = VwNote::delete_by_pk(&db, 1)
        .await
        .expect_err("delete rejected");
    assert!(matches!(pb_err, DbError::Readonly(_)), "got {pb_err:?}");

    // 数据未被写路径改动
    assert_eq!(VwNote::count(&db).await.expect("count unchanged"), 2);
}

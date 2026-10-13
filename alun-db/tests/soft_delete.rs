//! 软删除语义测试。
//!
//! `#[soft_delete]` 标记列后：
//! - `delete` / `delete` / `delete_by` 改写为 UPDATE 置标记列（保留数据）
//! - 查询族（all / find / find_by / paginate / count）自动追加
//!   `标记列 IS NULL` 过滤，屏蔽已删除行。

mod common;
use alun_db::prelude::*;
use common::*;

/// 启用软删除的模型：`deleted_at` 为 NULL 表示未删除。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("notes")]
pub struct Note {
    /// 主键
    #[key]
    pub id: i64,
    /// 标题
    pub title: String,
    /// 软删除标记：NULL 存活，非 NULL 表示已删除时间
    #[soft_delete]
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// 建表：deleted_at 以 TEXT 存储（SQLite 序列化时间戳）。
async fn create_notes_table(db: &Db) {
    db.execute(
        "CREATE TABLE notes (
            id INTEGER PRIMARY KEY,
            title TEXT NOT NULL,
            deleted_at TEXT
        )",
    )
    .await
    .expect("create notes table");
}

fn note(id: i64, title: &str) -> Note {
    Note {
        id,
        title: title.into(),
        deleted_at: None,
    }
}

#[tokio::test]
async fn soft_delete_shields_queries_but_keeps_row() {
    let db = setup_db().await;
    create_notes_table(&db).await;

    for mut n in [note(1, "a"), note(2, "b"), note(3, "c")] {
        n.insert_or_update(&db).await.expect("insert note");
    }
    assert_eq!(Note::count(&db).await.expect("count"), 3);

    // 删除 id=2：走 UPDATE 置 deleted_at，不删除行
    note(2, "b").delete(&db).await.expect("soft delete");

    // 数据仍在表中（软删不改动行）
    let retained: i64 = db
        .scalar_with_params(
            "SELECT COUNT(*) FROM notes WHERE id = ?",
            vec![Value::I64(2)],
        )
        .await
        .expect("row retained");
    assert_eq!(retained, 1, "soft delete must keep the row");

    // 查询族自动过滤已删除行
    let all = Note::query().find(&db).await.expect("all");
    assert_eq!(all.len(), 2, "deleted row must be filtered out");
    assert!(Note::count(&db).await.expect("count") == 2);
    assert!(
        Note::find(&db, 2).await.is_err(),
        "find must NotFind deleted"
    );
    assert!(Note::find_or_none(&db, 2).await.expect("none").is_none());
    assert!(Note::find_by(&db, "id > 0", ()).await.expect("where").len() == 2);
    assert!(
        Note::find_first_by(&db, "id = 2", ())
            .await
            .expect("first")
            .is_none()
    );
    assert!(
        Note::count_by(&db, "id > 0", ())
            .await
            .expect("count where")
            == 2
    );
    let page = Note::query().paginate(&db, 1, 10).await.expect("page");
    assert_eq!(page.total, 2);
}

#[tokio::test]
async fn soft_delete_bulk_and_pk_paths() {
    let db = setup_db().await;
    create_notes_table(&db).await;

    for mut n in [note(1, "a"), note(2, "b"), note(3, "c")] {
        n.insert_or_update(&db).await.expect("insert note");
    }

    // delete 亦为软删
    Note::delete_by_pk(&db, 1).await.expect("soft delete by pk");
    let dead: i64 = db
        .scalar_with_params(
            "SELECT COUNT(*) FROM notes WHERE id = 1 AND deleted_at IS NOT NULL",
            vec![],
        )
        .await
        .expect("marker set");
    assert_eq!(dead, 1, "delete must set the marker");

    // delete_by 亦为软删
    Note::delete_by(&db, "id = 3", ())
        .await
        .expect("soft delete where");
    assert!(Note::find(&db, 3).await.is_err());

    // id=2 仍存活
    assert!(Note::find(&db, 2).await.expect("id2 survives").id == 2);
}

//! `ModelQuery` 脊柱、`ChangeSet`、异构批量与宏默认约定的集成测试。
//!
//! 这里的模型刻意**不写** `#[table]` / `#[key]` / `#[auto]` / `#[hook]`：
//! 本文件同时验证四条宏默认约定——表名蛇形推导、`id` 默认主键且整型默认自增、
//! `created_at` 自动填时间戳、`deleted_at` 自动软删。

mod common;
use common::setup_db;

use alun_db::prelude::*;
use chrono::{DateTime, Utc};

/// 全靠默认约定的模型：表名推导为 `note`，`id` 既是主键也是自增列，
/// `created_at` 触发内置 `TimestampHook`，`deleted_at` 触发软删除。
#[derive(Model, Debug, Clone, PartialEq)]
struct Note {
    id: i64,
    title: String,
    status: i32,
    created_at: DateTime<Utc>,
    deleted_at: Option<DateTime<Utc>>,
}

/// 建表：列名与字段名一致（无 `#[column]` 重命名）。
async fn create_notes(db: &Db) {
    db.execute(
        "CREATE TABLE note (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            title TEXT NOT NULL,
            status INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            deleted_at TEXT
        )",
    )
    .await
    .expect("create note table");
}

/// 插入三条记录，返回数据库里的 id 列表。
async fn seed(db: &Db) -> Vec<i64> {
    let mut ids = Vec::new();
    for (title, status) in [("n1", 1), ("n2", 2), ("n3", 1)] {
        let mut note = Note {
            id: 0,
            title: title.into(),
            status,
            created_at: Utc::now(),
            deleted_at: None,
        };
        note.insert(db).await.expect("seed note");
        ids.push(note.id);
    }
    ids
}

async fn setup() -> Db {
    let db = setup_db().await;
    create_notes(&db).await;
    db
}

#[tokio::test]
async fn macro_defaults_derive_table_key_auto_and_timestamp_hook() {
    // 表名由结构体名推导；主键与软删列也是推导出来的
    assert_eq!(Note::TABLE, "note");
    assert_eq!(Note::PRIMARY_KEY, "id");
    assert_eq!(Note::SOFT_DELETE, Some("deleted_at"));
    assert!(
        Note::fields()
            .iter()
            .find(|f| f.is_key)
            .expect("key")
            .is_auto
    );

    let db = setup().await;
    let sentinel: DateTime<Utc> = "2000-01-01T00:00:00Z".parse().unwrap();
    let mut note = Note {
        id: 0,
        title: "a".into(),
        status: 1,
        created_at: sentinel,
        deleted_at: None,
    };
    note.insert(&db).await.expect("insert");

    assert!(note.id > 0, "id 默认自增并回填到模型");
    assert!(
        note.created_at > sentinel,
        "含 created_at 的模型自动挂载 TimestampHook，写入时覆盖为当前时间"
    );
    let stored = Note::find(&db, note.id).await.expect("find");
    assert_eq!(stored.created_at, note.created_at, "回填值与落库值一致");
}

#[tokio::test]
async fn soft_delete_is_automatic() {
    let db = setup().await;
    let ids = seed(&db).await;

    Note::delete_by_pk(&db, ids[0]).await.expect("soft delete");

    // 查询侧自动过滤（`query()` 与静态糖）
    assert!(Note::find_or_none(&db, ids[0]).await.unwrap().is_none());
    assert_eq!(Note::count(&db).await.unwrap(), 2);
    assert_eq!(Note::query().find(&db).await.unwrap().len(), 2);
    // 原始 SQL 不做软删过滤（列名与语义由调用方负责）
    let physical: i64 = sql!("SELECT COUNT(*) FROM note")
        .scalar::<i64, _>(&db)
        .await
        .unwrap();
    assert_eq!(physical, 3, "软删是标记而非物理删除");

    // ChangeSet 删除同样走标记
    Note::of(ids[1])
        .delete(&db)
        .await
        .expect("soft delete via changeset");
    assert_eq!(Note::count(&db).await.unwrap(), 1);
}

#[tokio::test]
async fn where_or_field_signature_accepts_field_or_fragment() {
    let db = setup().await;
    let ids = seed(&db).await;

    // 单参数 + 合法标识符 → 「字段 = ?」
    let rows = Note::find_by(&db, "status", (2,)).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "n2");
    assert_eq!(Note::count_by(&db, "status", (2,)).await.unwrap(), 1);
    assert!(
        Note::find_first_by(&db, "title", ("n1",))
            .await
            .unwrap()
            .is_some()
    );

    // 多参数 / 含语法 → 整段按 WHERE 片段处理
    let rows = Note::find_by(&db, "status = ? OR status = ?", (1, 2))
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    let rows = Note::find_by(&db, "id = ? ORDER BY id DESC", (ids[0],))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);

    // 条件删除
    assert_eq!(Note::delete_by(&db, "status", (2,)).await.unwrap(), 1);
    assert_eq!(Note::count(&db).await.unwrap(), 2);
}

#[tokio::test]
async fn in_queries_and_batch_delete() {
    let db = setup().await;
    let ids = seed(&db).await;

    assert_eq!(
        Note::find_in(&db, Note::status(), [1, 2])
            .await
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        Note::find_in(&db, Note::status(), Vec::<i32>::new())
            .await
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        Note::find_in_ids(&db, ids[..2].to_vec())
            .await
            .unwrap()
            .len(),
        2
    );

    assert_eq!(Note::delete_in(&db, Note::status(), [2]).await.unwrap(), 1);
    assert_eq!(
        Note::delete_in_ids(&db, ids[..1].to_vec()).await.unwrap(),
        1
    );
    assert_eq!(Note::count(&db).await.unwrap(), 1);
    assert_eq!(
        Note::delete_in(&db, Note::status(), Vec::<i32>::new())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn model_query_terminals() {
    let db = setup().await;
    seed(&db).await;

    assert_eq!(Note::query().find(&db).await.unwrap().len(), 3);
    assert_eq!(
        Note::query()
            .filter(Note::status().eq(1))
            .find(&db)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(
        Note::query()
            .filter(Note::status().eq(1))
            .exists(&db)
            .await
            .unwrap()
    );
    assert!(
        !Note::query()
            .filter(Note::status().eq(9))
            .exists(&db)
            .await
            .unwrap()
    );

    // find_first：无匹配返回 None
    assert!(
        Note::query()
            .filter(Note::status().eq(9))
            .find_first(&db)
            .await
            .unwrap()
            .is_none()
    );

    // find_one：恰好一行；多行/零行都是错误
    let one = Note::query()
        .filter(Note::title().eq("n2"))
        .find_one(&db)
        .await
        .unwrap();
    assert_eq!(one.title, "n2");
    assert!(matches!(
        Note::query()
            .filter(Note::status().eq(1))
            .find_one(&db)
            .await,
        Err(DbError::InvalidValue(_)),
    ));
    assert!(matches!(
        Note::query()
            .filter(Note::status().eq(9))
            .find_one(&db)
            .await,
        Err(DbError::NotFound),
    ));

    // find_one_or_null：0 或 1 行
    assert!(
        Note::query()
            .filter(Note::status().eq(9))
            .find_one_or_null(&db)
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        Note::query()
            .filter(Note::status().eq(1))
            .find_one_or_null(&db)
            .await,
        Err(DbError::InvalidValue(_)),
    ));

    // 分页：total / pages 由计数得出，顺序受 order_by 控制
    let page = Note::query()
        .order_by(Note::id().asc())
        .paginate(&db, 2, 2)
        .await
        .unwrap();
    assert_eq!(
        (page.total, page.pages, page.page, page.list.len()),
        (3, 2, 2, 1)
    );
    assert_eq!(page.list[0].title, "n3");
}

#[tokio::test]
async fn for_each_and_for_each_page_iterate() {
    let db = setup().await;
    seed(&db).await;

    // for_each：逐行；返回 Ok(false) 提前结束
    let mut titles = Vec::new();
    Note::query()
        .order_by(Note::id().asc())
        .for_each(&db, |n| {
            titles.push(n.title);
            Ok(true)
        })
        .await
        .unwrap();
    assert_eq!(titles, vec!["n1", "n2", "n3"]);

    let mut visited = 0;
    Note::query()
        .for_each(&db, |_| {
            visited += 1;
            Ok(false)
        })
        .await
        .unwrap();
    assert_eq!(visited, 1, "回调返回 Ok(false) 立即终止遍历");

    // for_each_page：按页取，内存受 size 约束（不预读 total）
    let mut pages = Vec::new();
    Note::query()
        .order_by(Note::id().asc())
        .for_each_page(&db, 2, |page| {
            assert_eq!(page.total, 0, "分页遍历不预读总数");
            pages.push((page.page, page.list.len()));
            Ok(true)
        })
        .await
        .unwrap();
    assert_eq!(pages, vec![(1, 2), (2, 1)]);
}

#[tokio::test]
async fn raw_sql_query_rejects_chain_modifiers() {
    let db = setup().await;
    seed(&db).await;

    // 原始 SQL 脊柱：不错，按 `Self` 映射
    let found = Note::sql(sql!("SELECT * FROM note WHERE title = ", { "n1" }))
        .find(&db)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);

    // 链式修饰在原始 SQL 上不可用（写进 sql! 即可），终态如实报错
    let err = Note::sql(sql!("SELECT * FROM note"))
        .filter(Note::status().eq(1))
        .find(&db)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("filter"), "got {err}");
}

#[tokio::test]
async fn change_set_updates_only_set_columns() {
    let db = setup().await;
    let ids = seed(&db).await;

    let affected = Note::of(ids[0])
        .set(Note::title(), "changed")
        .update(&db)
        .await
        .unwrap();
    assert_eq!(affected, 1);
    let after = Note::find(&db, ids[0]).await.unwrap();
    assert_eq!(after.title, "changed");
    assert_eq!(after.status, 1, "未 set 的列保持数据库原值");

    // 空变更集 / 变更主键都被拒绝
    assert!(matches!(
        Note::of(ids[0]).update(&db).await,
        Err(DbError::InvalidValue(_)),
    ));
    assert!(matches!(
        Note::of(ids[0]).set(Note::id(), 9).update(&db).await,
        Err(DbError::InvalidValue(_)),
    ));
    // 同一列多次 set 保留最后一次
    Note::of(ids[0])
        .set(Note::title(), "a")
        .set(Note::title(), "b")
        .update(&db)
        .await
        .unwrap();
    assert_eq!(Note::find(&db, ids[0]).await.unwrap().title, "b");
}

#[tokio::test]
async fn insert_or_update_handles_explicit_primary_key() {
    let db = setup().await;

    let mut note = Note {
        id: 77,
        title: "explicit".into(),
        status: 1,
        created_at: Utc::now(),
        deleted_at: None,
    };
    // 显式主键的首次写入必须是 INSERT（不能被误判成 UPDATE）
    note.insert_or_update(&db)
        .await
        .expect("insert with explicit pk");
    assert_eq!(Note::find(&db, 77).await.unwrap().title, "explicit");

    // 再次调用 → 已存在 → UPDATE
    note.title = "updated".into();
    note.insert_or_update(&db).await.expect("update existing");
    assert_eq!(Note::find(&db, 77).await.unwrap().title, "updated");
}

#[tokio::test]
async fn insert_many_writes_homogeneous_rows() {
    let db = setup().await;
    let rows: Vec<Note> = (0..3)
        .map(|i| Note {
            id: 0,
            title: format!("bulk{i}"),
            status: 1,
            created_at: Utc::now(),
            deleted_at: None,
        })
        .collect();

    Note::insert_many(&rows, &db).await.expect("insert_many");
    assert_eq!(Note::count(&db).await.unwrap(), 3);
}

#[tokio::test]
async fn batch_writes_heterogeneous_rows_and_rolls_back_on_failure() {
    let db = setup_db().await;
    db.execute(
        "CREATE TABLE scratch (id INTEGER PRIMARY KEY AUTOINCREMENT, a TEXT, b TEXT, c TEXT)",
    )
    .await
    .unwrap();

    // 异构：两行列集不同 → 两条语句 → 整批包在一个事务里
    let mut r1 = Row::new().with_entity("scratch", "id");
    r1.insert("a", "1");
    let mut r2 = Row::new().with_entity("scratch", "id");
    r2.insert("a", "2");
    r2.insert("b", "x");
    assert_eq!(db.batch_insert("scratch", &[r1, r2]).await.unwrap(), 2);

    // 列集相同的行合并为一条多行 VALUES
    let mut r3 = Row::new().with_entity("scratch", "id");
    r3.insert("c", "3");
    let mut r4 = Row::new().with_entity("scratch", "id");
    r4.insert("c", "4");
    assert_eq!(db.batch_insert("scratch", &[r3, r4]).await.unwrap(), 2);

    // 批量更新：每行按自身主键定位，只写该行携带的非主键列
    let mut u1 = Row::new().with_entity("scratch", "id");
    u1.insert("id", 1i64);
    u1.insert("c", "patched");
    let mut u2 = Row::new().with_entity("scratch", "id");
    u2.insert("id", 2i64);
    u2.insert("b", "patched");
    assert_eq!(db.batch_update("scratch", &[u1, u2]).await.unwrap(), 2);
    let rows = sql!("SELECT id, a, b, c FROM scratch ORDER BY id")
        .find(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].get_typed::<String>("c").unwrap(), "patched");
    assert_eq!(rows[1].get_typed::<String>("b").unwrap(), "patched");
    assert_eq!(
        rows[0].get_typed::<String>("a").unwrap(),
        "1",
        "未携带的列不被覆盖"
    );

    // 原子性：第二条语句冲突 → 整批回滚，第一条语句的行不落库
    let mut ok = Row::new().with_entity("scratch", "id");
    ok.insert("a", "rollback-me");
    let mut conflict = Row::new().with_entity("scratch", "id");
    conflict.insert("id", 1i64);
    conflict.insert("b", "dup");
    let err = db.batch_insert("scratch", &[ok, conflict]).await;
    assert!(err.is_err(), "主键冲突应使整批失败");
    let rows = sql!("SELECT id FROM scratch WHERE a = ", { "rollback-me" })
        .find(&db)
        .await
        .unwrap();
    assert!(rows.is_empty(), "同批中先成功的语句必须一并回滚");
}

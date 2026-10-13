//! 批量插入（`PersistExt::insert_many`）的方言参数上限分块测试。
//!
//! 覆盖缺陷 D005：整批拼单条 `INSERT` 会撞上方言占位符上限
//! （SQLite `too many SQL variables`）。修复后按上限自动分块，并把多块包在
//! 一个事务里，保证「一次调用 = 全部行生效或全部不生效」。

mod common;

use alun_db::prelude::*;
use alun_db_core::error::DbError;

/// 10 列的批插入模型（与真实业务表同宽，10 列 × 5000 行 = 50000 个占位符）。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("batch_rows")]
pub struct BatchRow {
    #[key]
    pub id: i64,
    pub c1: i64,
    pub c2: i64,
    pub c3: String,
    pub c4: String,
    pub c5: String,
    pub c6: String,
    pub c7: String,
    pub c8: String,
    pub c9: String,
}

/// 建表：`c1` 上带唯一约束，用于构造「后续分块失败」的场景。
async fn create_batch_table(db: &Db) {
    db.execute(
        "CREATE TABLE batch_rows (
            id INTEGER NOT NULL PRIMARY KEY,
            c1 INTEGER NOT NULL UNIQUE,
            c2 INTEGER NOT NULL,
            c3 TEXT NOT NULL,
            c4 TEXT NOT NULL,
            c5 TEXT NOT NULL,
            c6 TEXT NOT NULL,
            c7 TEXT NOT NULL,
            c8 TEXT NOT NULL,
            c9 TEXT NOT NULL
        )",
    )
    .await
    .expect("create batch_rows");
}

/// 构造一行（`c1` 与主键都取 `n`，便于制造唯一约束冲突）。
fn row(n: i64) -> BatchRow {
    BatchRow {
        id: n,
        c1: n,
        c2: n * 2,
        c3: format!("c3-{n}"),
        c4: format!("c4-{n}"),
        c5: format!("c5-{n}"),
        c6: format!("c6-{n}"),
        c7: format!("c7-{n}"),
        c8: format!("c8-{n}"),
        c9: format!("c9-{n}"),
    }
}

async fn count_rows(db: &Db) -> i64 {
    db.scalar("SELECT COUNT(*) FROM batch_rows")
        .await
        .expect("count")
}

#[tokio::test]
async fn wide_batch_beyond_param_limit_is_chunked() {
    let db = common::setup_db().await;
    create_batch_table(&db).await;

    // 5000 行 × 10 列 = 50000 个占位符，SQLite 上限 999 → 必须自动分块
    let rows: Vec<BatchRow> = (0..5_000_i64).map(row).collect();
    BatchRow::insert_many(&rows, &db)
        .await
        .expect("分块后应整体成功");

    assert_eq!(count_rows(&db).await, 5_000, "受影响行数应等于输入行数");

    // 首/尾行都可读回（分块边界不丢行、不重复）
    let head = BatchRow::find(&db, 0_i64).await.expect("first row");
    let tail = BatchRow::find(&db, 4_999_i64).await.expect("last row");
    assert_eq!(head.c9, "c9-0");
    assert_eq!(tail.c9, "c9-4999");
}

#[tokio::test]
async fn small_batch_still_uses_a_single_statement() {
    let db = common::setup_db().await;
    create_batch_table(&db).await;

    // 未超上限：单条语句路径（不额外开事务）
    let rows: Vec<BatchRow> = (0..10_i64).map(row).collect();
    BatchRow::insert_many(&rows, &db).await.expect("insert");
    assert_eq!(count_rows(&db).await, 10);

    // 空批是 no-op
    BatchRow::insert_many(&[], &db).await.expect("empty batch");
    assert_eq!(count_rows(&db).await, 10);
}

#[tokio::test]
async fn failing_chunk_rolls_back_the_whole_batch() {
    let db = common::setup_db().await;
    create_batch_table(&db).await;

    // 2000 行 × 10 列 → 多块；最后一块的末行与前一块的行撞唯一约束
    let mut rows: Vec<BatchRow> = (0..2_000_i64).map(row).collect();
    rows[1_999].c1 = 0; // 与第一行 (id=0, c1=0) 冲突

    let err = BatchRow::insert_many(&rows, &db)
        .await
        .expect_err("唯一约束冲突必须失败");
    assert!(
        matches!(err, DbError::UniqueViolation { .. }),
        "got {err:?}"
    );

    // 整个批次原子回滚：前面已经写入的块也不得留下
    assert_eq!(count_rows(&db).await, 0, "失败的分块批次必须整体回滚");

    // 失败后表仍可用
    let single: Vec<BatchRow> = (0..10_i64).map(row).collect();
    BatchRow::insert_many(&single, &db)
        .await
        .expect("table usable");
    assert_eq!(count_rows(&db).await, 10);
}

#[tokio::test]
async fn batch_preserves_every_column_value() {
    let db = common::setup_db().await;
    create_batch_table(&db).await;

    let rows: Vec<BatchRow> = (0..1_200_i64).map(row).collect();
    BatchRow::insert_many(&rows, &db).await.expect("insert");

    // 抽查若干行：所有列都按输入落库（分块不改变列与参数的对应关系）
    for n in [0_i64, 99, 100, 1_198, 1_199] {
        let fetched = BatchRow::find(&db, n).await.expect("row");
        assert_eq!(fetched, rows[n as usize], "row {n} 应与输入一致");
    }
    let miscounted: i64 = db
        .scalar("SELECT COUNT(*) FROM batch_rows WHERE c2 = id * 2")
        .await
        .expect("count");
    assert_eq!(miscounted, 1_200);
}

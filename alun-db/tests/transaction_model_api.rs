//! D003（A 部分）回归：**Model API 在事务内可用**。
//!
//! 修复前 `CrudExt` / `PersistExt` 的执行器参数写死 `db: &Db`，事务内只能写 raw SQL，
//! 于是每个需要原子性的服务都要自带「SQL + 手工行→模型映射」。现在参数为
//! `&impl SqlExecutor`（[`Db`] 与 [`Transaction`] 都实现该 trait），事务内的
//! `insert_or_update` / `find` / `ChangeSet` / `delete` / `count` / `paginate` /
//! `insert_many` 与事务外语义一致，包括：
//!
//! - 自增主键回填（`insert_returning_id`）；
//! - Hook 触发（Hook 仍是**同步**的，见 D003-B 的设计边界）；
//! - 多块批量写入在**外层事务**内保持原子性（不再另开嵌套事务）。

mod common;

use alun_db::prelude::*;
use alun_db_core::row::Row;

/// 探针模型：自增主键 + 可空列 + 一个同步 Hook。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("tx_probe")]
#[hook(TxStampHook)]
struct TxProbe {
    #[key]
    #[auto]
    id: i64,
    name: String,
    amount: i64,
    note: Option<String>,
}

/// 同步 Hook：写入前打标，用来证明「事务内 Hook 仍会触发」。
struct TxStampHook;

impl Hook for TxStampHook {
    fn before_insert(&self, row: &mut Row) -> alun_db_core::error::Result<()> {
        row.insert("note", Value::Text("hooked".into()));
        Ok(())
    }

    fn before_update(&self, row: &mut Row) -> alun_db_core::error::Result<()> {
        if row.contains("note") {
            row.insert("note", Value::Text("updated".into()));
        }
        Ok(())
    }
}

async fn setup() -> Db {
    let db = common::setup_db().await;
    db.execute(
        "CREATE TABLE tx_probe (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            amount BIGINT NOT NULL,
            note TEXT
        )",
    )
    .await
    .expect("create tx_probe");
    db
}

fn probe(id: i64, name: &str, amount: i64) -> TxProbe {
    TxProbe {
        id,
        name: name.to_string(),
        amount,
        note: None,
    }
}

/// 事务内 `save` + `find` + `count`，提交后对外可见；自增主键在事务内即回填。
#[tokio::test]
async fn model_api_inside_transaction_commits() {
    let db = setup().await;

    db.in_tx(|ex| {
        Box::pin(async move {
            let mut row = probe(0, "alpha", 10);
            row.insert_or_update(ex).await?;
            assert!(row.id > 0, "事务内必须回填自增主键，实际 id = {}", row.id);
            assert_eq!(row.note.as_deref(), Some("hooked"), "Hook 应在事务内触发");

            // 事务内可见未提交数据
            let loaded = TxProbe::find(ex, row.id).await?;
            assert_eq!(loaded, row);
            assert_eq!(TxProbe::count(ex).await?, 1);
            assert_eq!(TxProbe::count_by(ex, "amount > ?", (5i64,)).await?, 1);
            // 分页原语同样可用（默认实现走 count + query）
            let page = TxProbe::query().paginate(ex, 1, 10).await?;
            assert_eq!((page.total, page.list.len()), (1, 1));
            Ok(())
        })
    })
    .await
    .expect("事务提交");

    // 提交后事务外可见
    assert_eq!(TxProbe::count(&db).await.unwrap(), 1);
    let committed = TxProbe::query().find(&db).await.unwrap();
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0].note.as_deref(), Some("hooked"));
}

/// 事务内写入后返回 `Err` → 回滚：数据与自增序列效果都不落库。
#[tokio::test]
async fn model_api_inside_transaction_rolls_back() {
    let db = setup().await;

    let outcome: alun_db_core::error::Result<()> = db
        .in_tx(|ex| {
            Box::pin(async move {
                let mut row = probe(0, "beta", 20);
                row.insert_or_update(ex).await?;
                assert!(row.id > 0);
                assert_eq!(TxProbe::count(ex).await?, 1, "事务内可见");
                Err(DbError::InvalidValue("boom".into()))
            })
        })
        .await;
    assert!(outcome.is_err(), "闭包返回 Err 时 in_tx 必须回滚");

    assert_eq!(TxProbe::count(&db).await.unwrap(), 0, "回滚后不得残留行");
    assert!(
        TxProbe::find_or_none(&db, 1).await.unwrap().is_none(),
        "回滚后按主键查不到"
    );
}

/// 事务内变更集 / `delete` / `update` 语义与事务外一致。
#[tokio::test]
async fn model_api_partial_update_and_delete_inside_transaction() {
    let db = setup().await;

    let mut row = probe(0, "gamma", 30);
    row.insert_or_update(&db).await.unwrap();

    db.in_tx(|ex| {
        Box::pin(async move {
            let changed = TxProbe::find(ex, row.id).await?;
            assert_eq!(
                TxProbe::of(changed.id)
                    .set(TxProbe::amount(), 99)
                    .update(ex)
                    .await?,
                1
            );

            let after = TxProbe::find(ex, row.id).await?;
            assert_eq!(after.amount, 99, "部分更新生效");
            assert_eq!(after.note.as_deref(), Some("hooked"), "未列出的列保持不变");

            // 整行更新：Hook 收到完整行
            let mut full = after.clone();
            full.name = "gamma-2".into();
            full.update(ex).await?;
            assert_eq!(TxProbe::find(ex, row.id).await?.name, "gamma-2");

            assert_eq!(TxProbe::delete_by_pk(ex, row.id).await?, 1);
            assert_eq!(TxProbe::count(ex).await?, 0);
            Ok(())
        })
    })
    .await
    .expect("提交");

    assert_eq!(TxProbe::count(&db).await.unwrap(), 0, "删除已提交");
}

/// 事务内删除后回滚 → 行仍在（删除也受外层事务保护）。
#[tokio::test]
async fn delete_inside_transaction_is_rolled_back() {
    let db = setup().await;
    let mut row = probe(0, "delta", 40);
    row.insert_or_update(&db).await.unwrap();

    let outcome: alun_db_core::error::Result<()> = db
        .in_tx(|ex| {
            Box::pin(async move {
                assert_eq!(TxProbe::delete_by_pk(ex, row.id).await?, 1);
                Err(DbError::InvalidValue("rollback delete".into()))
            })
        })
        .await;
    assert!(outcome.is_err());
    assert_eq!(TxProbe::count(&db).await.unwrap(), 1, "回滚后行必须仍在");
}

/// **多块**批量写入在外层事务内保持原子性（`in_tx` 在事务中内联执行，
/// 不再另开嵌套事务）；回滚时整批消失。
#[tokio::test]
async fn chunked_insert_many_inside_transaction_is_atomic() {
    let db = setup().await;

    // 4 列 × 600 行 → 超过 SQLite 占位符上限，触发分块（多块）
    let rows: Vec<TxProbe> = (0..600).map(|i| probe(0, &format!("r{i}"), i)).collect();

    let outcome: alun_db_core::error::Result<()> = db
        .in_tx(|ex| {
            Box::pin(async move {
                TxProbe::insert_many(&rows, ex).await?;
                assert_eq!(TxProbe::count(ex).await?, 600, "分块写入行数完整");
                Err(DbError::InvalidValue("rollback batch".into()))
            })
        })
        .await;
    assert!(outcome.is_err());
    assert_eq!(
        TxProbe::count(&db).await.unwrap(),
        0,
        "外层事务回滚后分块写入必须整批消失"
    );
}

/// 事务外 `&Db` 调用点零改动：同一个方法在事务外仍按旧方式工作
/// （兼容性回归：参数由 `&Db` 放宽为 `&impl SqlExecutor`）。
#[tokio::test]
async fn db_callers_are_unchanged() {
    let db = setup().await;

    let mut row = probe(0, "epsilon", 50);
    row.insert_or_update(&db).await.unwrap();
    assert!(row.id > 0);
    assert_eq!(TxProbe::count(&db).await.unwrap(), 1);
    assert_eq!(TxProbe::find(&db, row.id).await.unwrap(), row);
    assert_eq!(TxProbe::delete_by_pk(&db, row.id).await.unwrap(), 1);
    assert_eq!(TxProbe::count(&db).await.unwrap(), 0);
}

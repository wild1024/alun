//! `AsyncHook` 的事务感知语义测试。
//!
//! 核心断言：异步副作用**绑定事务提交边界**——
//!
//! | 场景 | 期望 |
//! |---|---|
//! | 自动提交（`&Db`） | 语句执行后**立即**触发 |
//! | 显式事务 + `commit()` | 事务内**不触发**；提交成功后触发 |
//! | 显式事务 + `rollback()` | **绝不触发** |
//!
//! 本文件只有一个测试函数：Hook 触发记录是全局静态，单个函数内串行执行可避免
//! 测试并行造成的相互干扰。

mod common;
use alun_db::prelude::*;
use common::setup_db;
use std::sync::Mutex;

/// 全局触发记录。仅本文件使用，且全部断言在同一测试函数内串行完成。
static RECORD: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn push(entry: String) {
    RECORD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(entry);
}

/// 取出并清空记录。
fn take_record() -> Vec<String> {
    std::mem::take(
        &mut *RECORD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

/// 把每次触发写入全局记录，供断言校验**触发时机与顺序**。
struct RecordHook;

#[alun_db_core::async_trait]
impl AsyncHook for RecordHook {
    async fn after_insert(&self, row: &Row) -> Result<()> {
        let id: i64 = row.get_typed("id").unwrap_or(0);
        push(format!("insert:{id}"));
        Ok(())
    }

    async fn after_update(&self, row: &Row) -> Result<()> {
        let id: i64 = row.get_typed("id").unwrap_or(0);
        push(format!("update:{id}"));
        Ok(())
    }

    async fn after_delete(&self, pk: &Value) -> Result<()> {
        let id = match pk {
            Value::I64(v) => *v,
            _ => 0,
        };
        push(format!("delete:{id}"));
        Ok(())
    }
}

/// 测试模型：无 `created_at` / `updated_at`，避免自动挂载的 `TimestampHook` 干扰。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("async_items")]
#[hook_async(RecordHook)]
pub struct AsyncItem {
    /// 主键
    #[key]
    pub id: i64,
    /// 名称
    pub name: String,
}

#[tokio::test]
async fn async_hook_is_transaction_aware() {
    let db = setup_db().await;
    db.execute("CREATE TABLE async_items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .await
        .expect("create async_items table");

    // ── 1. 自动提交：副作用紧随语句执行 ─────────────────────────────
    let mut item = AsyncItem {
        id: 1,
        name: "a".into(),
    };
    item.insert(&db).await.expect("insert under autocommit");
    assert_eq!(
        take_record(),
        vec!["insert:1"],
        "自动提交（&Db）下异步 Hook 应立即触发"
    );

    // ── 2. 显式事务 + 提交：事务内不触发，提交后才触发 ────────────────
    db.transaction(|tx| async move {
        let mut it = AsyncItem {
            id: 2,
            name: "b".into(),
        };
        it.insert(&tx).await?;
        // 关键断言：语句已执行，但事务未提交 → 副作用必须尚未触发
        assert!(
            take_record().is_empty(),
            "事务提交前不得触发异步 Hook（否则会出现「审计已写、业务回滚」）"
        );
        Ok(())
    })
    .await
    .expect("transaction commit");
    assert_eq!(
        take_record(),
        vec!["insert:2"],
        "提交成功后应触发异步 Hook"
    );

    // ── 3. 显式事务 + 回滚：绝不触发 ────────────────────────────────
    let outcome: Result<()> = db
        .transaction(|tx| async move {
            let mut it = AsyncItem {
                id: 3,
                name: "c".into(),
            };
            it.insert(&tx).await?;
            Err(DbError::InvalidValue("deliberate rollback".into()))
        })
        .await;
    assert!(outcome.is_err(), "闭包返回 Err 应导致回滚");
    assert!(
        take_record().is_empty(),
        "回滚必须丢弃异步 Hook，不得触发"
    );

    // ── 4. update / delete 同样绑定提交边界，且保持调用顺序 ───────────
    db.transaction(|tx| async move {
        AsyncItem::of(1_i64)
            .set(AsyncItem::name(), "a2".to_string())
            .update(&tx)
            .await?;
        AsyncItem::of(1_i64).delete(&tx).await?;
        assert!(
            take_record().is_empty(),
            "事务内 update / delete 的异步 Hook 同样不得提前触发"
        );
        Ok(())
    })
    .await
    .expect("transaction commit");
    assert_eq!(
        take_record(),
        vec!["update:1", "delete:1"],
        "update / delete 的异步 Hook 应在提交后按调用顺序触发"
    );

    // ── 5. 数据确实按预期落库 ───────────────────────────────────────
    let rows = AsyncItem::query().find(&db).await.expect("query");
    assert_eq!(rows.len(), 1, "回滚的 id=3 不应存在：{rows:?}");
    assert_eq!(rows[0].id, 2, "仅 id=2 应存活：{rows:?}");
    assert_eq!(rows[0].name, "b");
}

/// 批量写入路径（`insert_many`）的异步 Hook。
///
/// 独占一套记录：本文件其它用例共享 `RECORD` 且会清空它，并行下会相互干扰。
static BATCH_RECORD: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// 记下每条被批量写入的行的主键。
struct BatchHook;

#[alun_db_core::async_trait]
impl AsyncHook for BatchHook {
    async fn after_insert(&self, row: &Row) -> Result<()> {
        let id: i64 = row.get_typed("id").unwrap_or(0);
        BATCH_RECORD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(format!("insert:{id}"));
        Ok(())
    }
}

#[derive(Model, Debug, Clone, PartialEq)]
#[table("batch_async_items")]
#[hook_async(BatchHook)]
pub struct BatchItem {
    /// 主键
    #[key]
    pub id: i64,
    /// 名称
    pub name: String,
}

fn take_batch_record() -> Vec<String> {
    std::mem::take(
        &mut *BATCH_RECORD
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    )
}

/// `insert_many` 必须**逐行**派发异步 Hook，且与单行路径共享事务边界语义。
///
/// 批量写入是审计的高危盲区：同步 Hook 出于性能刻意不跑（见 `persist.rs` 文档），
/// 但"什么都不触发"会让缓存失效 / 审计日志出现缺口——因此只观察不改写的
/// 异步 Hook 必须补上。
#[tokio::test]
async fn insert_many_dispatches_async_hooks_per_row() {
    let db = setup_db().await;
    db.execute("CREATE TABLE batch_async_items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .await
        .expect("create table");

    // ── 1. 自动提交：每行一次，顺序与输入一致 ───────────────────────
    let items = vec![
        BatchItem { id: 1, name: "a".into() },
        BatchItem { id: 2, name: "b".into() },
        BatchItem { id: 3, name: "c".into() },
    ];
    BatchItem::insert_many(&items, &db)
        .await
        .expect("insert_many under autocommit");
    assert_eq!(
        take_batch_record(),
        vec!["insert:1", "insert:2", "insert:3"],
        "批量写入应逐行触发异步 Hook（自动提交下立即执行）"
    );

    // ── 2. 事务内：提交前不触发，提交后按序触发 ─────────────────────
    db.transaction(|tx| async move {
        let more = vec![
            BatchItem { id: 4, name: "d".into() },
            BatchItem { id: 5, name: "e".into() },
        ];
        BatchItem::insert_many(&more, &tx).await?;
        assert!(
            take_batch_record().is_empty(),
            "事务提交前不得触发（否则回滚后副作用已发生）"
        );
        Ok(())
    })
    .await
    .expect("transaction commit");
    assert_eq!(
        take_batch_record(),
        vec!["insert:4", "insert:5"],
        "提交成功后应按行触发"
    );

    // ── 3. 回滚：一行都不触发 ───────────────────────────────────────
    let outcome: Result<()> = db
        .transaction(|tx| async move {
            let doomed = vec![BatchItem { id: 6, name: "f".into() }];
            BatchItem::insert_many(&doomed, &tx).await?;
            Err(DbError::InvalidValue("deliberate rollback".into()))
        })
        .await;
    assert!(outcome.is_err(), "闭包返回 Err 应导致回滚");
    assert!(take_batch_record().is_empty(), "回滚必须丢弃全部异步 Hook");

    let rows = BatchItem::query().find(&db).await.expect("query");
    assert_eq!(rows.len(), 5, "回滚的 id=6 不应落库：{rows:?}");
}

/// 异步 Hook 抛错**不影响**主流程（提交后失败不回滚，仅告警）。
#[tokio::test]
async fn failing_async_hook_does_not_break_write() {
    struct BoomHook;

    #[alun_db_core::async_trait]
    impl AsyncHook for BoomHook {
        async fn after_insert(&self, _row: &Row) -> Result<()> {
            Err(DbError::InvalidValue("boom".into()))
        }
    }

    #[derive(Model, Debug, Clone)]
    #[table("boom_items")]
    #[hook_async(BoomHook)]
    struct BoomItem {
        #[key]
        id: i64,
        name: String,
    }

    let db = setup_db().await;
    db.execute("CREATE TABLE boom_items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")
        .await
        .expect("create table");

    let mut it = BoomItem {
        id: 9,
        name: "x".into(),
    };
    // 写入必须成功：钩子失败不应冒泡为业务错误
    it.insert(&db).await.expect("write must succeed");

    let rows = BoomItem::query().find(&db).await.expect("query");
    assert_eq!(rows.len(), 1, "数据应已写入");
}

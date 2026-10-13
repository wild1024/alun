//! 事务封装。

use std::future::Future;
use std::sync::Arc;

use alun_db_core::error::{DbError, Result};
use alun_db_core::row::Row;
use alun_db_core::statement::{RawStmt, Statement};
use alun_db_core::stmt::Value;
use tokio::sync::Mutex;

use crate::db::Db;
use crate::executor::RawTransaction;
use crate::hook_async::PostCommitQueue;

/// 事务。
///
/// ## 连接独占
///
/// 事务从连接池借出专属连接，所有语句在该连接上执行，保证原子性。
/// **事务内必须使用 `tx` 句柄执行语句**：通过 `db` 执行的语句走其他
/// 连接，不受事务保护（共享单连接模式下甚至会等待事务终结）。
///
/// ## RAII 回滚
///
/// 忘记调用 `commit` / `rollback` 时，事务在句柄销毁（含 panic 展开）时
/// 由驱动自动回滚，无需异步等待。
///
/// ## 幂等终结
///
/// `commit` / `rollback` 可安全地多次调用：第一次调用生效，后续为无操作。
///
/// ## 使用方式
///
/// ### 方式一：闭包式（推荐）
///
/// ```ignore
/// db().transaction(|tx| async move {
///     tx.execute("...").await?;
///     Ok(())
/// }).await?;
/// ```
///
/// 闭包返回 `Ok` 时提交，返回 `Err` 时回滚。
///
/// ### 方式二：显式事务对象
///
/// ```ignore
/// let tx = db().begin().await?;
/// tx.execute("...").await?;
/// tx.commit().await?;
/// ```
pub struct Transaction {
    /// 共享的事务内核。闭包式 API 传入闭包的句柄与外层句柄共享同一内核。
    inner: Arc<TransactionInner>,
}

/// 事务内核。
///
/// `raw` 在事务终结（提交 / 回滚）后置空；销毁时若仍有驱动句柄，
/// 其 `Drop` 执行 RAII 回滚。
struct TransactionInner {
    /// 驱动事务句柄。
    raw: Mutex<Option<Box<dyn RawTransaction>>>,
    /// 事务所用连接的方言。
    dialect: &'static dyn alun_db_core::traits::Dialect,
    /// 提交后任务队列（异步 Hook）。见 [`PostCommitQueue`]。
    post_commit: PostCommitQueue,
}

impl Transaction {
    /// 构造事务。由 [`Db::begin`] 调用。
    fn new(
        raw: Box<dyn RawTransaction>,
        dialect: &'static dyn alun_db_core::traits::Dialect,
    ) -> Self {
        Transaction {
            inner: Arc::new(TransactionInner {
                raw: Mutex::new(Some(raw)),
                dialect,
                post_commit: PostCommitQueue::default(),
            }),
        }
    }

    /// 本事务的提交后任务队列。供 `SqlExecutor::post_commit_queue` 转发。
    pub(crate) fn post_commit_queue_ref(&self) -> &PostCommitQueue {
        &self.inner.post_commit
    }

    /// 复制句柄。仅 facade 内部用于闭包式事务 API。
    fn clone_handle(&self) -> Self {
        Transaction {
            inner: self.inner.clone(),
        }
    }

    /// 执行 INSERT / UPDATE / DELETE，返回受影响行数。
    pub async fn execute(&self, sql: &str) -> Result<u64> {
        self.execute_with_params(sql, vec![]).await
    }

    /// 事务所用连接的方言。
    pub fn dialect(&self) -> &'static dyn alun_db_core::traits::Dialect {
        self.inner.dialect
    }

    /// 带参数执行。
    pub async fn execute_with_params(&self, sql: &str, params: Vec<Value>) -> Result<u64> {
        self.execute_owned(sql.to_string(), params).await
    }

    /// 带参数执行（SQL 已拥有所有权，避免再克隆一份）。
    pub(crate) async fn execute_owned(&self, sql: String, params: Vec<Value>) -> Result<u64> {
        // F10：事务写路径埋点。计时**包含**等待事务内核互斥锁的时间——
        // 这是刻意选择：并发争用同一事务才是真实的用户体感耗时，
        // 只计语句本身会把"锁等待"藏起来，让 P99 看起来永远漂亮。
        #[cfg(feature = "metrics")]
        {
            crate::metrics::instrument(
                "tx_exec",
                crate::metrics::TABLE_RAW,
                self.execute_owned_inner(sql, params),
            )
            .await
        }
        #[cfg(not(feature = "metrics"))]
        {
            self.execute_owned_inner(sql, params).await
        }
    }

    async fn execute_owned_inner(&self, sql: String, params: Vec<Value>) -> Result<u64> {
        let stmt = Statement::Raw(RawStmt { sql, params });
        // 事务内核互斥：同一事务的多句柄串行执行
        let mut guard = self.inner.raw.lock().await;
        let raw = guard
            .as_mut()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        let result = raw.execute_raw(stmt).await?;
        Ok(result.rows_affected)
    }

    /// 执行 SELECT，返回所有行。
    pub async fn find(&self, sql: &str) -> Result<Vec<Row>> {
        self.find_with_params(sql, vec![]).await
    }

    /// 带参数的查询。
    pub async fn find_with_params(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Row>> {
        self.find_owned(sql.to_string(), params).await
    }

    /// 带参数的查询（SQL 已拥有所有权，避免再克隆一份）。
    pub(crate) async fn find_owned(&self, sql: String, params: Vec<Value>) -> Result<Vec<Row>> {
        // F10：事务读路径埋点（含锁等待，理由同 `execute_owned`）
        #[cfg(feature = "metrics")]
        {
            crate::metrics::instrument(
                "tx_query",
                crate::metrics::TABLE_RAW,
                self.find_owned_inner(sql, params),
            )
            .await
        }
        #[cfg(not(feature = "metrics"))]
        {
            self.find_owned_inner(sql, params).await
        }
    }

    async fn find_owned_inner(&self, sql: String, params: Vec<Value>) -> Result<Vec<Row>> {
        let stmt = Statement::Raw(RawStmt { sql, params });
        let mut guard = self.inner.raw.lock().await;
        let raw = guard
            .as_mut()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        let rows = raw.query_raw(stmt).await?;
        Ok(rows.0)
    }

    /// 提交事务。幂等：已终结时直接返回 `Ok`。
    ///
    /// 提交**成功后**统一执行排队的异步 Hook（见
    /// [`AsyncHook`](alun_db_core::traits::AsyncHook)）；钩子失败不回滚，
    /// 仅记 `warn` 日志。
    pub async fn commit(&self) -> Result<()> {
        let raw = self.take_raw().await;
        match raw {
            Some(raw) => match raw.commit_raw().await {
                Ok(()) => {
                    let executed = self.inner.post_commit.drain_and_run().await;
                    if executed > 0 {
                        tracing::debug!("事务提交后执行 {executed} 个异步 Hook");
                    }
                    Ok(())
                }
                Err(e) => {
                    // 提交失败 ⇒ 事务未生效 ⇒ 丢弃排队的副作用
                    self.inner.post_commit.clear();
                    Err(e)
                }
            },
            None => Ok(()),
        }
    }

    /// 回滚事务。幂等：已终结时直接返回 `Ok`。
    ///
    /// 排队的异步 Hook 在此**整体丢弃**——绝不触发，避免"审计已写、业务回滚"。
    pub async fn rollback(&self) -> Result<()> {
        let dropped = self.inner.post_commit.clear();
        if dropped > 0 {
            tracing::debug!("事务回滚，丢弃 {dropped} 个待执行的异步 Hook");
        }
        let raw = self.take_raw().await;
        match raw {
            Some(raw) => raw.rollback_raw().await,
            None => Ok(()),
        }
    }

    /// 取出驱动事务句柄。已终结时返回 `None`。
    async fn take_raw(&self) -> Option<Box<dyn RawTransaction>> {
        let mut guard = self.inner.raw.lock().await;
        guard.take()
    }
}

impl Db {
    /// 开始事务，返回显式事务句柄。
    ///
    /// 句柄销毁时若未提交，事务自动回滚。
    pub async fn begin(&self) -> Result<Transaction> {
        let raw = self.executor.begin_raw().await?;
        Ok(Transaction::new(raw, self.executor.dialect()))
    }

    /// 在事务中执行闭包。
    ///
    /// 闭包返回 `Ok` 时提交，返回 `Err` 时回滚。
    /// 闭包内可通过 `tx.commit()` / `tx.rollback()` 提前终结事务，
    /// 外层的提交 / 回滚调用是幂等的无操作。
    pub async fn transaction<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(Transaction) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let tx = self.begin().await?;
        // 闭包拿到共享内核的句柄；外层保留原句柄负责终结
        let handle = tx.clone_handle();
        match f(handle).await {
            Ok(value) => {
                tx.commit().await?;
                Ok(value)
            }
            Err(e) => {
                // 回滚失败不覆盖业务错误；未回滚的事务由 RAII 兜底
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }
}

//! SQLite 事务。持有从池中借出的专属连接，`Drop` 时自动回滚（如果未终结）。

use alun_db_core::error::{DbError, Result};

use crate::pool::PooledSqliteConnection;

/// SQLite 事务。
///
/// ## 连接独占
///
/// 事务全程独占一条池化连接：`BEGIN` / 语句 / `COMMIT` 都在该连接上执行，
/// 保证原子性。事务终结（提交 / 回滚 / 句柄销毁）后连接归还池。
///
/// ## RAII 回滚
///
/// 未终结的事务在 `Drop` 时同步执行 `ROLLBACK`。SQLite 的回滚是本地
/// 操作，无需异步等待。Drop 无法传播错误，回滚失败被显式忽略——
/// 连接归还时 SQLite 会丢弃残留事务状态。
///
/// ## 幂等终结
///
/// `commit` / `rollback` 可安全地多次调用：第一次生效，后续为无操作。
/// facade 的闭包式事务 API 依赖此性质。
pub struct SqlitePoolTransaction {
    /// 事务独占的池化连接。
    pooled: Option<PooledSqliteConnection>,
    /// 事务是否已终结（提交或回滚）。
    is_finished: bool,
}

impl SqlitePoolTransaction {
    /// 构造事务。由 [`crate::pool::SqlitePool::begin_transaction`] 调用，
    /// 连接已执行 `BEGIN`。
    pub(crate) fn begun(pooled: PooledSqliteConnection) -> Self {
        SqlitePoolTransaction {
            pooled: Some(pooled),
            is_finished: false,
        }
    }

    /// 临时取出事务连接。facade 在阻塞线程执行语句前后调用
    /// `take_pooled` / `restore_pooled`，避免异步上下文持有非 `Sync`
    /// 的连接引用。
    ///
    /// # 错误
    ///
    /// 事务已终结（连接已归还）时返回 [`DbError::Transaction`]。
    pub fn take_pooled(&mut self) -> Result<PooledSqliteConnection> {
        if self.is_finished {
            return Err(DbError::Transaction("transaction already finished".into()));
        }
        self.pooled
            .take()
            .ok_or_else(|| DbError::Transaction("transaction connection missing".into()))
    }

    /// 放回临时取出的连接。
    pub fn restore_pooled(&mut self, pooled: PooledSqliteConnection) {
        // 句柄独占期间连接不存在并发终结，直接放回
        self.pooled = Some(pooled);
    }

    /// 提交事务。幂等：已终结时直接返回 `Ok`。终结后连接归还池。
    pub fn commit(&mut self) -> Result<()> {
        if self.is_finished {
            return Ok(());
        }
        let pooled = self
            .pooled
            .as_ref()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        pooled.conn().commit()?;
        self.is_finished = true;
        // 显式置空归还连接
        self.pooled = None;
        Ok(())
    }

    /// 回滚事务。幂等：已终结时直接返回 `Ok`。终结后连接归还池。
    pub fn rollback(&mut self) -> Result<()> {
        if self.is_finished {
            return Ok(());
        }
        let pooled = self
            .pooled
            .as_ref()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        pooled.conn().rollback()?;
        self.is_finished = true;
        // 显式置空归还连接
        self.pooled = None;
        Ok(())
    }
}

impl Drop for SqlitePoolTransaction {
    fn drop(&mut self) {
        // 未终结的事务同步回滚；Drop 无法传播错误，显式忽略
        // （连接归还时 SQLite 丢弃残留事务状态，不会污染池）
        if !self.is_finished
            && let Some(pooled) = self.pooled.as_ref()
        {
            let _ = pooled.conn().rollback();
        }
        // pooled 随字段 Drop 归还连接
    }
}

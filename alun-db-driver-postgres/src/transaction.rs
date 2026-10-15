//! PostgreSQL 事务。持有从池中借出的专属连接。
//!
//! ## Drop 策略（用户决策）
//!
//! 异步驱动无法在 `Drop` 中等待 `ROLLBACK`。未终结的事务在句柄
//! 销毁时**丢弃连接**（不归还池）：连接销毁触发服务端断连，
//! 未提交事务由服务端自动回滚；池容量随 permit 释放恢复。
//! 代价是一次重连，换取确定性语义。
//!
//! ## 幂等终结
//!
//! `commit` / `rollback` 可安全地多次调用：第一次生效，后续为无操作。
//! facade 的闭包式事务 API 依赖此性质。

use alun_db_core::error::{DbError, Result};

use crate::pool::PooledPgConnection;

/// PostgreSQL 事务。
pub struct PostgresPoolTransaction {
    /// 事务独占的池化连接。终结（提交 / 回滚）后归还池并置 `None`。
    pooled: Option<PooledPgConnection>,
    /// 事务是否已终结（提交或回滚）。
    is_finished: bool,
}

impl PostgresPoolTransaction {
    /// 构造事务。由 [`crate::pool::PostgresPool::begin_transaction`] 调用，
    /// 连接已执行 `BEGIN`。
    pub(crate) fn begun(pooled: PooledPgConnection) -> Self {
        PostgresPoolTransaction {
            pooled: Some(pooled),
            is_finished: false,
        }
    }

    /// 事务内执行 INSERT / UPDATE / DELETE。
    ///
    /// # 错误
    ///
    /// 事务已终结或语句执行失败时返回 [`DbError`]。
    pub async fn execute_stmt(
        &self,
        stmt: &alun_db_core::statement::Statement,
    ) -> Result<alun_db_core::traits::executor::ExecuteResult> {
        let pooled = self
            .pooled
            .as_ref()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        pooled.conn().execute_stmt(stmt).await
    }

    /// 事务内执行 SELECT。
    ///
    /// # 错误
    ///
    /// 事务已终结或查询失败时返回 [`DbError`]。
    pub async fn query_stmt(
        &self,
        stmt: &alun_db_core::statement::Statement,
    ) -> Result<alun_db_core::traits::executor::Rows> {
        let pooled = self
            .pooled
            .as_ref()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        pooled.conn().query_stmt(stmt).await
    }

    /// 事务内执行整段脚本（simple query 协议，多语句一条消息发送）。
    ///
    /// `batch_execute` 走 simple query 协议：整段脚本一次发往服务端，由服务端
    /// 逐句解析执行，无需逐条 PREPARE / Bind——迁移等「大量互不相同的一次性语句」
    /// 场景的往返次数从 `O(语句数 × 2)` 降到 `O(1)`。
    ///
    /// 语句执行在当前事务内（同一连接）；任一语句失败返回错误并让事务进入
    /// aborted 状态，由调用方回滚。
    ///
    /// # 错误
    ///
    /// 事务已终结或脚本执行失败时返回 [`DbError`]。
    pub async fn execute_script(&self, sql: &str) -> Result<()> {
        let pooled = self
            .pooled
            .as_ref()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        pooled.conn().execute_script_async(sql).await
    }

    /// 提交事务。幂等：已终结时直接返回 `Ok`。
    ///
    /// # 错误
    ///
    /// `COMMIT` 执行失败时返回 [`DbError::Transaction`]；连接状态不可信，
    /// 句柄随后的 Drop 会丢弃该连接（不归还池）。
    pub async fn commit(&mut self) -> Result<()> {
        if self.is_finished {
            return Ok(());
        }
        let pooled = self
            .pooled
            .as_ref()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        pooled.conn().commit().await?;
        self.is_finished = true;
        // 置空触发归还：连接随 PooledPgConnection 的 Drop 回池
        self.pooled = None;
        Ok(())
    }

    /// 回滚事务。幂等：已终结时直接返回 `Ok`。
    ///
    /// # 错误
    ///
    /// `ROLLBACK` 执行失败时返回 [`DbError::Transaction`]；连接状态不可信，
    /// 句柄随后的 Drop 会丢弃该连接（不归还池）。
    pub async fn rollback(&mut self) -> Result<()> {
        if self.is_finished {
            return Ok(());
        }
        let pooled = self
            .pooled
            .as_ref()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        pooled.conn().rollback().await?;
        self.is_finished = true;
        // 置空触发归还：连接随 PooledPgConnection 的 Drop 回池
        self.pooled = None;
        Ok(())
    }
}

impl Drop for PostgresPoolTransaction {
    fn drop(&mut self) {
        // 未终结事务：丢弃连接（不归还池），服务端断连自动回滚；
        // permit 随之释放，池容量恢复
        if !self.is_finished
            && let Some(pooled) = self.pooled.take()
        {
            pooled.discard();
        }
        // 已终结：pooled 已置空；未置空且已终结的路径不存在
    }
}

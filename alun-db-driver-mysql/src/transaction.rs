//! MySQL 事务。持有从池中借出的专属连接。
//!
//! ## Drop 策略（用户决策）
//!
//! 异步驱动无法在 `Drop` 中等待 `ROLLBACK`。未终结的事务在句柄
//! 销毁时**丢弃连接**（不归还池）：连接销毁触发服务端断连，
//! 未提交事务由服务端自动回滚；池容量随 permit 释放恢复。
//! 代价是一次重连，换取确定性语义。
//!
//! ## 事务实现
//!
//! 不使用 mysql_async 的 `Transaction<'_>`（借用绑定连接生命周期，
//! 与池化句柄的所有权模型冲突），改为手动执行 `START TRANSACTION` /
//! `COMMIT` / `ROLLBACK`。
//!
//! ## 幂等终结
//!
//! `commit` / `rollback` 可安全地多次调用：第一次生效，后续为无操作。
//! facade 的闭包式事务 API 依赖此性质。

use alun_db_core::error::{DbError, Result};

use crate::pool::PooledMysqlConnection;

/// MySQL 事务。
pub struct MysqlPoolTransaction {
    /// 事务独占的池化连接。终结（提交 / 回滚）后归还池并置 `None`。
    pooled: Option<PooledMysqlConnection>,
    /// 事务是否已终结（提交或回滚）。
    is_finished: bool,
}

impl MysqlPoolTransaction {
    /// 构造事务。由 [`crate::pool::MysqlPool::begin_transaction`] 调用，
    /// 连接已执行 `START TRANSACTION`。
    pub(crate) fn begun(pooled: PooledMysqlConnection) -> Self {
        MysqlPoolTransaction {
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
        &mut self,
        stmt: &alun_db_core::statement::Statement,
    ) -> Result<alun_db_core::traits::executor::ExecuteResult> {
        let pooled = self
            .pooled
            .as_mut()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        pooled.conn_mut().execute_stmt(stmt).await
    }

    /// 事务内执行 SELECT。
    ///
    /// # 错误
    ///
    /// 事务已终结或查询失败时返回 [`DbError`]。
    pub async fn query_stmt(
        &mut self,
        stmt: &alun_db_core::statement::Statement,
    ) -> Result<alun_db_core::traits::executor::Rows> {
        let pooled = self
            .pooled
            .as_mut()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        pooled.conn_mut().query_stmt(stmt).await
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
            .as_mut()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        pooled.conn_mut().commit().await?;
        self.is_finished = true;
        // 置空触发归还：连接随 PooledMysqlConnection 的 Drop 回池
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
            .as_mut()
            .ok_or_else(|| DbError::Transaction("transaction already finished".into()))?;
        pooled.conn_mut().rollback().await?;
        self.is_finished = true;
        // 置空触发归还：连接随 PooledMysqlConnection 的 Drop 回池
        self.pooled = None;
        Ok(())
    }
}

impl Drop for MysqlPoolTransaction {
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

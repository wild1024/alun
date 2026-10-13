//! `Hook` trait —— Model 生命周期拦截。

use crate::error::Result;
use crate::row::Row;
use crate::stmt::Value;

/// Model 生命周期 Hook。
///
/// ## 执行层级
///
/// Hook 在 **Model 关联函数层**拦截。绕过 Model 直接写 SQL 时 Hook
/// 不触发——这是设计决策，不是缺陷：Hook 是 Model 级概念。
///
/// ## 执行顺序
///
/// 多个 Hook 按声明顺序执行：`before_*` 正序，`after_*` 逆序（洋葱模型）。
/// 任何 `before_*` 返回 `Err` 则中断操作并触发事务回滚。
///
/// ## UPDATE 收到的行（变更追踪）
///
/// 整行更新（`PersistExt::update`）与部分更新（`ChangeSet::update`）
/// 交给 Hook 的行不同：
///
/// - 整行更新：行含模型全部字段；
/// - 部分更新：行**只含主键 + 本次变更列**，`Row::keys()` 即变更集合——
///   审计 Hook 只需变更列 + 主键，无需自行 `SELECT` 再 diff。
///
/// ## 需要异步副作用时用 [`AsyncHook`]
///
/// 本 trait 全同步，且 `after_*` 在**事务内**执行。若副作用需要 IO（审计落库、
/// 缓存失效、发通知），请改用 [`AsyncHook`]：它只提供 `after_*`，并把触发时机
/// **推迟到事务提交之后**（回滚则丢弃），避免「副作用已生效、业务被回滚」。
pub trait Hook: Send + Sync + 'static {
    // ==================== INSERT ====================

    /// INSERT 前。可修改行数据。
    fn before_insert(&self, _row: &mut Row) -> Result<()> {
        Ok(())
    }
    /// INSERT 后。
    fn after_insert(&self, _row: &Row) -> Result<()> {
        Ok(())
    }

    // ==================== UPDATE ====================

    /// UPDATE 前。可修改行数据。
    fn before_update(&self, _row: &mut Row) -> Result<()> {
        Ok(())
    }
    /// UPDATE 后。
    fn after_update(&self, _row: &Row) -> Result<()> {
        Ok(())
    }

    // ==================== DELETE ====================

    /// DELETE 前。
    fn before_delete(&self, _pk: &Value) -> Result<()> {
        Ok(())
    }
    /// DELETE 后。
    fn after_delete(&self, _pk: &Value) -> Result<()> {
        Ok(())
    }
}

// ==================== 异步 Hook（事务感知） ====================

use async_trait::async_trait;

/// 异步 Hook —— 行级 CRUD **之后**的副作用。
///
/// 适用：审计落库、缓存精确失效、发站内信 / 邮件、写变更流水、同步外部系统。
///
/// # 为什么只有 `after_*`（没有 `before_*`）
///
/// `before_*` 在**事务内**执行且允许改写行数据。在其中 `await`（网络 / IO）会
/// 拉长持锁时间、显著放大死锁概率，并让「行数据已被改写但事务回滚」的时序变得
/// 不可推理。因此异步钩子**只提供 `after_*`**；`before_*` 仍由同步的 [`Hook`]
/// 承担（见 [`Hook::before_insert`]）。
///
/// # 执行时机：绑定事务提交边界
///
/// 这是本 trait 的核心语义（区别于 [`Hook::after_*`] 的「语句执行后」）：
///
/// | 执行器 | 触发时机 |
/// |---|---|
/// | `&Db`（自动提交） | 语句本身即已提交 → 紧随其后执行 |
/// | `&Transaction` | **排队**，在 `commit()` **成功后**统一执行 |
/// | `&Transaction` + `rollback()` | **丢弃**，绝不触发 |
///
/// 由此杜绝「审计已落库但业务被回滚」的经典不一致。
///
/// # 失败语义
///
/// 提交后的钩子失败**不会**回滚（事务已提交），仅记录 `warn` 日志。需要
/// 强一致的副作用请放进业务事务内（即与业务同一事务，而非提交后）。
///
/// # 声明方式
///
/// ```ignore
/// #[derive(Model)]
/// #[table("invoices")]
/// #[hook_async(AuditHook)]      // ← 与同步的 #[hook(..)] 并存
/// pub struct Invoice { /* … */ }
/// ```
///
/// # 示例
///
/// ```ignore
/// struct AuditHook;
///
/// #[async_trait::async_trait]
/// impl AsyncHook for AuditHook {
///     async fn after_insert(&self, row: &Row) -> alun_db_core::error::Result<()> {
///         audit_log::append(row).await
///     }
/// }
/// ```
#[async_trait]
pub trait AsyncHook: Send + Sync + 'static {
    /// INSERT 提交后。
    async fn after_insert(&self, _row: &Row) -> Result<()> {
        Ok(())
    }

    /// UPDATE 提交后。
    async fn after_update(&self, _row: &Row) -> Result<()> {
        Ok(())
    }

    /// DELETE 提交后（只传入主键值）。
    async fn after_delete(&self, _pk: &Value) -> Result<()> {
        Ok(())
    }
}

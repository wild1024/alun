//! 数据库 Hook：生命周期拦截（before/after 拦截 CRUD）
//!
//! 应用场景：
//! - 数据审计：自动记录谁在什么时间修改了什么
//! - 字段自动填充：created_at/updated_at
//! - 软删除：deleted_at 标记
use crate::Row;
use crate::DbResult;
use async_trait::async_trait;

/// CRUD 生命周期 Hook
///
/// 所有方法默认空实现，只需覆写关心的生命周期阶段。
///
/// # 示例
///
/// ```ignore
/// struct AuditHook;
///
/// #[async_trait]
/// impl Hook for AuditHook {
///     async fn before_insert(&self, row: &mut Row) -> DbResult<()> {
///         row.set("created_by", "system");
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Hook: Send + Sync {
    /// INSERT 之前（可修改即将插入的 Row）
    async fn before_insert(&self, row: &mut Row) -> DbResult<()> { let _ = row; Ok(()) }

    /// INSERT 之后（可读取已插入的 Row，不可修改）
    async fn after_insert(&self, row: &Row) -> DbResult<()> { let _ = row; Ok(()) }

    /// UPDATE 之前（可修改即将更新的 Row）
    async fn before_update(&self, row: &mut Row) -> DbResult<()> { let _ = row; Ok(()) }

    /// UPDATE 之后
    async fn after_update(&self, row: &Row) -> DbResult<()> { let _ = row; Ok(()) }

    /// DELETE 之前（可校验或拒绝删除）
    async fn before_delete(&self, table: &str, id: &str) -> DbResult<()> {
        let _ = (table, id); Ok(())
    }

    /// DELETE 之后
    async fn after_delete(&self, table: &str, id: &str) -> DbResult<()> {
        let _ = (table, id); Ok(())
    }
}

/// 空 Hook —— 所有生命周期方法均为空操作
///
/// 用作未配置 Hook 时的默认实现，避免 `Option<Hook>` 的额外分支。
pub struct NullHook;

#[async_trait]
impl Hook for NullHook {}

/// Hook 链 —— 多个 Hook 顺序执行
///
/// 将多个 Hook 聚合成一个，按注册顺序依次调用。
/// 若任一 Hook 返回 `Err`，后续 Hook 不再执行。
pub struct HookChain {
    /// Hook 列表
    hooks: Vec<Box<dyn Hook>>,
}

impl HookChain {
    /// 创建空的 Hook 链
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// 添加 Hook（链式调用），返回 `&mut Self` 以支持连续添加
    pub fn add<H: Hook + 'static>(&mut self, hook: H) -> &mut Self {
        self.hooks.push(Box::new(hook));
        self
    }
}

#[async_trait]
impl Hook for HookChain {
    async fn before_insert(&self, row: &mut Row) -> DbResult<()> {
        for hook in &self.hooks {
            hook.before_insert(row).await?;
        }
        Ok(())
    }

    async fn after_insert(&self, row: &Row) -> DbResult<()> {
        for hook in &self.hooks {
            hook.after_insert(row).await?;
        }
        Ok(())
    }

    async fn before_update(&self, row: &mut Row) -> DbResult<()> {
        for hook in &self.hooks {
            hook.before_update(row).await?;
        }
        Ok(())
    }

    async fn after_update(&self, row: &Row) -> DbResult<()> {
        for hook in &self.hooks {
            hook.after_update(row).await?;
        }
        Ok(())
    }

    async fn before_delete(&self, table: &str, id: &str) -> DbResult<()> {
        for hook in &self.hooks {
            hook.before_delete(table, id).await?;
        }
        Ok(())
    }

    async fn after_delete(&self, table: &str, id: &str) -> DbResult<()> {
        for hook in &self.hooks {
            hook.after_delete(table, id).await?;
        }
        Ok(())
    }
}

impl Default for HookChain {
    fn default() -> Self {
        Self::new()
    }
}

/// 自动填充时间戳的 Hook —— INSERT 时设置 `created_at` 和 `updated_at`
///
/// UPDATE 时自动刷新 `updated_at` 字段。
/// 时间格式使用 RFC3339（UTC），依赖 `chrono` crate。
pub struct TimestampHook;

#[async_trait]
impl Hook for TimestampHook {
    async fn before_insert(&self, row: &mut Row) -> DbResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        row.set("created_at", now.as_str());
        row.set("updated_at", now.as_str());
        Ok(())
    }

    async fn before_update(&self, row: &mut Row) -> DbResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        row.set("updated_at", now.as_str());
        Ok(())
    }
}

//! 内置 Hook 与洋葱模型执行器。
//!
//! Hook 由 `#[hook(Path)]` 声明在 Model 上，宏生成 `Model::hooks()`，
//! facade 层的 `PersistExt` / `ChangeSet` 在 CRUD 时按洋葱模型调用：
//! `before_*` 正序、`after_*` 逆序；任何 `before_*` 返回 `Err` 则中断操作。
//!
//! 本模块位于 core 而非 facade，因为 `#[derive(Model)]` 需要在内置 Hook
//! （[`TimestampHook`]）自动挂载时生成指向它的绝对路径。

use crate::error::{DbError, Result};
use crate::row::Row;
use crate::stmt::Value;
use crate::stmt::compact::CompactTimestampTz;
use crate::traits::hook::Hook;

// ==================== 内置 Hook ====================

/// 内置时间戳 Hook。
///
/// 接管约定列 `created_at` / `updated_at`：
///
/// - INSERT 前：两列均强制写为当前 UTC 时间
/// - UPDATE 前：`updated_at` 强制写为当前 UTC 时间
///
/// 语义对齐 Rails `timestamps` / Django `auto_now`：审计时间戳以写入时刻
/// 为准，调用方设置的值会被覆盖；需保留自定义时间戳的模型不要挂载。
/// 仅当 Row 中存在对应键时填充（模型可按需裁剪列集）。
///
/// ## 自动挂载
///
/// `#[derive(Model)]` 在结构体含 `created_at` 或 `updated_at` 字段时
/// **自动挂载**本 Hook，无需手写 `#[hook(TimestampHook)]`；显式挂载
/// 与其他 Hook 的声明顺序共同决定执行顺序（`before_*` 正序）。
///
/// 时间值通过 `std::time` 构造为 [`Value::TimestampTz`]，
/// 不依赖 chrono feature；驱动负责将其序列化为数据库可存储的格式。
pub struct TimestampHook;

impl Hook for TimestampHook {
    fn before_insert(&self, row: &mut Row) -> Result<()> {
        let now = now_value()?;
        if row.contains("created_at") {
            row.insert("created_at", now.clone());
        }
        if row.contains("updated_at") {
            row.insert("updated_at", now);
        }
        Ok(())
    }

    fn before_update(&self, row: &mut Row) -> Result<()> {
        if row.contains("updated_at") {
            row.insert("updated_at", now_value()?);
        }
        Ok(())
    }
}

/// 获取当前 UTC 时间，构造 `Value::TimestampTz`。
///
/// 使用 `std::time` 而非 chrono，保证无 chrono feature 时可用。
/// 亦被软删除改写（`delete` → `UPDATE SET deleted_at = now`）复用。
pub fn now_value() -> Result<Value> {
    let micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| DbError::InvalidValue(format!("system clock error: {e}")))?
        .as_micros() as i64;
    Ok(Value::TimestampTz(CompactTimestampTz {
        utc_micros: micros,
        offset_secs: 0,
    }))
}

// ==================== 洋葱模型执行器 ====================

/// 正序执行 `before_insert`，任一 `Err` 立即中断。
pub fn run_before_insert(hooks: &[&dyn Hook], row: &mut Row) -> Result<()> {
    for hook in hooks {
        hook.before_insert(row)?;
    }
    Ok(())
}

/// 逆序执行 `after_insert`（洋葱模型）。
pub fn run_after_insert(hooks: &[&dyn Hook], row: &Row) -> Result<()> {
    for hook in hooks.iter().rev() {
        hook.after_insert(row)?;
    }
    Ok(())
}

/// 正序执行 `before_update`，任一 `Err` 立即中断。
pub fn run_before_update(hooks: &[&dyn Hook], row: &mut Row) -> Result<()> {
    for hook in hooks {
        hook.before_update(row)?;
    }
    Ok(())
}

/// 逆序执行 `after_update`（洋葱模型）。
pub fn run_after_update(hooks: &[&dyn Hook], row: &Row) -> Result<()> {
    for hook in hooks.iter().rev() {
        hook.after_update(row)?;
    }
    Ok(())
}

/// 正序执行 `before_delete`，任一 `Err` 则拦截删除。
pub fn run_before_delete(hooks: &[&dyn Hook], pk: &Value) -> Result<()> {
    for hook in hooks {
        hook.before_delete(pk)?;
    }
    Ok(())
}

/// 逆序执行 `after_delete`（洋葱模型）。
pub fn run_after_delete(hooks: &[&dyn Hook], pk: &Value) -> Result<()> {
    for hook in hooks.iter().rev() {
        hook.after_delete(pk)?;
    }
    Ok(())
}

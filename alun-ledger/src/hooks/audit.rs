//! 审计 Hook：把「即将写入的行」快照记录到线程本地，供断言 Hook 触发时机。
//!
//! Hook trait 是**同步且无 IO** 的（`alun-db-core/src/traits/hook.rs`）——这是**设计边界**
//! 而非缺陷（D003-B 明确不修：异步 Hook 无法保证与业务变更同事务，且引入回滚归属与递归歧义）。
//! 审计落库由 [`crate::services::audit`] 在**同一事务内显式插入**完成；
//! 而事务内使用 Model API 已可用（D003-A 已修复，见 `alun-db/tests/transaction_model_api.rs`）。

use alun_db::prelude::*;
use alun_db_core::row::Row;
use std::cell::RefCell;

thread_local! {
    static TRACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// 取出并清空当前线程的审计痕迹。
pub fn drain_audit_trace() -> Vec<String> {
    TRACE.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

fn push(label: &str, row: &Row) {
    let keys: Vec<&str> = row.keys().collect();
    TRACE.with(|t| t.borrow_mut().push(format!("{label}[{}]", keys.join(","))));
}

/// 审计痕迹 Hook（只观察，不写库）。
pub struct AuditTraceHook;

impl Hook for AuditTraceHook {
    fn before_insert(&self, row: &mut Row) -> alun_db_core::error::Result<()> {
        push("before_insert", row);
        Ok(())
    }
    fn before_update(&self, row: &mut Row) -> alun_db_core::error::Result<()> {
        push("before_update", row);
        Ok(())
    }
    fn after_update(&self, row: &Row) -> alun_db_core::error::Result<()> {
        push("after_update", row);
        Ok(())
    }
}

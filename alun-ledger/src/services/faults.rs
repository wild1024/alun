//! 故障注入点（`FailPoint`）。
//!
//! 由集成测试通过 `alun_ledger::services::faults::set(...)` 打开；
//! 服务在被测步骤处调用 [`check`]，命中即返回 `DbError::Transaction`，
//! 用于验证「任何步骤失败 → 整体回滚」。
//!
//! 生产构建（未开 `failpoint` feature）时 [`check`] 恒为 `Ok`，
//! 不影响性能与语义。

#[cfg(feature = "failpoint")]
use alun_db_core::error::DbError;
use alun_db_core::error::Result;
use std::cell::Cell;

/// 注入点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FailPoint {
    /// 扣款之后、入账之前。
    AfterDebit = 1,
    /// 入账之前。
    BeforeCredit = 2,
    /// 审计写入时。
    AuditWrite = 3,
    /// 提交之前。
    Commit = 4,
}

thread_local! {
    /// 注入点按**线程**生效：并行执行的用例互不干扰
    /// （`#[tokio::test]` 默认 current-thread 运行时，同一个用例的
    /// 业务代码与注入检查在同一线程上执行）。
    static ACTIVE: Cell<u8> = const { Cell::new(0) };
}

/// 设置当前线程生效的注入点（`None` 关闭）。
pub fn set(fp: Option<FailPoint>) {
    ACTIVE.with(|c| c.set(fp.map(|f| f as u8).unwrap_or(0)));
}

/// 当前线程生效的注入点。
pub fn active() -> Option<FailPoint> {
    match ACTIVE.with(|c| c.get()) {
        1 => Some(FailPoint::AfterDebit),
        2 => Some(FailPoint::BeforeCredit),
        3 => Some(FailPoint::AuditWrite),
        4 => Some(FailPoint::Commit),
        _ => None,
    }
}

/// 命中注入点则返回事务错误。
pub fn check(fp: FailPoint) -> Result<()> {
    #[cfg(feature = "failpoint")]
    {
        if active() == Some(fp) {
            return Err(DbError::Transaction(format!("failpoint {fp:?} triggered")));
        }
    }
    #[cfg(not(feature = "failpoint"))]
    {
        let _ = fp;
    }
    Ok(())
}

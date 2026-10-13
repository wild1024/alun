//! Hook 实现与探针。
//!
//! 内建 `TimestampHook` 由模型上的 `#[hook(TimestampHook)]` 挂载；
//! 本模块提供「顺序探针」（验证洋葱模型）与「审计记录器」（供 Hook 顺序断言）。

pub mod audit;
pub mod probe;

pub use audit::{AuditTraceHook, drain_audit_trace};
pub use probe::{HookA, HookB, HookC, drain_order};

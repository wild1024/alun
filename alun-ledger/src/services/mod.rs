//! 业务服务层。
//!
//! 设计约束（来自框架真实能力，见 DEFECTS.md）：
//! - **事务内只能用 raw SQL**：`PersistExt::save(&Db)` 不接受 `&Transaction`，
//!   因此事务内的写入统一走 `tx.execute_with_params`。
//! - **金额读取走 [`money`] 适配器**：`Decimal::from_value` 只接受 `Value::Decimal`，
//!   凡方言以文本/整型承载金额的列（SQLite 必现）都要手工转换。
//! - **审计落库在服务层**：`Hook` 是同步无 IO 的 trait，无法在 Hook 内写库。

pub mod audit;
pub mod billing;
pub mod error;
pub mod faults;
pub mod invoice;
pub mod ledger;
pub mod money;
pub mod reconcile;
pub mod report;
pub mod rowx;
pub mod tenant;

pub use error::{LedgerError, map_db_err};
pub use faults::FailPoint;

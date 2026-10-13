//! 业务模型（`#[derive(Model)]`）。
//!
//! 列序刻意与迁移 DDL 一致，命中 `from_row` 的索引快路径。
//! 状态机统一用 `#[value_map]` 枚举（`Value::Text` 承载，三库读写一致）。

pub mod account;
pub mod audit_log;
pub mod invoice;
pub mod ledger_entry;
pub mod probe;
pub mod report;
pub mod subscription;
pub mod tenant;
pub mod transfer;
pub mod user;

pub use account::Account;
pub use audit_log::AuditLog;
pub use invoice::{Invoice, InvoiceItem, InvoiceStatus};
pub use ledger_entry::{Direction, LedgerEntry};
pub use probe::{AuditProbe, EnumProbe, HookProbe, IntState, MoneyProbe, TextState, TypeProbe};
pub use report::BillingReport;
pub use subscription::{Subscription, SubscriptionStatus, UsageRecord};
pub use tenant::Tenant;
pub use transfer::{Transfer, TransferStatus};
pub use user::{Profile, User};

#[cfg(test)]
mod compile_checks {
    use super::*;

    #[test]
    fn enums_are_serde_round_trip() {
        fn assert_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}
        assert_serde::<invoice::InvoiceStatus>();
        assert_serde::<transfer::TransferStatus>();
        let s = serde_json::to_string(&invoice::InvoiceStatus::Draft).unwrap();
        assert_eq!(s, "\"draft\"");
    }
}

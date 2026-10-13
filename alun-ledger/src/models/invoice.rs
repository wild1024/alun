//! 发票与条目。

use alun_db::prelude::*;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

/// 发票状态机：`Draft → Issued → Paid`，`Voided` 为终态。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
#[value_map(serde, "draft" => Draft, "issued" => Issued, "paid" => Paid, "voided" => Voided)]
pub enum InvoiceStatus {
    Draft,
    Issued,
    Paid,
    Voided,
}

impl InvoiceStatus {
    /// 允许的状态流转。
    pub fn can_transition_to(self, next: InvoiceStatus) -> bool {
        matches!(
            (self, next),
            (InvoiceStatus::Draft, InvoiceStatus::Issued)
                | (InvoiceStatus::Issued, InvoiceStatus::Paid)
                | (InvoiceStatus::Draft, InvoiceStatus::Voided)
                | (InvoiceStatus::Issued, InvoiceStatus::Voided)
        )
    }
}

/// 发票。
#[derive(Model, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[table("invoices")]
#[model(no_timestamps)] // created_at 承载业务时间（开票/记账/事件时刻），不用内置 TimestampHook 覆盖
pub struct Invoice {
    #[key]
    pub id: i64,
    pub tenant_id: i64,
    pub customer_id: i64,
    pub status: InvoiceStatus,
    pub total: Decimal,
    #[json]
    pub metadata: Option<serde_json::Value>,
    pub issued_at: Option<DateTime<Utc>>,
    pub paid_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub note: Option<String>,
}

/// 发票条目。
#[derive(Model, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[table("invoice_items")]
pub struct InvoiceItem {
    #[key]
    pub id: i64,
    pub invoice_id: i64,
    pub description: String,
    pub quantity: i64,
    pub unit_price: Decimal,
    pub amount: Decimal,
    pub tax_rate: String,
}

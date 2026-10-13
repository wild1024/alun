//! 报表只读视图。

use alun_db::prelude::*;
use rust_decimal::Decimal;

/// MRR 视图模型（`#[view("v_mrr")]` → 写操作返回 [`DbError::Readonly`]）。
#[derive(Model, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[view("v_mrr")]
pub struct BillingReport {
    #[key]
    pub tenant_id: i64,
    pub month: String,
    pub total: Decimal,
    pub status: String,
}

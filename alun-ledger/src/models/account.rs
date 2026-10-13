//! 复式记账账户。

use alun_db::prelude::*;
use rust_decimal::Decimal;

/// 账户（余额为 `Decimal`）。
#[derive(Model, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[table("accounts")]
pub struct Account {
    #[key]
    pub id: i64,
    pub tenant_id: i64,
    pub code: String,
    pub name: String,
    pub kind: String,
    pub balance: Decimal,
    pub currency: String,
}

impl Account {
    pub const KIND_ASSET: &'static str = "asset";
    pub const KIND_LIABILITY: &'static str = "liability";
    pub const KIND_EQUITY: &'static str = "equity";
    pub const KIND_REVENUE: &'static str = "revenue";
    pub const KIND_EXPENSE: &'static str = "expense";
}

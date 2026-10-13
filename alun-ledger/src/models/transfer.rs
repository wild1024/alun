//! 转账与账务分录。

use alun_db::prelude::*;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

/// 转账状态机。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
#[value_map(serde, "posted" => Posted, "rejected" => Rejected)]
pub enum TransferStatus {
    Posted,
    Rejected,
}

/// 转账单。
#[derive(Model, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[table("transfers")]
#[model(no_timestamps)] // created_at 承载业务时间（开票/记账/事件时刻），不用内置 TimestampHook 覆盖
pub struct Transfer {
    #[key]
    pub id: i64,
    pub tenant_id: i64,
    pub from_account: i64,
    pub to_account: i64,
    pub amount: Decimal,
    pub status: TransferStatus,
    pub created_at: DateTime<Utc>,
}

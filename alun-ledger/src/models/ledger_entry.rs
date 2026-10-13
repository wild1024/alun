//! 账务分录。

use alun_db::prelude::*;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

/// 分录方向（借贷）。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
#[value_map(serde, "debit" => Debit, "credit" => Credit)]
pub enum Direction {
    Debit,
    Credit,
}

impl Direction {
    /// 余额变动的符号：借增贷减（本项目账户均为资产/负债科目，符号由科目类型决定，
    /// 这里对分录金额做统一符号约定：debit = +1，credit = -1）。
    pub fn sign(self) -> i32 {
        match self {
            Direction::Debit => 1,
            Direction::Credit => -1,
        }
    }
}

/// 账务分录。
#[derive(Model, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[table("ledger_entries")]
#[model(no_timestamps)] // created_at 承载业务时间（开票/记账/事件时刻），不用内置 TimestampHook 覆盖
pub struct LedgerEntry {
    #[key]
    pub id: i64,
    pub tenant_id: i64,
    pub account_id: i64,
    pub transfer_id: i64,
    pub amount: Decimal,
    pub direction: Direction,
    pub created_at: DateTime<Utc>,
}

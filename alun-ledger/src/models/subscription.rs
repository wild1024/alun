//! 订阅与用量记录。

use alun_db::prelude::*;
use chrono::{DateTime, Utc};

/// 订阅状态。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
#[value_map(serde, "active" => Active, "canceled" => Canceled)]
pub enum SubscriptionStatus {
    Active,
    Canceled,
}

/// 订阅。
#[derive(Model, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[table("subscriptions")]
pub struct Subscription {
    #[key]
    pub id: i64,
    pub tenant_id: i64,
    pub customer_id: i64,
    pub plan: String,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub status: SubscriptionStatus,
}

/// 用量记录。
#[derive(Model, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[table("usage_records")]
pub struct UsageRecord {
    #[key]
    pub id: i64,
    pub subscription_id: i64,
    pub usage_date: DateTime<Utc>,
    pub quantity: i64,
    #[json]
    pub metadata: Option<serde_json::Value>,
}

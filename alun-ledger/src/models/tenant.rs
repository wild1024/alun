//! 租户。

use alun_db::prelude::*;
use chrono::{DateTime, Utc};

/// 租户（多租户根实体）。
#[derive(Model, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[table("tenants")]
#[hook(TimestampHook)]
pub struct Tenant {
    #[key]
    pub id: i64,
    pub name: String,
    pub plan: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub region: String,
}

impl Tenant {
    pub const ACTIVE: &'static str = "active";
    pub const SUSPENDED: &'static str = "suspended";
}

//! 审计日志。

use alun_db::prelude::*;
use chrono::{DateTime, Utc};

/// 审计日志（只写不改）。
#[derive(Model, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[table("audit_logs")]
#[model(no_timestamps)] // created_at 承载业务时间（开票/记账/事件时刻），不用内置 TimestampHook 覆盖
pub struct AuditLog {
    #[key]
    pub id: i64,
    pub tenant_id: i64,
    pub entity_type: String,
    pub entity_id: i64,
    pub action: String,
    #[json]
    pub before_json: Option<serde_json::Value>,
    #[json]
    pub after_json: Option<serde_json::Value>,
    pub actor_id: i64,
    pub created_at: DateTime<Utc>,
}

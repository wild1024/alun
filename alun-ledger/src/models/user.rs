//! 用户与 JSON profile。

use alun_db::prelude::*;
use chrono::{DateTime, Utc};

/// `users.profile` 列（JSON）的强类型载荷。
#[derive(Row, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub avatar: Option<String>,
    pub bio: String,
}

/// 用户。
#[derive(Model, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[table("users")]
#[hook(TimestampHook)]
pub struct User {
    #[key]
    pub id: i64,
    pub tenant_id: i64,
    #[unique]
    pub email: String,
    pub name: String,
    pub status: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub first_name: String,
    pub last_name: String,
    #[json]
    pub profile: Option<Profile>,
    pub locale: String,
}

impl User {
    pub const ACTIVE: &'static str = "active";
    pub const DISABLED: &'static str = "disabled";
    pub const ROLE_ADMIN: &'static str = "admin";
    pub const ROLE_MEMBER: &'static str = "member";
}

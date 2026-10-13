//! 测试用 Model 定义。
//!
//! 共享模型集：各测试二进制按需取用，未引用的模型不视为死代码。
#![allow(dead_code)]

use alun_db::prelude::*;
use chrono::{DateTime, Utc};

#[derive(Model, Debug, Clone, PartialEq)]
#[table("users")]
pub struct User {
    #[key]
    #[auto]
    pub id: i64,
    #[unique]
    pub email: String,
    pub name: String,
    pub status: i32,
    #[json]
    pub profile: Option<Profile>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Row, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub avatar: Option<String>,
    pub bio: String,
}

#[derive(Model, Debug, Clone, PartialEq)]
#[table("orders")]
pub struct Order {
    #[key]
    #[auto]
    pub id: i64,
    pub user_id: i64,
    pub amount: String,
    pub status: OrderStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(TypeMapper, Debug, Clone, Copy, PartialEq)]
#[value_map("P" => Pending, "C" => Confirmed, "S" => Shipped)]
pub enum OrderStatus {
    Pending,
    Confirmed,
    Shipped,
}

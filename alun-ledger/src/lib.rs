//! Alun Ledger：多租户 SaaS 计费与账务系统。
//!
//! 本项目是「alun + alun-db 融合验证」的参考实现与压力测试平台，
//! 对应需求文档《测试系统需求文档.md》与规划《测试项目测试规划.md》。
//!
//! 分层：
//! - [`models`]：`#[derive(Model)]` 强类型模型
//! - [`services`]：业务服务（事务、Decimal、审计、报表、对账）
//! - [`api`]：REST 端点（`#[alun::get/post/...]`）
//! - [`hooks`]：Hook 实现（时间戳 / 审计 / 顺序探针）

pub mod api;
pub mod hooks;
pub mod models;
pub mod services;

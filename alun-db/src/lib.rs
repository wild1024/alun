//! # alun-db
//!
//! Alun database: Row 模式 + 事务 RAII + Hook + SQL 模板。
//!
//! ## 设计理念
//!
//! - **SQL 表达语义，链式 API 只选执行方式** —— 唯一脊柱是 `sql!` / `Model::query()`，
//!   终态动词固定为 `find` / `find_first` / `find_one` / `exists` / `paginate` /
//!   `for_each` / `execute`（对齐 aifei 的 `Dao`）
//! - **Db + Row 模式** —— 无 Model 也可操作，字段追踪 + 类型安全
//! - **事务隐式提交** —— Rust 用 `?` 天然保证永不「忘记回滚」
//! - **Hook 生命周期** —— before/after 拦截 CRUD，支持审计/时间戳自动填充
//! - **约定优于配置** —— 表名由结构体名推导、`id` 默认主键自增、
//!   `created_at`/`updated_at` 自动填时间戳、`deleted_at` 自动软删
//! - **多数据库支持** —— PostgreSQL / MySQL / SQLite，工厂按 `type` 自动选择
//!
//! ## 快速开始
//!
//! ```ignore
//! use alun_db::prelude::*;
//!
//! #[derive(Model, Debug)]
//! struct User {
//!     id: i64,                     // 名为 id 的整型字段 → 主键 + 自增
//!     name: String,
//!     created_at: DateTime<Utc>,   // 时间戳自动填充（无需挂 Hook）
//! }
//! // 表名自动推导为 `user`；需要别的名字写 `#[table("sys_user")]`
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     alun_db::init("sqlite://./dev.db").await?;
//!
//!     // 强类型脊柱：SQL 表达语义，终态动词执行
//!     let users: Vec<User> = User::query()
//!         .filter(User::name().like("%a%"))
//!         .find(db()).await?;
//!
//!     // 原始 SQL（编译期校验的 `sql!` 模板）
//!     let row_count: i64 = sql!("SELECT COUNT(*) FROM user")
//!         .scalar::<i64, _>(db()).await?;
//!
//!     // 免 SELECT 的部分更新
//!     User::of(1).set(User::name(), "James").update(db()).await?;
//!     Ok(())
//! }
//! ```

#![deny(missing_docs)]
#![warn(clippy::all)]

pub mod builder;
pub mod changeset;
pub mod config;
pub mod crud;
pub mod db;
pub mod factory;
pub mod hook_async;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod migrate;
pub mod model_query;
pub mod persist;
pub mod query;
pub mod rowops;
pub mod tenant;
pub mod tx;

mod executor;

pub mod prelude;

// ==================== 兼容导出 ====================
//
// 供框架层(app.rs / resources.rs / alun prelude)以旧符号使用。

pub use crate::config::{DatabaseConfig, PoolConfig};
pub use crate::db::Db;
pub use crate::migrate::{
    Divergent, DryRunReport, FreshReport, Migration, MigrationKind, MigrationState,
    MigrationStatus, Migrator, MigratorOptions, RepairReport, ResetReport, Version,
    split_statements,
};
pub use crate::tx::Transaction;
pub use alun_db_core::page::Page;
pub use alun_db_core::row::Row;

// ==================== 过程宏 ====================

/// `migrate!` 宏：把迁移目录在**编译期**编入二进制。
///
/// 见 [`alun_db_macro::migrate`]。
pub use alun_db_macro::migrate;

/// 重导出 `async_trait` 属性宏：实现 [`AsyncHook`](alun_db_core::traits::AsyncHook)
/// 时写 `#[alun_db::async_trait]`，无需自行依赖 `async-trait`。
pub use alun_db_core::async_trait;

/// 供过程宏生成代码使用的内部转发。
///
/// 生成代码按 `::<前缀>::…` 引用内核类型：调用方只依赖 `alun-db` 时前缀即
/// `::alun_db::__db_core`（见 `alun-db-macro` 的 `resolve` 模块）。
#[doc(hidden)]
pub use alun_db_core as __db_core;

/// 同上，供 `#[json]` 字段的生成代码解析 `::alun_db::__serde_json::…`。
#[cfg(feature = "json")]
#[doc(hidden)]
pub use serde_json as __serde_json;

// ==================== 全局初始化 ====================

use alun_db_core::error::Result;
use std::sync::OnceLock;

/// 全局数据库实例。
static GLOBAL_DB: OnceLock<db::Db> = OnceLock::new();

/// 初始化全局数据库实例。
///
/// 从连接 URL 创建连接池并注册为全局单例。只能调用一次。
///
/// # 参数
///
/// - `url`：数据库连接 URL，如 `"sqlite://./dev.db"`、`"postgres://..."`
///
/// # 示例
///
/// ```ignore
/// alun_db::init("sqlite://./dev.db").await?;
/// ```
pub async fn init(url: &str) -> Result<()> {
    let db = db::Db::connect(url).await?;
    GLOBAL_DB.set(db).map_err(|_| {
        alun_db_core::error::DbError::Connection("global Db already initialized".into())
    })?;
    Ok(())
}

/// 初始化全局数据库实例（带配置）。
pub async fn init_with(url: &str, config: config::DatabaseConfig) -> Result<()> {
    let db = db::Db::connect_with(url, config).await?;
    GLOBAL_DB.set(db).map_err(|_| {
        alun_db_core::error::DbError::Connection("global Db already initialized".into())
    })?;
    Ok(())
}

/// 获取全局数据库实例。
///
/// # Panics
///
/// 如果尚未调用 [`init`] 或 [`init_with`]，则 panic。
/// 如需安全版本，使用 [`try_db`]。
pub fn db() -> &'static db::Db {
    GLOBAL_DB
        .get()
        .expect("global Db not initialized; call alun_db::init() first")
}

/// 安全获取全局数据库实例。
///
/// 未初始化时返回 `None`。
pub fn try_db() -> Option<&'static db::Db> {
    GLOBAL_DB.get()
}

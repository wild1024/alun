//! # alun-db-driver-sqlite
//!
//! `alun-db` 的 SQLite 驱动实现。
//!
//! ## 架构
//!
//! ```text
//! SqliteDriver
//!   ├── dialect() -> &SqliteDialect
//!   └── connect(url, config) -> SqlitePool
//!         └── SqlitePool
//!               ├── get() -> PooledSqliteConnection
//!               └── begin_transaction() -> SqlitePoolTransaction
//!                     └── SqliteConnection
//!                           ├── execute_sync(stmt) -> ExecuteResult
//!                           └── query_sync(stmt) -> Rows
//! ```
//!
//! ## 同步与异步
//!
//! rusqlite 是同步库。本驱动的定位：
//!
//! - `SqliteConnection`：同步核心，所有数据库操作在此实现
//! - `SqlitePool`：连接池，`get()` 为异步（信号量限流等待）
//! - 异步包装（spawn_blocking、事务句柄的 dyn-safe 适配）由
//!   facade（`alun-db`）统一完成，驱动不重复实现

#![deny(missing_docs)]
#![warn(clippy::all)]

mod connection;
mod convert;
mod driver;
mod pool;
mod transaction;

pub use connection::SqliteConnection;
pub use driver::SqliteDriver;
pub use pool::{PooledSqliteConnection, SqlitePool};
pub use transaction::SqlitePoolTransaction;

pub use alun_db_sql::dialects::sqlite::SqliteDialect;

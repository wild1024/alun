//! # alun-db-driver-postgres
//!
//! `alun-db` 的 PostgreSQL 驱动实现（tokio-postgres）。
//!
//! ## 架构
//!
//! ```text
//! PostgresDriver
//!   ├── dialect() -> &PostgresDialect
//!   └── connect(url, config) -> PostgresPool
//!         └── PostgresPool
//!               ├── get() -> PooledPgConnection
//!               └── begin_transaction() -> PostgresPoolTransaction
//!                     └── PostgresConnection
//!                           ├── execute_stmt(stmt) -> ExecuteResult
//!                           └── query_stmt(stmt) -> Rows
//! ```
//!
//! ## 类型编解码
//!
//! 本驱动**不启用** tokio-postgres 的 chrono / uuid / serde_json 集成
//! feature，避免生态版本冲突：`stmt::Value` 与 PG 二进制格式的互转由
//! 手写的 [`convert`] 模块完成（`PgParam` / `PgValue` 中间表示）。
//!
//! ## 事务 Drop 策略
//!
//! 异步驱动无法在 `Drop` 中等待 `ROLLBACK`。未终结的事务在句柄销毁时
//! **丢弃连接而非归还池**：服务端断连自动回滚未提交事务，池容量随
//! permit 释放恢复。代价是一次重连，换取确定性语义。
//!
//! ## 占位符
//!
//! Raw 语句统一以 `?` 书写，执行前经 [`alun_db_sql::rewrite_placeholders`]
//! 改写为 `$n` 风格，与 `sql!` 宏路径行为一致。

#![deny(missing_docs)]
#![warn(clippy::all)]

mod connection;
mod convert;
mod driver;
mod pool;
mod transaction;

pub use connection::PostgresConnection;
pub use driver::PostgresDriver;
pub use pool::{PooledPgConnection, PostgresPool};
pub use transaction::PostgresPoolTransaction;

pub use alun_db_sql::dialects::postgres::PostgresDialect;

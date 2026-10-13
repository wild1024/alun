//! # alun-db-driver-mysql
//!
//! `alun-db` 的 MySQL 驱动实现（mysql_async）。
//!
//! ## 架构
//!
//! ```text
//! MysqlDriver
//!   ├── dialect() -> &MysqlDialect
//!   └── connect(url, config) -> MysqlPool
//!         └── MysqlPool
//!               ├── get() -> PooledMysqlConnection
//!               └── begin_transaction() -> MysqlPoolTransaction
//!                     └── MysqlConnection
//!                           ├── execute_stmt(stmt) -> ExecuteResult
//!                           └── query_stmt(stmt) -> Rows
//! ```
//!
//! ## 连接管理
//!
//! 裸用 mysql_async 的 [`Conn`]（不使用其自带连接池），池化由自研的
//! [`pool`] 模块完成，与 SQLite / PostgreSQL 驱动同构。
//!
//! ## 类型编解码
//!
//! [`stmt::Value`] 与 MySQL 二进制协议值的互转由 [`convert`] 模块完成。
//! 读取方向需要列元数据（列类型 + flags）区分 `Bytes` 载荷的文本 /
//! 二进制 / 十进制 / JSON 语义。
//!
//! ## 事务 Drop 策略
//!
//! 异步驱动无法在 `Drop` 中等待 `ROLLBACK`。未终结的事务在句柄销毁时
//! **丢弃连接而非归还池**：服务端断连自动回滚未提交事务，池容量随
//! permit 释放恢复。代价是一次重连，换取确定性语义。
//!
//! ## 占位符
//!
//! Raw 语句以 `?` 书写，与 MySQL 原生占位符一致，无需改写。
//!
//! ## 自增主键
//!
//! MySQL 不支持 `RETURNING`，自增主键经 `INSERT` 后的
//! `Conn::last_insert_id` / `QueryResult::last_insert_id` 取回。

#![deny(missing_docs)]
#![warn(clippy::all)]

mod connection;
mod convert;
mod driver;
mod pool;
mod transaction;

pub use connection::MysqlConnection;
pub use driver::MysqlDriver;
pub use pool::{MysqlPool, PooledMysqlConnection};
pub use transaction::MysqlPoolTransaction;

pub use alun_db_sql::dialects::mysql::MysqlDialect;

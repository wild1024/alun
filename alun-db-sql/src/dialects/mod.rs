//! 方言实现。

pub mod mysql;
pub mod postgres;
pub mod sqlite;

pub use mysql::MysqlDialect;
pub use postgres::PostgresDialect;
pub use sqlite::SqliteDialect;

//! SQLite 驱动。

use alun_db_core::error::Result;
use alun_db_core::traits::dialect::Dialect;
use alun_db_core::traits::driver::{Driver, PoolConfig};
use alun_db_sql::dialects::sqlite::SqliteDialect;

use crate::pool::SqlitePool;

/// SQLite 驱动。
pub struct SqliteDriver;

impl Driver for SqliteDriver {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    fn dialect(&self) -> &'static dyn Dialect {
        &SqliteDialect
    }

    type Pool = SqlitePool;

    async fn connect(&self, url: &str, config: PoolConfig) -> Result<Self::Pool> {
        SqlitePool::new(url, config)
    }
}

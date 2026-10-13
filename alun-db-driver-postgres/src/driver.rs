//! PostgreSQL 驱动入口。

use alun_db_core::error::Result;
use alun_db_core::traits::dialect::Dialect;
use alun_db_core::traits::driver::{Driver, PoolConfig};
use alun_db_sql::dialects::postgres::PostgresDialect;

use crate::pool::PostgresPool;

/// PostgreSQL 驱动。
pub struct PostgresDriver;

impl Driver for PostgresDriver {
    fn name(&self) -> &'static str {
        "postgres"
    }

    fn dialect(&self) -> &'static dyn Dialect {
        &PostgresDialect
    }

    type Pool = PostgresPool;

    async fn connect(&self, url: &str, config: PoolConfig) -> Result<Self::Pool> {
        PostgresPool::new(url, config).await
    }
}

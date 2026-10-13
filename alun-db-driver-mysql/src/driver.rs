//! MySQL 驱动入口。

use alun_db_core::error::Result;
use alun_db_core::traits::dialect::Dialect;
use alun_db_core::traits::driver::{Driver, PoolConfig};
use alun_db_sql::dialects::mysql::MysqlDialect;

use crate::pool::MysqlPool;

/// MySQL 驱动。
pub struct MysqlDriver;

impl Driver for MysqlDriver {
    fn name(&self) -> &'static str {
        "mysql"
    }

    fn dialect(&self) -> &'static dyn Dialect {
        &MysqlDialect
    }

    type Pool = MysqlPool;

    async fn connect(&self, url: &str, config: PoolConfig) -> Result<Self::Pool> {
        MysqlPool::new(url, config).await
    }
}

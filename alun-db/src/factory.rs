//! 数据库驱动工厂。按配置自动选择驱动。

use std::sync::Arc;

use alun_db_core::error::{DbError, Result};
use alun_db_core::traits::driver::Driver;

use crate::config::DatabaseConfig;
use crate::executor::RawExecutor;

impl From<crate::config::PoolConfig> for alun_db_core::traits::driver::PoolConfig {
    fn from(c: crate::config::PoolConfig) -> Self {
        Self {
            max_connections: c.max_connections,
            min_connections: c.min_connections,
            connect_timeout: c.connect_timeout,
            idle_timeout: c.idle_timeout,
            max_lifetime: c.max_lifetime,
        }
    }
}

/// 根据配置创建驱动执行器。
///
/// 仅 facade 内部使用（`Db::connect*` 的工厂入口），不对外暴露
/// 内部的 `RawExecutor` 类型。连接建立成功后记录脱敏连接日志，
/// 便于排查而不泄露密码。
pub(crate) async fn create_driver(config: &DatabaseConfig) -> Result<Arc<dyn RawExecutor>> {
    let executor: Arc<dyn RawExecutor> = match config.db_type.as_str() {
        #[cfg(feature = "sqlite")]
        "sqlite" => {
            let driver = alun_db_driver_sqlite::SqliteDriver;
            Arc::new(
                driver
                    .connect(&config.url, config.pool.clone().into())
                    .await?,
            )
        }

        #[cfg(feature = "postgres")]
        "postgres" | "postgresql" => {
            let driver = alun_db_driver_postgres::PostgresDriver;
            Arc::new(
                driver
                    .connect(&config.url, config.pool.clone().into())
                    .await?,
            )
        }

        #[cfg(feature = "mysql")]
        "mysql" => {
            let driver = alun_db_driver_mysql::MysqlDriver;
            Arc::new(
                driver
                    .connect(&config.url, config.pool.clone().into())
                    .await?,
            )
        }

        other => {
            return Err(DbError::UnsupportedFeature {
                feature: "driver",
                dialect: other.to_string(),
            });
        }
    };

    tracing::info!(
        db_type = %config.db_type,
        url = %sanitize_url(&config.url),
        "database connection established"
    );
    Ok(executor)
}

/// 脱敏连接 URL：隐藏 URL 中的密码段，避免日志泄露凭据。
///
/// 仅对 `scheme://user:pass@host/...` 形式脱敏；无密码段时原样返回。
pub(crate) fn sanitize_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let rest = &url[scheme_end + 3..];
    let Some(at_pos) = rest.find('@') else {
        return url.to_string();
    };
    let userinfo = &rest[..at_pos];
    match userinfo.find(':') {
        Some(colon) => format!(
            "{}://{}:***@{}",
            &url[..scheme_end],
            &userinfo[..colon],
            &rest[at_pos + 1..]
        ),
        None => url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_url;

    #[test]
    fn sanitize_hides_password() {
        assert_eq!(
            sanitize_url("postgres://user:secret@localhost:5432/db"),
            "postgres://user:***@localhost:5432/db"
        );
    }

    #[test]
    fn sanitize_keeps_url_without_password() {
        assert_eq!(
            sanitize_url("postgres://user@localhost:5432/db"),
            "postgres://user@localhost:5432/db"
        );
    }

    #[test]
    fn sanitize_keeps_sqlite_url() {
        assert_eq!(sanitize_url("sqlite://./dev.db"), "sqlite://./dev.db");
    }
}

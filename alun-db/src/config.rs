//! 数据库配置。

use std::time::Duration;

/// 数据库配置。
///
/// 支持从连接 URL 或显式字段构造。
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// 数据库类型：`"sqlite"`、`"postgres"`、`"mysql"`。
    pub db_type: String,
    /// 连接 URL（不含密码）。
    pub url: String,
    /// 密码。支持从环境变量读取。
    pub password: Option<String>,
    /// 连接池配置。
    pub pool: PoolConfig,
}

/// 连接池配置。
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// 最大连接数。
    pub max_connections: u32,
    /// 最小连接数。
    pub min_connections: u32,
    /// 连接超时。
    pub connect_timeout: Duration,
    /// 空闲连接超时。
    pub idle_timeout: Option<Duration>,
    /// 连接最大存活时长。
    pub max_lifetime: Option<Duration>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        PoolConfig {
            max_connections: 10,
            min_connections: 1,
            connect_timeout: Duration::from_secs(30),
            idle_timeout: Some(Duration::from_secs(600)),
            max_lifetime: Some(Duration::from_secs(1800)),
        }
    }
}

impl DatabaseConfig {
    /// 从连接 URL 推断数据库类型和配置。
    ///
    /// 支持格式：
    /// - `sqlite://./path/to.db`
    /// - `sqlite::memory:`
    /// - `postgres://user:pass@host/db`
    /// - `mysql://user:pass@host/db`
    pub fn from_url(url: &str) -> Self {
        let db_type = if url.starts_with("sqlite") {
            "sqlite"
        } else if url.starts_with("postgres") || url.starts_with("postgresql") {
            "postgres"
        } else if url.starts_with("mysql") {
            "mysql"
        } else {
            "sqlite" // 默认
        };

        DatabaseConfig {
            db_type: db_type.to_string(),
            url: url.to_string(),
            password: None,
            pool: PoolConfig::default(),
        }
    }

    /// 从环境变量读取密码。
    pub fn with_password_from_env(mut self, env_var: &str) -> Self {
        self.password = std::env::var(env_var).ok();
        self
    }
}

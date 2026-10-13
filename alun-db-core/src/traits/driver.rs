//! `Driver` trait —— 连接管理与方言提供。

use crate::error::Result;
use crate::traits::dialect::Dialect;

/// 驱动 trait。每个数据库后端实现一次。
pub trait Driver: Send + Sync + 'static {
    /// 驱动名称（如 `"sqlite"`、`"postgres"`）。
    fn name(&self) -> &'static str;

    /// 返回该驱动使用的方言。
    fn dialect(&self) -> &'static dyn Dialect;

    /// 连接池类型。由驱动决定，facade 层将其包装为 dyn-safe 执行器。
    type Pool: Send + Sync + 'static;

    /// 建立连接池。
    fn connect(
        &self,
        url: &str,
        config: PoolConfig,
    ) -> impl std::future::Future<Output = Result<Self::Pool>>;
}

/// 连接池配置。
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// 最大连接数。
    pub max_connections: u32,
    /// 最小连接数。
    pub min_connections: u32,
    /// 连接超时。
    pub connect_timeout: std::time::Duration,
    /// 空闲连接超时。
    pub idle_timeout: Option<std::time::Duration>,
    /// 连接最大存活时长。
    pub max_lifetime: Option<std::time::Duration>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        PoolConfig {
            max_connections: 10,
            min_connections: 1,
            connect_timeout: std::time::Duration::from_secs(30),
            idle_timeout: Some(std::time::Duration::from_secs(600)),
            max_lifetime: Some(std::time::Duration::from_secs(1800)),
        }
    }
}

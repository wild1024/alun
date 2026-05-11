//! 缓存插件 —— 管理缓存实例的生命周期
//!
//! 与 `alun-cache` 的关系：
//! - `alun-cache`：提供 Cache trait + LocalCache/RedisCache 实现（纯存储层）
//! - `CachePlugin`：管理缓存实例的 start/stop（生命周期管理层）
//!
//! 使用模式：
//! ```ignore
//! let plugin = CachePlugin::new(&config);
//! plugin.start().await?;
//! let cache = plugin.cache().unwrap();
//! cache.set("key", &"value").await?;
//! plugin.stop().await?;
//! ```

use async_trait::async_trait;
use alun_core::{Plugin, Result};
use alun_cache::SharedCache;
use parking_lot::RwLock;

/// 缓存插件 —— 管理缓存实例的生命周期
///
/// 在 `start()` 时根据配置自动创建 LocalCache 或 RedisCache，
/// `stop()` 时释放连接。
pub struct CachePlugin {
    /// 缓存实例（运行时初始化）
    cache: RwLock<Option<SharedCache>>,
    /// 缓存配置
    cache_config: alun_config::CacheConfig,
    /// Redis 配置
    redis_config: alun_config::RedisConfig,
}

impl CachePlugin {
    /// 创建缓存插件
    pub fn new(cache_config: &alun_config::CacheConfig, redis_config: &alun_config::RedisConfig) -> Self {
        Self {
            cache: RwLock::new(None),
            cache_config: cache_config.clone(),
            redis_config: redis_config.clone(),
        }
    }

    /// 获取缓存实例（需在 start 之后调用）
    pub fn cache(&self) -> Option<SharedCache> {
        self.cache.read().clone()
    }
}

#[async_trait]
impl Plugin for CachePlugin {
    fn name(&self) -> &str { "cache" }

    async fn start(&self) -> Result<()> {
        let instance = alun_cache::create_cache(&self.cache_config, &self.redis_config).await?;
        *self.cache.write() = Some(instance);
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        *self.cache.write() = None;
        Ok(())
    }
}

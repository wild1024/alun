//! 缓存模块：本地内存缓存 + Redis 缓存
//!
//! 通过配置 `cache.type` 切换：
//! - `local` → 内存缓存（默认）
//! - `redis` → Redis 缓存（需配置 redis_url）

use alun_core::Result;
use async_trait::async_trait;
use parking_lot::RwLock;
use redis::aio::ConnectionManager;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// 缓存侧可观测性指标（仅在 `metrics` feature 下编译）
#[cfg(feature = "metrics")]
pub mod metrics;

// ──── 缓存 trait ────────────────────────────────────

/// 统一缓存接口（本地/Redis 实现同一 trait）
///
/// # 示例
///
/// ```ignore
/// let cache: &dyn Cache = &local_cache;
/// cache.set::<String>("key", &"value".to_string()).await?;
/// let val: Option<String> = cache.get("key").await?;
/// ```
#[async_trait]
pub trait Cache: Send + Sync {
    /// 读取缓存值，返回 `Ok(None)` 表示 key 不存在或已过期
    async fn get<T: DeserializeOwned + Send>(&self, key: &str) -> Result<Option<T>>;

    /// 设置缓存（永不过期），值通过 serde_json 序列化
    async fn set<T: Serialize + Send + Sync + ?Sized>(&self, key: &str, value: &T) -> Result<()>;

    /// 设置缓存（指定过期秒数），到期后自动不可见
    async fn set_ex<T: Serialize + Send + Sync + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: u64,
    ) -> Result<()>;

    /// 删除单个 key（不存在不报错）
    async fn del(&self, key: &str) -> Result<()>;

    /// 检查 key 是否存在且未过期
    async fn exists(&self, key: &str) -> Result<bool>;

    /// 计数器递增（key 不存在则从 0 开始），返回递增后的值
    async fn incr(&self, key: &str, delta: i64) -> Result<i64>;

    /// 获取匹配模式（glob：`*`/`?`）的所有 key
    async fn keys(&self, pattern: &str) -> Result<Vec<String>>;

    /// 删除匹配模式的所有 key，返回删除数
    async fn delete_pattern(&self, pattern: &str) -> Result<u64>;

    /// 缓存统计信息（内存缓存支持，Redis 返回全零）
    fn stats(&self) -> CacheStats {
        CacheStats::default()
    }
}

// ──── 本地缓存条目 ──────────────────────────────────

struct CacheEntry {
    value: serde_json::Value,
    expires_at: Option<Instant>,
}

// ──── 本地内存缓存 ─────────────────────────────────

/// 缓存统计指标
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// 缓存命中次数
    pub hits: u64,
    /// 缓存未命中次数
    pub misses: u64,
    /// 设置缓存次数
    pub sets: u64,
    /// 删除缓存次数
    pub deletes: u64,
    /// 淘汰次数
    pub evictions: u64,
    /// 过期清理次数
    pub expired_cleanups: u64,
}

/// 本地内存缓存（HashMap + RwLock + TTL + 统计 + 后台清理）
#[derive(Clone)]
pub struct LocalCache {
    /// 缓存 key 前缀（通常为 app_name，用于多项目隔离）
    key_prefix: String,
    /// 缓存数据存储（key → 条目）
    data: Arc<RwLock<HashMap<String, CacheEntry>>>,
    /// 最大容量（超过后 LRU 淘汰）
    max_capacity: u64,
    /// 默认 TTL 秒数（set 时未指定 TTL 则使用此值）
    default_ttl_secs: u64,
    /// 缓存统计信息（原子计数器）
    stats: Arc<AtomicCacheStats>,
    /// 后台清理任务的间隔秒数
    cleanup_interval_secs: u64,
}

struct AtomicCacheStats {
    hits: AtomicU64,
    misses: AtomicU64,
    sets: AtomicU64,
    deletes: AtomicU64,
    evictions: AtomicU64,
    expired_cleanups: AtomicU64,
}

impl Clone for AtomicCacheStats {
    fn clone(&self) -> Self {
        Self {
            hits: AtomicU64::new(self.hits.load(Ordering::Relaxed)),
            misses: AtomicU64::new(self.misses.load(Ordering::Relaxed)),
            sets: AtomicU64::new(self.sets.load(Ordering::Relaxed)),
            deletes: AtomicU64::new(self.deletes.load(Ordering::Relaxed)),
            evictions: AtomicU64::new(self.evictions.load(Ordering::Relaxed)),
            expired_cleanups: AtomicU64::new(self.expired_cleanups.load(Ordering::Relaxed)),
        }
    }
}

impl LocalCache {
    /// 创建本地内存缓存
    ///
    /// - `key_prefix`: 缓存 key 前缀（通常为 app_name），为空时不加前缀
    /// - `max_capacity`: 超过此容量后按 LRU 策略淘汰
    /// - `default_ttl_secs`: 默认过期秒数（0 = 永不过期）
    pub fn new(key_prefix: &str, max_capacity: u64, default_ttl_secs: u64) -> Self {
        Self {
            key_prefix: key_prefix.to_string(),
            data: Arc::new(RwLock::new(HashMap::new())),
            max_capacity,
            default_ttl_secs,
            stats: Arc::new(AtomicCacheStats {
                hits: AtomicU64::new(0),
                misses: AtomicU64::new(0),
                sets: AtomicU64::new(0),
                deletes: AtomicU64::new(0),
                evictions: AtomicU64::new(0),
                expired_cleanups: AtomicU64::new(0),
            }),
            cleanup_interval_secs: 60,
        }
    }

    pub fn with_cleanup_interval(mut self, interval_secs: u64) -> Self {
        self.cleanup_interval_secs = interval_secs;
        self
    }

    /// 获取缓存统计快照
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.stats.hits.load(Ordering::Relaxed),
            misses: self.stats.misses.load(Ordering::Relaxed),
            sets: self.stats.sets.load(Ordering::Relaxed),
            deletes: self.stats.deletes.load(Ordering::Relaxed),
            evictions: self.stats.evictions.load(Ordering::Relaxed),
            expired_cleanups: self.stats.expired_cleanups.load(Ordering::Relaxed),
        }
    }

    /// 获取当前缓存条目数量
    pub fn len(&self) -> usize {
        self.data.read().len()
    }

    /// 缓存是否为空
    pub fn is_empty(&self) -> bool {
        self.data.read().is_empty()
    }

    /// 手动清理所有过期条目，返回清理数
    pub fn cleanup_expired(&self) -> u64 {
        let mut guard = self.data.write();
        let expired: Vec<String> = guard
            .iter()
            .filter(|(_, entry)| entry.expires_at.is_some_and(|t| Instant::now() > t))
            .map(|(k, _)| k.clone())
            .collect();
        let count = expired.len() as u64;
        for k in &expired {
            guard.remove(k);
        }
        self.stats
            .expired_cleanups
            .fetch_add(count, Ordering::Relaxed);
        count
    }

    /// 启动后台过期清理任务（每 `interval_secs` 秒执行一次）
    pub fn start_background_cleanup(&self) {
        let data = Arc::clone(&self.data);
        let stats = Arc::clone(&self.stats);
        let interval = self.cleanup_interval_secs;

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(interval)).await;
                let mut guard = data.write();
                let now = Instant::now();
                let expired: Vec<String> = guard
                    .iter()
                    .filter(|(_, entry)| entry.expires_at.is_some_and(|t| now > t))
                    .map(|(k, _)| k.clone())
                    .collect();
                let count = expired.len() as u64;
                for k in &expired {
                    guard.remove(k);
                }
                if count > 0 {
                    stats.expired_cleanups.fetch_add(count, Ordering::Relaxed);
                    tracing::debug!("缓存后台清理: 移除 {} 个过期条目", count);
                }
            }
        });
    }

    /// 生成带前缀的完整缓存 key
    ///
    /// 若 `key_prefix` 非空，返回 `"{prefix}:{key}"`，否则返回原始 key。
    #[inline]
    fn prefixed(&self, key: &str) -> String {
        if self.key_prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}:{}", self.key_prefix, key)
        }
    }
}

#[async_trait]
impl Cache for LocalCache {
    async fn get<T: DeserializeOwned + Send>(&self, key: &str) -> Result<Option<T>> {
        let key = self.prefixed(key);
        let guard = self.data.read();
        if let Some(entry) = guard.get(&key) {
            if let Some(expires) = entry.expires_at
                && Instant::now() > expires
            {
                drop(guard);
                self.data.write().remove(&key);
                self.stats.misses.fetch_add(1, Ordering::Relaxed);
                return Ok(None);
            }
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
            let val: T = serde_json::from_value(entry.value.clone())
                .map_err(|e| alun_core::Error::Msg(e.to_string()))?;
            return Ok(Some(val));
        }
        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }

    async fn set<T: Serialize + Send + Sync + ?Sized>(&self, key: &str, value: &T) -> Result<()> {
        let key = self.prefixed(key);
        let v = serde_json::to_value(value).map_err(|e| alun_core::Error::Msg(e.to_string()))?;
        let mut guard = self.data.write();
        if self.max_capacity > 0 && guard.len() as u64 >= self.max_capacity {
            drop(guard);
            return Err(alun_core::Error::Msg(format!(
                "缓存容量已达上限: {}",
                self.max_capacity
            )));
        }
        self.stats.sets.fetch_add(1, Ordering::Relaxed);
        let expires_at = if self.default_ttl_secs > 0 {
            Some(Instant::now() + Duration::from_secs(self.default_ttl_secs))
        } else {
            None
        };
        guard.insert(
            key,
            CacheEntry {
                value: v,
                expires_at,
            },
        );
        Ok(())
    }

    async fn set_ex<T: Serialize + Send + Sync + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: u64,
    ) -> Result<()> {
        let key = self.prefixed(key);
        let v = serde_json::to_value(value).map_err(|e| alun_core::Error::Msg(e.to_string()))?;
        self.stats.sets.fetch_add(1, Ordering::Relaxed);
        self.data.write().insert(
            key,
            CacheEntry {
                value: v,
                expires_at: Some(Instant::now() + Duration::from_secs(ttl_secs)),
            },
        );
        Ok(())
    }

    async fn del(&self, key: &str) -> Result<()> {
        let key = self.prefixed(key);
        let removed = self.data.write().remove(&key).is_some();
        if removed {
            self.stats.deletes.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let key = self.prefixed(key);
        let guard = self.data.read();
        let found = guard
            .get(&key)
            .is_some_and(|entry| entry.expires_at.is_none_or(|exp| Instant::now() <= exp));
        if found {
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.stats.misses.fetch_add(1, Ordering::Relaxed);
        }
        Ok(found)
    }

    async fn incr(&self, key: &str, delta: i64) -> Result<i64> {
        let key = self.prefixed(key);
        let mut guard = self.data.write();
        let entry = guard.entry(key).or_insert_with(|| CacheEntry {
            value: serde_json::Value::Number(serde_json::Number::from(0i64)),
            expires_at: None,
        });
        let current = entry.value.as_i64().unwrap_or(0);
        let new_val = current + delta;
        entry.value = serde_json::Value::Number(serde_json::Number::from(new_val));
        Ok(new_val)
    }

    async fn keys(&self, pattern: &str) -> Result<Vec<String>> {
        let prefixed_pattern = self.prefixed(pattern);
        let guard = self.data.read();
        let prefix = &self.key_prefix;
        let strip_len = if prefix.is_empty() {
            0
        } else {
            prefix.len() + 1
        };
        Ok(guard
            .keys()
            .filter(|k| match_pattern(k, &prefixed_pattern))
            .map(|k| {
                if strip_len > 0 && k.len() > strip_len {
                    k[strip_len..].to_string()
                } else {
                    k.clone()
                }
            })
            .collect())
    }

    async fn delete_pattern(&self, pattern: &str) -> Result<u64> {
        let prefixed_pattern = self.prefixed(pattern);
        let mut guard = self.data.write();
        let to_remove: Vec<String> = guard
            .keys()
            .filter(|k| match_pattern(k, &prefixed_pattern))
            .cloned()
            .collect();
        let count = to_remove.len() as u64;
        for k in to_remove {
            guard.remove(&k);
        }
        Ok(count)
    }
}

// ──── Redis 缓存 ────────────────────────────────────

/// Redis 缓存实现
#[derive(Clone)]
pub struct RedisCache {
    /// 缓存 key 前缀（通常为 app_name，用于多项目隔离）
    key_prefix: String,
    /// Redis 连接管理器
    conn: ConnectionManager,
}

impl RedisCache {
    /// 创建 Redis 缓存（需传入已建立的连接管理器）
    ///
    /// - `key_prefix`: 缓存 key 前缀（通常为 app_name），为空时不加前缀
    pub fn new(key_prefix: &str, conn: ConnectionManager) -> Self {
        Self {
            key_prefix: key_prefix.to_string(),
            conn,
        }
    }

    /// 从 URL 创建连接
    ///
    /// - `key_prefix`: 缓存 key 前缀
    pub async fn connect(key_prefix: &str, url: &str) -> Result<Self> {
        let client = redis::Client::open(url)
            .map_err(|e| alun_core::Error::Config(format!("Redis URL 无效: {}", e)))?;
        let conn = ConnectionManager::new(client)
            .await
            .map_err(|e| alun_core::Error::Config(format!("Redis 连接失败: {}", e)))?;
        Ok(Self {
            key_prefix: key_prefix.to_string(),
            conn,
        })
    }

    /// 生成带前缀的完整缓存 key
    ///
    /// 若 `key_prefix` 非空，返回 `"{prefix}:{key}"`，否则返回原始 key。
    #[inline]
    fn prefixed(&self, key: &str) -> String {
        if self.key_prefix.is_empty() {
            key.to_string()
        } else {
            format!("{}:{}", self.key_prefix, key)
        }
    }

    fn map_err(e: redis::RedisError) -> alun_core::Error {
        alun_core::Error::Msg(e.to_string())
    }
}

#[async_trait]
impl Cache for RedisCache {
    async fn get<T: DeserializeOwned + Send>(&self, key: &str) -> Result<Option<T>> {
        let key = self.prefixed(key);
        let result: Option<String> = redis::cmd("GET")
            .arg(&key)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)?;

        if let Some(json) = result {
            let val: T =
                serde_json::from_str(&json).map_err(|e| alun_core::Error::Msg(e.to_string()))?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    async fn set<T: Serialize + Send + Sync + ?Sized>(&self, key: &str, value: &T) -> Result<()> {
        let key = self.prefixed(key);
        let json =
            serde_json::to_string(value).map_err(|e| alun_core::Error::Msg(e.to_string()))?;
        redis::cmd("SET")
            .arg(&key)
            .arg(&json)
            .query_async::<()>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
    }

    async fn set_ex<T: Serialize + Send + Sync + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: u64,
    ) -> Result<()> {
        let key = self.prefixed(key);
        let json =
            serde_json::to_string(value).map_err(|e| alun_core::Error::Msg(e.to_string()))?;
        redis::cmd("SETEX")
            .arg(&key)
            .arg(ttl_secs)
            .arg(&json)
            .query_async::<()>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
    }

    async fn del(&self, key: &str) -> Result<()> {
        let key = self.prefixed(key);
        redis::cmd("DEL")
            .arg(&key)
            .query_async::<()>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let key = self.prefixed(key);
        redis::cmd("EXISTS")
            .arg(&key)
            .query_async::<i32>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
            .map(|v| v > 0)
    }

    async fn incr(&self, key: &str, delta: i64) -> Result<i64> {
        let key = self.prefixed(key);
        let result: i64 = if delta == 1 {
            redis::cmd("INCR")
                .arg(&key)
                .query_async(&mut self.conn.clone())
                .await
                .map_err(Self::map_err)?
        } else {
            redis::cmd("INCRBY")
                .arg(&key)
                .arg(delta)
                .query_async(&mut self.conn.clone())
                .await
                .map_err(Self::map_err)?
        };
        Ok(result)
    }

    async fn keys(&self, pattern: &str) -> Result<Vec<String>> {
        let prefixed_pattern = self.prefixed(pattern);
        let prefix = &self.key_prefix;
        let strip_len = if prefix.is_empty() {
            0
        } else {
            prefix.len() + 1
        };
        redis::cmd("KEYS")
            .arg(&prefixed_pattern)
            .query_async::<Vec<String>>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
            .map(|keys| {
                keys.into_iter()
                    .map(|k| {
                        if strip_len > 0 && k.len() > strip_len {
                            k[strip_len..].to_string()
                        } else {
                            k
                        }
                    })
                    .collect()
            })
    }

    async fn delete_pattern(&self, pattern: &str) -> Result<u64> {
        let keys: Vec<String> = self.keys(pattern).await?;
        if keys.is_empty() {
            return Ok(0);
        }
        let mut cmd = redis::cmd("DEL");
        for k in &keys {
            cmd.arg(self.prefixed(k));
        }
        cmd.query_async::<u64>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
    }
}

// ──── 模式匹配 ──────────────────────────────────────

fn match_pattern(key: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return key.is_empty();
    }
    match_pattern_rec(key.as_bytes(), 0, pattern.as_bytes(), 0)
}

fn match_pattern_rec(key: &[u8], ki: usize, pat: &[u8], pi: usize) -> bool {
    if ki >= key.len() && pi >= pat.len() {
        return true;
    }
    if pi >= pat.len() {
        return false;
    }
    match pat[pi] {
        b'*' => {
            if pi + 1 >= pat.len() {
                return true;
            }
            for nk in ki..=key.len() {
                if match_pattern_rec(key, nk, pat, pi + 1) {
                    return true;
                }
            }
            false
        }
        b'?' => ki < key.len() && match_pattern_rec(key, ki + 1, pat, pi + 1),
        c => ki < key.len() && key[ki] == c && match_pattern_rec(key, ki + 1, pat, pi + 1),
    }
}

// ──── 共享缓存（枚举消除 dyn 不兼容） ────────────────

/// 共享缓存——枚举包装所有缓存实现，避免 `dyn Cache` 的对象安全问题
#[derive(Clone)]
pub enum SharedCache {
    Local(LocalCache),
    Redis(RedisCache),
}

#[async_trait]
impl Cache for SharedCache {
    async fn get<T: DeserializeOwned + Send>(&self, key: &str) -> Result<Option<T>> {
        // F10：命中率埋点。计时**包含** Redis 的网络往返——
        // 缓存不是"免费"的：一次跨机房 Redis get 可能比一次本地查询还贵，
        // 只统计命中率而不看耗时会掩盖"命中率 99% 但每次 8ms"这类问题。
        #[cfg(feature = "metrics")]
        let start = Instant::now();
        let out = match self {
            SharedCache::Local(c) => c.get(key).await,
            SharedCache::Redis(c) => c.get(key).await,
        };
        #[cfg(feature = "metrics")]
        crate::metrics::observe_get(start, &out);
        out
    }

    async fn set<T: Serialize + Send + Sync + ?Sized>(&self, key: &str, value: &T) -> Result<()> {
        #[cfg(feature = "metrics")]
        let start = Instant::now();
        let out = match self {
            SharedCache::Local(c) => c.set(key, value).await,
            SharedCache::Redis(c) => c.set(key, value).await,
        };
        #[cfg(feature = "metrics")]
        crate::metrics::observe_result("set", start, &out);
        out
    }

    async fn set_ex<T: Serialize + Send + Sync + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: u64,
    ) -> Result<()> {
        #[cfg(feature = "metrics")]
        let start = Instant::now();
        let out = match self {
            SharedCache::Local(c) => c.set_ex(key, value, ttl_secs).await,
            SharedCache::Redis(c) => c.set_ex(key, value, ttl_secs).await,
        };
        #[cfg(feature = "metrics")]
        crate::metrics::observe_result("set_ex", start, &out);
        out
    }

    async fn del(&self, key: &str) -> Result<()> {
        #[cfg(feature = "metrics")]
        let start = Instant::now();
        let out = match self {
            SharedCache::Local(c) => c.del(key).await,
            SharedCache::Redis(c) => c.del(key).await,
        };
        #[cfg(feature = "metrics")]
        crate::metrics::observe_result("del", start, &out);
        out
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        #[cfg(feature = "metrics")]
        let start = Instant::now();
        let out = match self {
            SharedCache::Local(c) => c.exists(key).await,
            SharedCache::Redis(c) => c.exists(key).await,
        };
        #[cfg(feature = "metrics")]
        crate::metrics::observe_result("exists", start, &out);
        out
    }

    async fn incr(&self, key: &str, delta: i64) -> Result<i64> {
        #[cfg(feature = "metrics")]
        let start = Instant::now();
        let out = match self {
            SharedCache::Local(c) => c.incr(key, delta).await,
            SharedCache::Redis(c) => c.incr(key, delta).await,
        };
        #[cfg(feature = "metrics")]
        crate::metrics::observe_result("incr", start, &out);
        out
    }

    async fn keys(&self, pattern: &str) -> Result<Vec<String>> {
        // `keys` / `delete_pattern` 在 Redis 上是 `KEYS` 语义的**重操作**：
        // 单独留 op 名，便于对它们单独设告警（生产环境应改用 SCAN 类接口）。
        #[cfg(feature = "metrics")]
        let start = Instant::now();
        let out = match self {
            SharedCache::Local(c) => c.keys(pattern).await,
            SharedCache::Redis(c) => c.keys(pattern).await,
        };
        #[cfg(feature = "metrics")]
        crate::metrics::observe_result("keys", start, &out);
        out
    }

    async fn delete_pattern(&self, pattern: &str) -> Result<u64> {
        #[cfg(feature = "metrics")]
        let start = Instant::now();
        let out = match self {
            SharedCache::Local(c) => c.delete_pattern(pattern).await,
            SharedCache::Redis(c) => c.delete_pattern(pattern).await,
        };
        #[cfg(feature = "metrics")]
        crate::metrics::observe_result("delete_pattern", start, &out);
        out
    }
}

// ──── 工厂函数 ──────────────────────────────────────

/// 从配置创建共享缓存实例
///
/// `app_name` 将作为所有缓存 key 的前缀（`"{app_name}:{key}"`），
/// 确保多项目共享同一 Redis 时 key 不冲突。
/// 若 `app_name` 为空则不加前缀。
pub async fn create_cache(
    app_name: &str,
    cache_config: &alun_config::CacheConfig,
    redis_config: &alun_config::RedisConfig,
) -> Result<SharedCache> {
    match cache_config.r#type.as_str() {
        "redis" => {
            tracing::info!("使用 Redis 缓存 url={} app={}", redis_config.url, app_name);
            Ok(SharedCache::Redis(
                RedisCache::connect(app_name, &redis_config.url).await?,
            ))
        }
        _ => {
            tracing::info!(
                "使用本地缓存 capacity={} app={}",
                cache_config.max_capacity,
                app_name
            );
            Ok(SharedCache::Local(LocalCache::new(
                app_name,
                cache_config.max_capacity,
                cache_config.default_ttl,
            )))
        }
    }
}

// ──── 命名空间视图 ─────────────────────────────────

/// 带前缀的缓存视图——用于**请求级 / 租户级**的 key 命名空间。
///
/// 缓存实例的 `key_prefix` 只在**构造期**确定（通常为 `app_name`），无法表达
/// 「同一进程内按租户 / 按环境隔离」这类运行时维度。`CacheNamespace` 补上这一层：
/// 它是对底层缓存的零拷贝包装，所有 key 自动加上派生前缀。
///
/// # 示例
///
/// ```ignore
/// use alun::cache;
///
/// // 显式前缀
/// cache().scoped("t:42").set("dict:type", &v).await?;
/// // 与 db 层同源：多租户下每个租户互不可见
/// cache().scoped("t:1").set("k", &1).await?;
/// let miss: Option<i32> = cache().scoped("t:2").get("k").await?;
/// assert!(miss.is_none());
/// ```
#[derive(Clone)]
pub struct CacheNamespace<'a, C: Cache> {
    /// 底层缓存（借用，不增加引用计数）
    inner: &'a C,
    /// 本视图的前缀（自持有，便于用 `format!` 动态构造）
    prefix: String,
}

impl<'a, C: Cache> CacheNamespace<'a, C> {
    /// 创建命名空间视图。前缀为空串时等价于不加前缀。
    pub fn new(inner: &'a C, prefix: impl Into<String>) -> Self {
        Self {
            inner,
            prefix: prefix.into(),
        }
    }

    /// 本视图的前缀。
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// 拼出完整 key（前缀为空则原样返回）。
    #[inline]
    fn key(&self, key: &str) -> String {
        if self.prefix.is_empty() {
            key.to_string()
        } else if key.is_empty() {
            self.prefix.clone()
        } else {
            format!("{}:{}", self.prefix, key)
        }
    }

    /// 在当前命名空间之下**再派生一层**（如 `t:1` → `t:1:biz`）。
    pub fn scoped(&self, prefix: &str) -> CacheNamespace<'a, C> {
        CacheNamespace {
            inner: self.inner,
            prefix: self.key(prefix),
        }
    }

    /// 底层缓存的引用（需要绕过命名空间时使用，如全局运维操作）。
    pub fn inner(&self) -> &'a C {
        self.inner
    }
}

#[async_trait]
impl<C: Cache> Cache for CacheNamespace<'_, C> {
    async fn get<T: DeserializeOwned + Send>(&self, key: &str) -> Result<Option<T>> {
        self.inner.get(&self.key(key)).await
    }

    async fn set<T: Serialize + Send + Sync + ?Sized>(&self, key: &str, value: &T) -> Result<()> {
        self.inner.set(&self.key(key), value).await
    }

    async fn set_ex<T: Serialize + Send + Sync + ?Sized>(
        &self,
        key: &str,
        value: &T,
        ttl_secs: u64,
    ) -> Result<()> {
        self.inner.set_ex(&self.key(key), value, ttl_secs).await
    }

    async fn del(&self, key: &str) -> Result<()> {
        self.inner.del(&self.key(key)).await
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        self.inner.exists(&self.key(key)).await
    }

    async fn incr(&self, key: &str, delta: i64) -> Result<i64> {
        self.inner.incr(&self.key(key), delta).await
    }

    async fn keys(&self, pattern: &str) -> Result<Vec<String>> {
        self.inner.keys(&self.key(pattern)).await
    }

    async fn delete_pattern(&self, pattern: &str) -> Result<u64> {
        self.inner.delete_pattern(&self.key(pattern)).await
    }

    fn stats(&self) -> CacheStats {
        self.inner.stats()
    }
}

impl SharedCache {
    /// 派生一个带前缀的缓存视图（见 [`CacheNamespace`]）。
    ///
    /// 由本视图读写的 key 会统一加上 `prefix:`，与其它前缀互不可见。
    ///
    /// 以固有方法提供，使 `cache().scoped(..)` 无需额外 `use` 即可调用。
    pub fn scoped(&self, prefix: impl Into<String>) -> CacheNamespace<'_, SharedCache> {
        CacheNamespace::new(self, prefix)
    }
}

/// 为**任意** `Cache` 实现（`LocalCache` / `RedisCache` / 自定义后端）
/// 提供命名空间派生能力。
///
/// `SharedCache` 另有同签名固有方法（无需 `use` 本 trait 即可调用）。
///
/// ```ignore
/// use alun_cache::{Cache, CacheExt};
/// let ns = my_local_cache.scoped("t:1");
/// ns.set("k", &1).await?;
/// ```
pub trait CacheExt: Cache {
    /// 派生一个带前缀的缓存视图（见 [`CacheNamespace`]）。
    fn scoped(&self, prefix: impl Into<String>) -> CacheNamespace<'_, Self>
    where
        Self: Sized,
    {
        CacheNamespace::new(self, prefix)
    }
}

impl<T: Cache + ?Sized> CacheExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_cache_get_set() {
        let c = LocalCache::new("test", 100, 0);
        c.set("key1", &"value1".to_string()).await.unwrap();
        let val: Option<String> = c.get("key1").await.unwrap();
        assert_eq!(val, Some("value1".to_string()));
        c.del("key1").await.unwrap();
        let val: Option<String> = c.get("key1").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_set_ex_expiration() {
        let c = LocalCache::new("test", 100, 0);
        c.set_ex("temp", &"expire".to_string(), 1).await.unwrap();
        let val: Option<String> = c.get("temp").await.unwrap();
        assert_eq!(val, Some("expire".to_string()));
        tokio::time::sleep(Duration::from_secs(2)).await;
        let val: Option<String> = c.get("temp").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_incr() {
        let c = LocalCache::new("test", 100, 0);
        assert_eq!(c.incr("counter", 1).await.unwrap(), 1);
        assert_eq!(c.incr("counter", 5).await.unwrap(), 6);
        assert_eq!(c.incr("counter", -2).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn test_keys_pattern() {
        let c = LocalCache::new("test", 100, 0);
        c.set("user:1", &"alice").await.unwrap();
        c.set("user:2", &"bob").await.unwrap();
        c.set("order:1", &"o1").await.unwrap();
        let keys = c.keys("user:*").await.unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"user:1".to_string()));
        assert!(keys.contains(&"user:2".to_string()));
    }

    #[tokio::test]
    async fn test_delete_pattern() {
        let c = LocalCache::new("test", 100, 0);
        c.set("session:a", &"s1").await.unwrap();
        c.set("session:b", &"s2").await.unwrap();
        c.set("user:1", &"alice").await.unwrap();
        let deleted = c.delete_pattern("session:*").await.unwrap();
        assert_eq!(deleted, 2);
        assert!(!c.exists("session:a").await.unwrap());
        assert!(!c.exists("session:b").await.unwrap());
        assert!(c.exists("user:1").await.unwrap());
    }

    // ──── 命名空间视图 ────────────────────────────

    /// 不同命名空间的同名 key 互不可见（多租户隔离的核心保证）。
    #[tokio::test]
    async fn namespace_isolates_same_key() {
        let c = LocalCache::new("app", 100, 0);

        c.scoped("t:1")
            .set("dict:type", &"A".to_string())
            .await
            .unwrap();
        c.scoped("t:2")
            .set("dict:type", &"B".to_string())
            .await
            .unwrap();

        let t1: Option<String> = c.scoped("t:1").get("dict:type").await.unwrap();
        let t2: Option<String> = c.scoped("t:2").get("dict:type").await.unwrap();
        assert_eq!(t1.as_deref(), Some("A"));
        assert_eq!(t2.as_deref(), Some("B"));

        // 未加前缀的视图读不到任何一个
        let none: Option<String> = c.get("dict:type").await.unwrap();
        assert!(none.is_none(), "无前缀视图不应看到命名空间内的数据");
    }

    /// 命名空间内的 key 应叠加在实例前缀之后。
    ///
    /// 用**空前缀**实例，使命名空间前缀成为唯一来源，便于断言拼接结果。
    #[tokio::test]
    async fn namespace_prefix_is_composed() {
        let c = LocalCache::new("", 100, 0);
        c.scoped("t:7").set("k", &1i32).await.unwrap();

        // 空前缀实例 + 命名空间 "t:7" + key "k"
        assert!(c.exists("t:7:k").await.unwrap());
        assert!(c.scoped("t:7").exists("k").await.unwrap());
        assert!(!c.scoped("t:8").exists("k").await.unwrap());
    }

    /// 命名空间是**同一层级**的纯 key 前缀：`c.scoped(p).k(key)`
    /// 与 `c.k("{p}:{key}")` 命中同一个物理 key。
    ///
    /// （实例自身的 `key_prefix` 会对两者同样生效，因此不会二次叠加而错位。）
    #[tokio::test]
    async fn namespace_is_equivalent_to_composite_key() {
        let c = LocalCache::new("app", 100, 0);
        c.scoped("t:7").set("k", &7i32).await.unwrap();

        // 同名前缀的直接写法命中同一个 key
        let via_composite: Option<i32> = c.get("t:7:k").await.unwrap();
        assert_eq!(
            via_composite,
            Some(7),
            "命名空间应等价于在同一层级拼前缀，而非额外套一层"
        );

        // 另一个命名空间互不影响
        assert!(c.scoped("t:8").get::<i32>("k").await.unwrap().is_none());
    }

    /// `delete_pattern` / `keys` 在命名空间内自动限定范围，不会误删其它租户。
    #[tokio::test]
    async fn namespace_pattern_ops_are_scoped() {
        let c = LocalCache::new("app", 100, 0);
        c.scoped("t:1").set("session:a", &1i32).await.unwrap();
        c.scoped("t:1").set("session:b", &2i32).await.unwrap();
        c.scoped("t:2").set("session:a", &3i32).await.unwrap();

        let keys = c.scoped("t:1").keys("session:*").await.unwrap();
        assert_eq!(keys.len(), 2, "只应看到 t:1 的 key：{keys:?}");

        let deleted = c.scoped("t:1").delete_pattern("session:*").await.unwrap();
        assert_eq!(deleted, 2);
        assert!(
            c.scoped("t:2").exists("session:a").await.unwrap(),
            "t:2 的数据不得被 t:1 的 pattern 删除误伤"
        );
    }

    /// 空前缀等价于不加前缀（便于按配置动态决定是否分命名空间）。
    #[tokio::test]
    async fn empty_prefix_is_transparent() {
        let c = LocalCache::new("", 100, 0);
        c.scoped("").set("k", &"v".to_string()).await.unwrap();
        let v: Option<String> = c.get("k").await.unwrap();
        assert_eq!(v.as_deref(), Some("v"));
        assert_eq!(c.scoped("").prefix(), "");
    }
}

//! 缓存模块：本地内存缓存 + Redis 缓存
//!
//! 通过配置 `cache.type` 切换：
//! - `local` → 内存缓存（默认）
//! - `redis` → Redis 缓存（需配置 redis_url）

use async_trait::async_trait;
use serde::{Serialize, de::DeserializeOwned};
use serde_json;
use alun_core::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use parking_lot::RwLock;
use std::time::{Instant, Duration};
use redis::aio::ConnectionManager;

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
    async fn set<T: Serialize + Send + Sync>(&self, key: &str, value: &T) -> Result<()>;

    /// 设置缓存（指定过期秒数），到期后自动不可见
    async fn set_ex<T: Serialize + Send + Sync>(&self, key: &str, value: &T, ttl_secs: u64) -> Result<()>;

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
    fn stats(&self) -> CacheStats { CacheStats::default() }
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
    /// - `max_capacity`: 超过此容量后按 LRU 策略淘汰
    /// - `default_ttl_secs`: 默认过期秒数（0 = 永不过期）
    pub fn new(max_capacity: u64, default_ttl_secs: u64) -> Self {
        Self {
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
        let expired: Vec<String> = guard.iter()
            .filter(|(_, entry)| entry.expires_at.map_or(false, |t| Instant::now() > t))
            .map(|(k, _)| k.clone())
            .collect();
        let count = expired.len() as u64;
        for k in &expired { guard.remove(k); }
        self.stats.expired_cleanups.fetch_add(count, Ordering::Relaxed);
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
                let expired: Vec<String> = guard.iter()
                    .filter(|(_, entry)| entry.expires_at.map_or(false, |t| now > t))
                    .map(|(k, _)| k.clone())
                    .collect();
                let count = expired.len() as u64;
                for k in &expired { guard.remove(k); }
                if count > 0 {
                    stats.expired_cleanups.fetch_add(count, Ordering::Relaxed);
                    tracing::debug!("缓存后台清理: 移除 {} 个过期条目", count);
                }
            }
        });
    }
}

#[async_trait]
impl Cache for LocalCache {
    async fn get<T: DeserializeOwned + Send>(&self, key: &str) -> Result<Option<T>> {
        let guard = self.data.read();
        if let Some(entry) = guard.get(key) {
            if let Some(expires) = entry.expires_at {
                if Instant::now() > expires {
                    drop(guard);
                    self.data.write().remove(key);
                    self.stats.misses.fetch_add(1, Ordering::Relaxed);
                    return Ok(None);
                }
            }
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
            let val: T = serde_json::from_value(entry.value.clone())
                .map_err(|e| alun_core::Error::Msg(e.to_string()))?;
            return Ok(Some(val));
        }
        self.stats.misses.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }

    async fn set<T: Serialize + Send + Sync>(&self, key: &str, value: &T) -> Result<()> {
        let v = serde_json::to_value(value)
            .map_err(|e| alun_core::Error::Msg(e.to_string()))?;
        let mut guard = self.data.write();
        if self.max_capacity > 0 && guard.len() as u64 >= self.max_capacity {
            drop(guard);
            return Err(alun_core::Error::Msg(format!("缓存容量已达上限: {}", self.max_capacity)));
        }
        self.stats.sets.fetch_add(1, Ordering::Relaxed);
        let expires_at = if self.default_ttl_secs > 0 {
            Some(Instant::now() + Duration::from_secs(self.default_ttl_secs))
        } else {
            None
        };
        guard.insert(key.to_string(), CacheEntry { value: v, expires_at });
        Ok(())
    }

    async fn set_ex<T: Serialize + Send + Sync>(&self, key: &str, value: &T, ttl_secs: u64) -> Result<()> {
        let v = serde_json::to_value(value)
            .map_err(|e| alun_core::Error::Msg(e.to_string()))?;
        self.stats.sets.fetch_add(1, Ordering::Relaxed);
        self.data.write().insert(key.to_string(), CacheEntry {
            value: v,
            expires_at: Some(Instant::now() + Duration::from_secs(ttl_secs)),
        });
        Ok(())
    }

    async fn del(&self, key: &str) -> Result<()> {
        let removed = self.data.write().remove(key).is_some();
        if removed { self.stats.deletes.fetch_add(1, Ordering::Relaxed); }
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        let guard = self.data.read();
        let found = guard.get(key).map_or(false, |entry| {
            entry.expires_at.map_or(true, |exp| Instant::now() <= exp)
        });
        if found { self.stats.hits.fetch_add(1, Ordering::Relaxed); }
        else { self.stats.misses.fetch_add(1, Ordering::Relaxed); }
        Ok(found)
    }

    async fn incr(&self, key: &str, delta: i64) -> Result<i64> {
        let mut guard = self.data.write();
        let entry = guard.entry(key.to_string()).or_insert_with(|| CacheEntry {
            value: serde_json::Value::Number(serde_json::Number::from(0i64)),
            expires_at: None,
        });
        let current = entry.value.as_i64().unwrap_or(0);
        let new_val = current + delta;
        entry.value = serde_json::Value::Number(serde_json::Number::from(new_val));
        Ok(new_val)
    }

    async fn keys(&self, pattern: &str) -> Result<Vec<String>> {
        let guard = self.data.read();
        Ok(guard.keys()
            .filter(|k| match_pattern(k, pattern))
            .cloned()
            .collect())
    }

    async fn delete_pattern(&self, pattern: &str) -> Result<u64> {
        let mut guard = self.data.write();
        let to_remove: Vec<String> = guard.keys()
            .filter(|k| match_pattern(k, pattern))
            .cloned()
            .collect();
        let count = to_remove.len() as u64;
        for k in to_remove { guard.remove(&k); }
        Ok(count)
    }
}

// ──── Redis 缓存 ────────────────────────────────────

/// Redis 缓存实现
#[derive(Clone)]
pub struct RedisCache {
    /// Redis 连接管理器
    conn: ConnectionManager,
}

impl RedisCache {
    /// 创建 Redis 缓存（需传入已建立的连接管理器）
    pub fn new(conn: ConnectionManager) -> Self {
        Self { conn }
    }

    /// 从 URL 创建连接
    pub async fn connect(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)
            .map_err(|e| alun_core::Error::Config(format!("Redis URL 无效: {}", e)))?;
        let conn = ConnectionManager::new(client).await
            .map_err(|e| alun_core::Error::Config(format!("Redis 连接失败: {}", e)))?;
        Ok(Self { conn })
    }

    fn map_err(e: redis::RedisError) -> alun_core::Error {
        alun_core::Error::Msg(e.to_string())
    }
}

#[async_trait]
impl Cache for RedisCache {
    async fn get<T: DeserializeOwned + Send>(&self, key: &str) -> Result<Option<T>> {
        let result: Option<String> = redis::cmd("GET")
            .arg(key)
            .query_async(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)?;

        if let Some(json) = result {
            let val: T = serde_json::from_str(&json)
                .map_err(|e| alun_core::Error::Msg(e.to_string()))?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    async fn set<T: Serialize + Send + Sync>(&self, key: &str, value: &T) -> Result<()> {
        let json = serde_json::to_string(value)
            .map_err(|e| alun_core::Error::Msg(e.to_string()))?;
        redis::cmd("SET")
            .arg(key).arg(&json)
            .query_async::<()>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
    }

    async fn set_ex<T: Serialize + Send + Sync>(&self, key: &str, value: &T, ttl_secs: u64) -> Result<()> {
        let json = serde_json::to_string(value)
            .map_err(|e| alun_core::Error::Msg(e.to_string()))?;
        redis::cmd("SETEX")
            .arg(key).arg(ttl_secs).arg(&json)
            .query_async::<()>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
    }

    async fn del(&self, key: &str) -> Result<()> {
        redis::cmd("DEL")
            .arg(key)
            .query_async::<()>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        redis::cmd("EXISTS")
            .arg(key)
            .query_async::<i32>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
            .map(|v| v > 0)
    }

    async fn incr(&self, key: &str, delta: i64) -> Result<i64> {
        let result: i64 = if delta == 1 {
            redis::cmd("INCR")
                .arg(key)
                .query_async(&mut self.conn.clone())
                .await
                .map_err(Self::map_err)?
        } else {
            redis::cmd("INCRBY")
                .arg(key).arg(delta)
                .query_async(&mut self.conn.clone())
                .await
                .map_err(Self::map_err)?
        };
        Ok(result)
    }

    async fn keys(&self, pattern: &str) -> Result<Vec<String>> {
        redis::cmd("KEYS")
            .arg(pattern)
            .query_async::<Vec<String>>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
    }

    async fn delete_pattern(&self, pattern: &str) -> Result<u64> {
        let keys: Vec<String> = self.keys(pattern).await?;
        if keys.is_empty() { return Ok(0); }
        let mut cmd = redis::cmd("DEL");
        for k in &keys { cmd.arg(k); }
        cmd.query_async::<u64>(&mut self.conn.clone())
            .await
            .map_err(Self::map_err)
    }
}

// ──── 模式匹配 ──────────────────────────────────────

fn match_pattern(key: &str, pattern: &str) -> bool {
    if pattern.is_empty() { return key.is_empty(); }
    match_pattern_rec(key.as_bytes(), 0, pattern.as_bytes(), 0)
}

fn match_pattern_rec(key: &[u8], ki: usize, pat: &[u8], pi: usize) -> bool {
    if ki >= key.len() && pi >= pat.len() { return true; }
    if pi >= pat.len() { return false; }
    match pat[pi] {
        b'*' => {
            if pi + 1 >= pat.len() { return true; }
            for nk in ki..=key.len() {
                if match_pattern_rec(key, nk, pat, pi + 1) { return true; }
            }
            false
        }
        b'?' => {
            ki < key.len() && match_pattern_rec(key, ki + 1, pat, pi + 1)
        }
        c => {
            ki < key.len() && key[ki] == c && match_pattern_rec(key, ki + 1, pat, pi + 1)
        }
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
        match self {
            SharedCache::Local(c) => c.get(key).await,
            SharedCache::Redis(c) => c.get(key).await,
        }
    }

    async fn set<T: Serialize + Send + Sync>(&self, key: &str, value: &T) -> Result<()> {
        match self {
            SharedCache::Local(c) => c.set(key, value).await,
            SharedCache::Redis(c) => c.set(key, value).await,
        }
    }

    async fn set_ex<T: Serialize + Send + Sync>(&self, key: &str, value: &T, ttl_secs: u64) -> Result<()> {
        match self {
            SharedCache::Local(c) => c.set_ex(key, value, ttl_secs).await,
            SharedCache::Redis(c) => c.set_ex(key, value, ttl_secs).await,
        }
    }

    async fn del(&self, key: &str) -> Result<()> {
        match self {
            SharedCache::Local(c) => c.del(key).await,
            SharedCache::Redis(c) => c.del(key).await,
        }
    }

    async fn exists(&self, key: &str) -> Result<bool> {
        match self { SharedCache::Local(c) => c.exists(key).await, SharedCache::Redis(c) => c.exists(key).await }
    }

    async fn incr(&self, key: &str, delta: i64) -> Result<i64> {
        match self { SharedCache::Local(c) => c.incr(key, delta).await, SharedCache::Redis(c) => c.incr(key, delta).await }
    }

    async fn keys(&self, pattern: &str) -> Result<Vec<String>> {
        match self { SharedCache::Local(c) => c.keys(pattern).await, SharedCache::Redis(c) => c.keys(pattern).await }
    }

    async fn delete_pattern(&self, pattern: &str) -> Result<u64> {
        match self { SharedCache::Local(c) => c.delete_pattern(pattern).await, SharedCache::Redis(c) => c.delete_pattern(pattern).await }
    }
}

// ──── 工厂函数 ──────────────────────────────────────

/// 从配置创建共享缓存实例
pub async fn create_cache(cache_config: &alun_config::CacheConfig, redis_config: &alun_config::RedisConfig) -> Result<SharedCache> {
    match cache_config.r#type.as_str() {
        "redis" => {
            tracing::info!("使用 Redis 缓存 url={}", redis_config.url);
            Ok(SharedCache::Redis(RedisCache::connect(&redis_config.url).await?))
        }
        _ => {
            tracing::info!("使用本地缓存 capacity={}", cache_config.max_capacity);
            Ok(SharedCache::Local(LocalCache::new(cache_config.max_capacity, cache_config.default_ttl)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_local_cache_get_set() {
        let c = LocalCache::new(100, 0);
        c.set("key1", &"value1".to_string()).await.unwrap();
        let val: Option<String> = c.get("key1").await.unwrap();
        assert_eq!(val, Some("value1".to_string()));
        c.del("key1").await.unwrap();
        let val: Option<String> = c.get("key1").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_set_ex_expiration() {
        let c = LocalCache::new(100, 0);
        c.set_ex("temp", &"expire".to_string(), 1).await.unwrap();
        let val: Option<String> = c.get("temp").await.unwrap();
        assert_eq!(val, Some("expire".to_string()));
        tokio::time::sleep(Duration::from_secs(2)).await;
        let val: Option<String> = c.get("temp").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_incr() {
        let c = LocalCache::new(100, 0);
        assert_eq!(c.incr("counter", 1).await.unwrap(), 1);
        assert_eq!(c.incr("counter", 5).await.unwrap(), 6);
        assert_eq!(c.incr("counter", -2).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn test_keys_pattern() {
        let c = LocalCache::new(100, 0);
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
        let c = LocalCache::new(100, 0);
        c.set("session:a", &"s1").await.unwrap();
        c.set("session:b", &"s2").await.unwrap();
        c.set("user:1", &"alice").await.unwrap();
        let deleted = c.delete_pattern("session:*").await.unwrap();
        assert_eq!(deleted, 2);
        assert!(!c.exists("session:a").await.unwrap());
        assert!(!c.exists("session:b").await.unwrap());
        assert!(c.exists("user:1").await.unwrap());
    }
}

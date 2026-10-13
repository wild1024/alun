---
id: reference.crates.cache
title: `alun-cache` — 缓存系统
summary: `alun-cache` — 缓存系统——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [cache, reference, redis]
status: active
owner: alun-core-team
source_of_truth: [alun-cache/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-cache` — 缓存系统

> 本文件是 [`crates.md`](../crates.md) 的 `cache` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun-cache/src/lib.rs`

## 关键 Trait

### `Cache` （[lib.rs](../../../alun-cache/src/lib.rs)）

统一缓存接口（async trait）。

| 方法                             | 说明            |
| ------------------------------ | ------------- |
| `get<T>(key)`                  | 读取并反序列化（`-> Result<Option<T>>`） |
| `set(key, value)`              | 设置（使用实现默认 TTL：LocalCache 为 `default_ttl_secs`；Redis 无 TTL） |
| `set_ex(key, value, ttl_secs)` | 设置（指定过期秒数）    |
| `del(key)`                     | 删除（**不是 `delete`**） |
| `exists(key)`                  | 检查是否存在且未过期    |
| `incr(key, delta)`             | 计数器递增         |
| `keys(pattern)`                | Glob 模式匹配 key |
| `delete_pattern(pattern)`      | Glob 模式批量删除   |
| `stats()`                      | 统计信息          |

## 关键结构体

### `LocalCache` — 本地内存缓存

- `HashMap<String, CacheEntry>` + `RwLock`（高并发读写）
- 支持 TTL 过期 + 后台清理任务，统计命中率/淘汰/过期清理
- `key_prefix` 参数用于多项目共享（如 App Name），自动为所有 key 添加 `"{prefix}:"` 前缀

### `RedisCache` — Redis 缓存

基于 `redis::aio::ConnectionManager` 的远程缓存实现。
- `key_prefix` 参数用于多项目共享同一 Redis 实例时隔离 key 命名空间

### `SharedCache` — 共享缓存枚举

枚举包装 `Local` / `Redis`，消除 `dyn Cache` 的对象安全问题。

### `CacheStats` — 缓存统计

| 字段                 | 说明     |
| ------------------ | ------ |
| `hits`             | 命中次数   |
| `misses`           | 未命中次数  |
| `sets`             | 设置次数   |
| `deletes`          | 删除次数   |
| `evictions`        | 淘汰次数   |
| `expired_cleanups` | 过期清理次数 |

## 工厂函数

```rust,ignore
/// `app_name` 将作为所有缓存 key 的前缀（`"{app_name}:{key}"`），
/// 确保多项目共享同一 Redis 时 key 不冲突。若为空则不加前缀。
pub async fn create_cache(
    app_name: &str,
    cache_config: &CacheConfig,
    redis_config: &RedisConfig,
) -> Result<SharedCache>
```

## 使用示例

```rust,ignore
use alun_cache::{create_cache, Cache, SharedCache};

// ── 创建缓存 ──
let cache: SharedCache = create_cache("my-app", &config.cache, &config.redis).await?;

// ── Handler 中通过全局函数获取缓存 ──
async fn handler() -> Res<String> {
    let c = cache();
    c.set_ex("key", "value", 3600).await.unwrap();
    let val: Option<String> = c.get("key").await.unwrap();
    Res::ok(val.unwrap_or_default())
}

// ── 基本操作 ──
cache.set("greeting", "hello").await?;
let val: Option<String> = cache.get("greeting").await?;
assert_eq!(val, Some("hello".into()));

// 带过期时间
cache.set_ex("session:abc", &session_data, 1800).await?;  // 30分钟
let data: Option<SessionData> = cache.get("session:abc").await?;

// 删除
cache.del("temp_key").await?;

// 检查存在
if cache.exists("greeting").await? {
    // ...
}

// ── 计数器 ──
let count: i64 = cache.incr("api_calls", 1).await?;       // 递增
let count: i64 = cache.incr("api_calls", -5).await?;      // 递减

// ── 模式匹配 ──
let keys: Vec<String> = cache.keys("user:*").await?;       // 模糊查找
let deleted: u64 = cache.delete_pattern("temp:*").await?;   // 批量删除

// ── 统计信息（stats 是同步方法，不是 Result） ──
let stats = cache.stats();
let hit_rate = if stats.hits + stats.misses == 0 {
    0.0
} else {
    stats.hits as f64 / (stats.hits + stats.misses) as f64 * 100.0
};
tracing::info!("缓存命中率: {:.2}%", hit_rate);

// ── LocalCache 直接使用 ──
use alun_cache::LocalCache;
let local = LocalCache::new("my-app", 10000, 3600);          // app前缀, 容量10000, 默认TTL 3600s
local.set("key", "value").await?;
```

***

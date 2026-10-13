# alun-cache

Alun 缓存抽象：本地内存缓存 + Redis，外加一层**命名空间视图**（按业务域 / 租户切 key）。

> 业务代码通常不直接依赖本 crate —— 门面侧 `features = ["cache"]` 后，
> 用 `alun::prelude::*` 里的 `cache()` / `try_cache()` / `set_cache()` 取全局缓存句柄即可。

## 主要 API

全部在 crate 根：

- `trait Cache`（async）—— `get<T: DeserializeOwned>(key) -> Result<Option<T>>`、
  `set<T: Serialize>(key, &value)`、`set_ex(key, &value, ttl_secs)`、`del(key)`、`exists(key)`、
  `incr(key, delta) -> Result<i64>`、`keys(pattern) -> Result<Vec<String>>`、
  `delete_pattern(pattern) -> Result<u64>`、`stats() -> CacheStats`（默认实现）
- `CacheStats` —— `hits` / `misses` / `sets` / `deletes` / `evictions` / `expired_cleanups`
- `LocalCache` —— `new(key_prefix, max_capacity, default_ttl_secs)`、`with_cleanup_interval(secs)`、
  `stats()`、`len()`、`is_empty()`、`cleanup_expired()`、`start_background_cleanup()`
- `RedisCache` —— `connect(key_prefix, url)`、`new(key_prefix, conn)`（复用已有的
  `redis::aio::ConnectionManager`）
- `SharedCache` —— 枚举 `{ Local(LocalCache), Redis(RedisCache) }`，本身也实现 `Cache`；
  用作框架级缓存句柄（如 `alun_web::set_cache`）
- `CacheNamespace<'a, C>` —— 命名空间视图：`new(inner, prefix)`、`prefix()`、`scoped(sub)`，
  以及全套 `get` / `set` / `set_ex` / `del` / `exists` / `keys` / `delete_pattern`（自动加前缀）
- `create_cache(app_name, &CacheConfig, &RedisConfig) -> Result<SharedCache>` —— 按配置里
  `cache.type` 选后端并构造

这里的 `Result` 是 `alun_core::Result`。`cache()` / `try_cache()` / `set_cache()` 这三个访问器
**不在本 crate** —— 它们是 `alun-web` 提供的全局资源（`alun` 门面再导出）。

## 示例

```rust,no_run
use alun_cache::{Cache, LocalCache};

let cache = LocalCache::new("myapp", 10_000, 300); // 前缀、容量、默认 TTL（秒）
cache.start_background_cleanup();

cache.set("user:1", &"alice").await?;
assert_eq!(cache.get::<String>("user:1").await?, Some("alice".into()));

cache.set_ex("session:1", &"token", 60).await?;    // 显式 TTL
assert!(cache.exists("session:1").await?);

let keys = cache.keys("user:*").await?;
let removed = cache.delete_pattern("user:*").await?;
cache.del("session:1").await?;

let stats = cache.stats();
```

命名空间（多租户 / 多业务域的推荐姿势）：

```rust,no_run
use alun::prelude::*;   // 门面侧：cache() / cache_tenant()

let ns = cache().scoped("biz:order");      // 实际 key = "biz:order:hot"
ns.set("hot", &vec![1, 2, 3]).await?;
let v: Option<Vec<i32>> = ns.get("hot").await?;
```

按配置构造并注册为全局缓存：

```rust,ignore
use alun_cache::create_cache;
use alun_config::ConfigManager;

let cfg = ConfigManager::load(None);
let shared = create_cache(&cfg.get().app_name, &cfg.get().cache, &cfg.get().redis).await?;
alun_web::set_cache(shared).expect("cache already initialized");   // 需 alun-web 的 cache feature
```

## Feature

本 crate 不声明任何 cargo feature；Redis 后端始终编译在内。

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)
- 速查参考：`skill-alun/references/plugins.md`（缓存与插件一节）

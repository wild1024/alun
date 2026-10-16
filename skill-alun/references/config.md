# Configuration System

## Loading Order

1. `config/config.toml` — base config（不存在则整份用默认值 `AppConfig::default()`）
2. `config/config-{profile}.toml` — profile 覆盖（可选，**字段级**合并，见下）
3. `ALUN_*` 环境变量 — 最后覆盖（白名单键，见「CLI & Environment」）

profile 的选取顺序（`alun-config/src/env.rs:7` `detect_profile()`）：

1. 命令行参数 `profile=xxx`（注意是**裸参数**，`--profile=xxx` 不匹配）
2. `ALUN_PROFILE`
3. `ALUN_ENV`
4. 回退 `"dev"`

加载入口：`App::new()`（= `App::from_config_dir("config")`）内部调用
`ConfigManager::load(Some(dir))`（`alun-web/src/app.rs:98`），后者依次执行「base 文件 → profile 文件 →
env 覆盖」并把 `cfg.profile` 强制写成检测到的 profile（`alun-config/src/lib.rs:808`）。
需要跳过文件加载时用 `App::with_config(AppConfig)` 或 `App::with_config_manager(Arc<ConfigManager>)`
（`alun-web/src/app.rs:103,114`）。

### Profile 合并语义（重要）

`config-{profile}.toml` 不是整体替换，而是**逐字段判断「是否与默认值不同」**后覆盖 base
（`merge_configs` / `merge_database` / `merge_middleware`，`alun-config/src/lib.rs:896`）：

- 大部分标量：只有 profile 里的值 ≠ 编译期默认值时才覆盖（如 `listen`、`log.level`、`database.host`）。
- 布尔开关：只有 profile 里写成与默认值**不同**的那个方向才生效。例如
  `[middleware.auth] enabled` 默认 `false`，profile 里写 `enabled = true` 能打开、写 `enabled = false`
  覆盖不了 base 的 `true`（`merge_middleware` 用 `!= default` 比较）。
- 特例：`middleware.rate_limit` 只要任一字段与默认不同就**整块替换**；`plugins` 在
  `enabled` 非空时整块替换（数组无法字段级合并）；`custom` 按 key 合并（base 与 profile 求并集）。
- `static_files` 只在 `path` 与默认值不同时才整块覆盖（仅改 `enabled` 不生效）。

### AppConfig 顶层字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `app_name` | `String` | 应用名，默认 `"Alun"`（缓存键前缀、日志等使用） |
| `profile` | `String` | 当前 profile，默认 `"dev"`；加载时被检测结果覆盖 |
| `server` | `ServerConfig` | `listen: String`（默认 `"8023"`，可为 `"0.0.0.0:8023"`） |
| `log` | `LogConfig` | `level`（默认 `info`）、`format`（`text`/`json`）、`dir: Option<String>`、`file_prefix`（默认 `alun`） |
| `database` | `DatabaseConfig` | `enabled`（默认 `true`）、`type`（`postgres`/`mysql`/`sqlite`，默认 `postgres`）、`host`、`port: Option<u16>`、`name`、`user`、`password`、`password_encrypted`、`max_connections`、`min_connections`、`connect_timeout`、`sql_logging`、`slow_query_ms`、`ssl_mode`（`disable`/`prefer`/`require`/`verify-ca`/`verify-full`）、`migration` |
| `database.migration` | `MigrationConfig` | `enabled`、`path`（默认 `migrations`）、`auto_migrate`（`enabled && auto_migrate` 时启动阶段跑迁移）、`sources: Vec<String>`（非空时取代 `path`）、`seed_path`（种子目录，空 = 不跑）、`statement_split`（默认 `true`）、`up_down_sections`（默认 `true`）、`version_prefix`、`name_separator`、`repeatable_prefix`（默认 `R`）、`seed_prefix`（默认 `S`）、`baseline_prefix`（默认 `B`）（三者空串 = 关闭该种类）、`validate_checksum`（默认 `true`）、`divergent_policy`（`abort`/`warn`/`ignore`，默认 `abort`）、`out_of_order`、`transaction_default`（默认 `true`）、`import_legacy`、`baseline_on_migrate`、`baseline_version` |
| `redis` | `RedisConfig` | `enabled`、`url`（默认 `redis://127.0.0.1:6379`）、`max_connections` |
| `cache` | `CacheConfig` | `type`（`local`/`redis`，默认 `local`）、`max_capacity`（默认 `10000`）、`default_ttl`（秒，默认 `3600`，`0` = 永不过期） |
| `middleware` | `MiddlewareConfig` | `request_id`、`request_log`、`request_log_config`、`auth`、`cors`、`compression`、`rate_limit`、`security_headers`、`permission`、`idempotency`、`nonce` |
| `router` | `RouterConfig` | `prefix`（全局路由前缀）、`not_found`（`enabled` 默认 `true`、`message` 默认 `"请求的资源不存在"`） |
| `plugins` | `PluginsConfig` | `enabled: Vec<String>` 名单 + `notification` / `async_task` / `scheduler` / `serial` 各插件配置 |
| `upload` | `UploadConfig` | `path`（默认 `uploads`，启动时自动创建）、`max_size_mb`（默认 `10`；并据此推导请求体硬顶 = `max_size_mb MB + 32MiB`，框架自动装配 `DefaultBodyLimit`，业务无需自行挂载；业务侧前置校验请复用 `UploadConfig::max_request_body_bytes()`） |
| `download` | `DownloadConfig` | `path`（默认 `downloads`，启动时自动创建） |
| `template` | `TemplateConfig` | `path`（默认 `templates`） |
| `static_files` | `StaticConfig` | `path`（默认 `static`）、`enabled`（默认 `false`；开启后用 `ServeDir` 兜底） |
| `custom` | `HashMap<String, serde_json::Value>` | 自定义段，插件/业务自用（任意 TOML 值） |

启动时真正被消费的字段：`server.listen`（监听地址）、`router.prefix` / `router.not_found`、
`static_files`、`middleware.*`（中间件链，日志会打印一次摘要）、`database.*`（建池 + 可选迁移）、
`cache.*`/`redis.*`、`template.path`、`upload.path` / `upload.max_size_mb`（请求体硬顶）、`download.path`、`log.*`
（`alun-web/src/app.rs:550` `init_global_resources`、`build_middleware_chain`、`start`）。

## Global Access

```rust,no_run
use alun::prelude::*;

# fn demo() {
// Static config (immutable after load)
let port = cfg().server.listen.clone();
let db_type = cfg().database.r#type.clone();
let jwt_enabled = cfg().middleware.auth.enabled;

// Dynamic config (runtime read/write)
config().set_dynamic("rate_limit.requests_per_window", json!(200));
let limit: Option<i64> = config().get_dynamic("rate_limit.requests_per_window")
    .and_then(|v| v.as_i64());
config().remove_dynamic("temp.key");

// Custom section
let val = cfg().custom.get("my_app_key")
    .and_then(|v| v.as_str());

// Safe accessors (return Option — no panic)
let cm = try_config();          // Option<&Arc<ConfigManager>>
# let _ = (port, db_type, jwt_enabled, limit, val, cm);
# }
```

### ConfigManager API

`cfg()` = `config().get()`（`alun-web/src/resources.rs:105`），两者都从全局单例取。

| 方法 | 签名 | 说明 |
|------|------|------|
| `ConfigManager::load` | `load(dir: Option<&str>) -> Self` | 加载 base + profile 文件并应用 env 覆盖；`None` → `"config"` |
| `get` | `get(&self) -> &AppConfig` | 静态配置（进程内不可变），即 `cfg()` |
| `get_dynamic` | `get_dynamic(&self, key: &str) -> Option<serde_json::Value>` | 读动态配置 |
| `set_dynamic` | `set_dynamic(&self, key: &str, value: Value)` | 写/覆盖动态配置 |
| `remove_dynamic` | `remove_dynamic(&self, key: &str)` | 删除动态配置 |
| `generate_default` | `generate_default(dir: &str) -> std::io::Result<()>` | 把默认配置写到 `{dir}/config.toml`（会覆盖同名文件） |

公开字段：`static_config: AppConfig`、`dynamic: RwLock<HashMap<String, serde_json::Value>>`
（`alun-config/src/lib.rs:799`）。

> 动态配置是**进程内内存表**：key 是普通字符串（点号只是约定，没有层级解析），不落盘、不做文件
> 监听，重启即失效。`config.toml` 的改动也需要重启才生效（生成文件头部的注释即如此说明）。

### `[custom]` 自定义段

```toml
[custom]
my_app_key = "value"
feature_flags = { beta = true }
retry = 3
```

```rust,no_run
use alun::prelude::*;

# fn demo() {
let s = cfg().custom.get("my_app_key").and_then(|v| v.as_str());       // Option<&str>
let b = cfg().custom.get("feature_flags").and_then(|v| v.get("beta")).and_then(|v| v.as_bool());
let n = cfg().custom.get("retry").and_then(|v| v.as_u64());
# let _ = (s, b, n);
# }
```

## CLI & Environment

```bash
# Generate default config file
cargo run -- gen-config

# Print current resolved config
cargo run -- print-config

# Activate a profile
cargo run -- profile=prod
ALUN_PROFILE=prod cargo run

# Override config items via env vars
ALUN_SERVER_LISTEN=3000 cargo run
ALUN_LOG_LEVEL=debug cargo run
ALUN_DATABASE_HOST=10.0.0.1 cargo run
ALUN_DATABASE_SSL_MODE=require cargo run
```

- 参数由 `alun_config::env::parse_args()` 解析（`alun-config/src/env.rs:29`），识别的是**裸参数**
  `gen-config`（别名 `generate-config`）与 `print-config`；`--gen-config` 这种带横线的写法不匹配。
- **必须在 `App` 上调用 `.parse_cli()`**，否则 `App::new()` 里 `gen_config_only` / `print_config`
  都是 `false`，这些分支不生效（`alun-web/src/app.rs:139`；示例见
  `examples/00-quick-start/src/main.rs:117`）。
- `gen-config` 走完早退：只写 `config/config.toml` 就返回（`alun-web/src/app.rs:426`）；
  `print-config` 打印解析后的完整 TOML，但**不会**早退——服务随后照常初始化并启动。
- `App::serve("8080")` 可显式指定监听地址，覆盖 `server.listen`。

### `ALUN_*` 环境变量覆盖（白名单）

`merge_env_overrides` 只认下列键（`alun-config/src/env.rs:54`），**空字符串值会被忽略**；
未列入的 `ALUN_*` 变量被静默丢弃（`_ => {}`）——例如
`ALUN_DATABASE_MIGRATION_AUTO_MIGRATE=true` **不会**生效，迁移开关只能写进 TOML。

| 环境变量 | 覆盖字段 |
|----------|----------|
| `ALUN_SERVER_LISTEN` | `server.listen` |
| `ALUN_LOG_LEVEL` / `ALUN_LOG_FORMAT` / `ALUN_LOG_DIR` | `log.level` / `log.format` / `log.dir` |
| `ALUN_DATABASE_HOST` / `ALUN_DATABASE_NAME` / `ALUN_DATABASE_USER` / `ALUN_DATABASE_PASSWORD` | `database.*` |
| `ALUN_DATABASE_SSL_MODE` | `database.ssl_mode` |
| `ALUN_REDIS_URL` | `redis.url` |
| `ALUN_CACHE_TYPE` | `cache.type` |
| `ALUN_ROUTER_PREFIX` | `router.prefix` |
| `ALUN_UPLOAD_PATH` / `ALUN_DOWNLOAD_PATH` / `ALUN_TEMPLATE_PATH` | `upload.path` / `download.path` / `template.path` |
| `ALUN_MIDDLEWARE_AUTH_JWT_SECRET` | `middleware.auth.jwt_secret` |
| `FACAI_JWT_SECRET` | 同上（历史别名，优先级更高，单独处理） |
| `ALUN_PROFILE` / `ALUN_ENV` | profile 选取（由 `detect_profile` 读取，不走字段覆盖） |

## 启动钩子

在全局资源就绪、插件启动之前，执行自定义初始化：

```rust,no_run
use alun::prelude::*;

# async fn demo() -> Result<()> {
App::new()?
    .on_startup(|| async {
        // Initialize custom globals here
    })
    .scan()
    .start()
    .await
# }
```

钩子签名为 `FnOnce() -> Future<Output = ()> + Send + 'static`，只执行一次，无返回值、无错误通道
（`alun-web/src/app.rs:390`）。

## Testing Pattern

测试里通常不需要配置加载：直接用 `AlunRouter` 组路由，再用 axum 的 `oneshot` 发请求
（`App` 本身没有 `empty()`/`with_state()` 之类的无配置构造器；`AlunRouter::new()` 不依赖配置）。

```rust,no_run
use alun::AlunRouter;
use axum::Router;

# fn demo() -> Router {
let mut router = AlunRouter::new();
router.add_get("/health", || async { "OK" });
router.into_axum()
# }
```

需要 `App` 语义但不想读文件时，可 `App::with_config(AppConfig::default())`（会跳过文件与环境变量）。

## Plugin Configuration

`[plugins] enabled` 是**插件名单**：`App` 不会读取它，插件必须在代码里显式注册
（`App::plugin(..)`），或先用工厂函数按名单批量构造 `PluginManager`。

```toml
[plugins]
enabled = ["cache", "notification", "async-task", "scheduler", "serial"]

[plugins.serial]
backend = "memory"     # 见下方说明（当前不参与后端选择）
```

```rust,no_run
use alun::prelude::*;

# fn demo() -> Result<()> {
// 方式一：逐个注册（App 只认这一种显式注册；插件按拓扑序启动、逆序关闭）
let _app = App::new()?
    .plugin(alun::SerialPlugin::with_memory(cfg().plugins.serial.clone()));

// 方式二：按 `[plugins] enabled` 名单批量构造（返回 PluginManager，不返回 Result）
let _mgr = alun_plugin::create_plugins_from_config(cfg());
# Ok(())
# }
```

`create_plugins_from_config`（`alun-plugin/src/lib.rs:19`）认识的名字：
`notification`、`async-task`、`scheduler`、`cache`、`sid`、`serial`；未知名字只打一条
`warn!("未知插件: ..，跳过")`。各内置插件的 `Plugin::name()` 即上表名字（`async-task` 带横线）。

> `[plugins.serial] backend` 目前**不参与后端选择**：工厂函数一律用
> `SerialPlugin::with_memory(..)`，`SerialPlugin::start()` 也只使用预注入的生成器或内存后端
> （`alun-plugin/src/serial_plugin.rs:137`）。Redis / PostgreSQL 后端需自行实现
> `SerialGenerator` 并 `SerialPlugin::new(cfg.plugins.serial.clone(), backend)` 注入。

| `SerialRuleConfig` 字段 | 类型 | 默认值 | 说明 |
|-------------------------|------|--------|------|
| `key` | String | - | 规则唯一标识（必填） |
| `format` | String | - | 格式模板（必填），如 `ORD{YYYY}{MM}{DD}{SEQ:8}` |
| `cycle` | String | `"no_cycle"` | 循环周期：`no_cycle` / `daily` / `monthly` / `yearly`（其他值 → `NoCycle`） |
| `initial_value` | u64 | `1` | 计数器初始值 |
| `step` | String | `"sequential"` | 增量策略：`sequential` 或 `random:N`（如 `random:100`，解析失败按 `1` 处理） |

配置文件里的规则由插件在 `start()` 时转成 `SerialRule`（`is_enabled` 恒为 `true`）。

### 格式模板语法

| 占位符 | 输出 | 示例 |
|--------|------|------|
| `{YYYY}` | 4位年份 | 2026 |
| `{YY}` | 2位年份 | 26 |
| `{MM}` | 2位月份 | 06 |
| `{DD}` | 2位日期 | 11 |
| `{SEQ:N}` | N位补零序号 | `{SEQ:6}` → 000001 |
| `{RAND:N}` | N位随机数 | `{RAND:4}` → 5821 |
| `{TS}` | Unix时间戳（秒） | 1716537600 |
| `{TSMS}` | Unix时间戳（毫秒） | 1716537600123 |

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [认证与鉴权](auth.md) | [插件 · 缓存 · 模板](plugins.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

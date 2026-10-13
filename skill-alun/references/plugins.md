# Plugins, Caching & Templates

> **依赖声明**：插件设施只依赖门面 crate —— 在 `alun` 上开 feature 即可，**不需要**在
> `Cargo.toml` 单独声明 `alun-core` / `alun-plugin`（`alun::prelude` 已导出
> `Plugin` / `PluginManager` / `Result`，且转发整个 `alun_plugin` 模块为 `alun::alun_plugin`；
> `alun/src/lib.rs:1056-1059`、`:1101-1102`）。
>
> | feature | 内容 |
> |---------|------|
> | `plugin` | 插件基础设施（`alun_plugin`：scheduler / notification / async-task / cache / sid / serial） |
> | `task` | `alun_task`（Kafka 分布式任务，自带 rdkafka） |
> | `cache` / `template` | 全局缓存 / 模板引擎 |
>
> `full`（默认）已包含以上全部。

## Plugin Trait

```rust,no_run
use alun::prelude::*;
use async_trait::async_trait;

struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    fn name(&self) -> &str { "my-plugin" }
    async fn start(&self) -> Result<()> { Ok(()) }
    async fn stop(&self) -> Result<()> { Ok(()) }
    fn depends_on(&self) -> &[&str] { &["db"] }  // 依赖的插件名（对应对方的 name()）
}
```

trait 定义见 `alun-core/src/plugin.rs:32-54`：只有 `name()` / `async start()` /
`async stop()` / `depends_on()`（默认返回 `&[]`）四个方法。

### Plugin Lifecycle

- **start**: `PluginManager::start_all()` 先做 Kahn 拓扑排序，再按依赖顺序依次调用；
  **任一插件返回 `Err` 立即中止**，后续插件不会启动
  （`alun-core/src/plugin.rs:106-121`、`:135-185`）。
- **stop**: `PluginManager::stop_all()` 按**注册顺序的逆序**调用（不是拓扑序），
  某个插件失败只记错误日志、继续关闭其余插件
  （`alun-core/src/plugin.rs:123-133`）。
- **depends_on**: 只影响启动排序。依赖名与任何已注册插件的 `name()` 都不匹配时**静默忽略**
  （不报错）；存在循环依赖时 `start_all()` 返回 `Error::Config`，错误串里列出成环的插件名
  （`alun-core/src/plugin.rs:143-183`）。
- 启动前会 `check_duplicate_names()`：插件名重复 → `Error::Config`，`App::start()` 就此失败
  （`alun-core/src/plugin.rs:186-195`、`alun-web/src/app.rs:457-460`）。
- `App` 生命周期：`App::start()` 先初始化全局资源 → 自定义 startup 钩子 → 插件
  `start_all()`；`axum::serve` 优雅退出后调用 `stop_all()`（`alun-web/src/app.rs:452-460`、`:518`）。

## Built-in Plugins

用 `App::plugin(..)` 注册（每个 `App` 只能注册一次同名插件，重复名会在启动时报错）：

```rust,ignore
App::new()?
    .plugin(SomePlugin::new(/* ... */))
    .scan()
    .start()
    .await
```

> `[plugins] enabled = [...]` **不会被 `App` 自动消费**——`App::start()` 只启动你显式传入
> `App::plugin(..)` 的插件。`alun::alun_plugin::create_plugins_from_config(&cfg())`
> 可按配置列表构造一个 `PluginManager`（`alun-plugin/src/lib.rs:19-68`），但工作区内没有
> 调用点，需要自行 `start_all()` / `stop_all()`，与 `App` 的插件生命周期无关。

内置插件与 `name()`（`alun-plugin/src/lib.rs:24-63`）：

| 插件 | `name()` | 配置段 |
|------|----------|--------|
| `CachePlugin` | `cache` | `[cache]` / `[redis]` |
| `SchedulerPlugin` | `scheduler` | `[plugins.scheduler]` |
| `NotificationPlugin` | `notification` | `[plugins.notification]` |
| `AsyncTaskPlugin` | `async-task` | `[plugins.async_task]` |
| `SerialPlugin` | `serial` | `[plugins.serial]` |
| `SidPlugin` | `sid` | 无 |

**CachePlugin**：管理一个独立的缓存实例——`start()` 时按 `[cache]`/`[redis]` 创建
`SharedCache`（Local 或 Redis），`stop()` 释放；`plugin.cache() -> Option<SharedCache>`
（`alun-plugin/src/cache_plugin.rs:33-68`）。注意它是**独立实例**：框架的全局单例
`cache()` / `try_cache()` 由 `App::init_global_resources` 初始化
（`alun-web/src/app.rs:601-605`），不由 CachePlugin 写入。

**SchedulerPlugin**（真调度：解析 cron + 到点执行）：
```rust,ignore
use alun::alun_plugin::scheduler::SchedulerPlugin;

let scheduler = SchedulerPlugin::new();       // 无参数；with_clock(clock) 可注入测试时钟
scheduler.register(
    "cleanup",
    "0 */2 * * *",                            // Cron: 每 2 小时
    "清理临时文件",
    || tokio::spawn(async { /* 任务体 */ }),   // runner 必须返回 JoinHandle<()>
);
let handle = scheduler.trigger("cleanup");     // Option<JoinHandle<()>>，同步调用；未注册/未绑定 → None（不 panic）
let jobs = scheduler.list();                   // Vec<(名称, cron, 描述)>
scheduler.remove("cleanup");                   // 运行中会停止其调度循环

// 只登记表达式、稍后绑定处理函数
scheduler.register_scheduled("report", "0 0 3 * * *", "日报");
scheduler.attach("report", || tokio::spawn(async { /* ... */ }));   // bool：名字不存在 → false

// 真调度：start() 编译全部表达式并为每个注册项起调度循环，到点调用 runner
Plugin::start(&scheduler).await?;              // 非法 cron → Err(Config)，且不半启动任何任务
scheduler.is_running();                        // 运行态
Plugin::stop(&scheduler).await?;               // 优雅停止（async、幂等；Drop 亦会停）
```

- API：`new()` / `with_clock(clock)` / `register` / `register_scheduled` / `attach` /
  `remove` / `list` / `trigger` / `is_running`（`alun-plugin/src/scheduler.rs:317-401`）。
- `start()` 先整体解析全部表达式，任一非法即 `Err(Config)` 且**不启动任何**调度循环；
  解析成功但 `next_after()` 无解的项（如 `0 0 30 2 *`）只告警并跳过
  （`alun-plugin/src/scheduler.rs:409-452`）。
- 未绑定处理函数的注册项**仍按期触发**，每次触发只记 `warn!("未绑定处理函数，跳过本次触发")`
  （`alun-plugin/src/scheduler.rs:480-519`）。
- `stop()` 发 `watch` 信号并 await 所有调度循环退出；`Drop` 也会发信号，与外部关闭流程解耦
  （`alun-plugin/src/scheduler.rs:457-478`、`:521-530`）。

cron 支持 **5 段**（分 时 日 月 周）与 **6 段**（秒 分 时 日 月 周），每段支持
`*`、`a`、`a,b`、`a-b`、`*/n`、`a-b/n`（另容忍 `a/n` ≡ `a-max/n`）；周字段 `0`/`7` 均为周日，
日/周同时受限时取“或”（标准 cron 语义）。纯解析/推算可单独用
`CronSchedule::parse(expr)?` 与 `next_after(t)`（含秒级）
（`alun-plugin/src/scheduler.rs:103-138`、`:142-178`、`:192-202`）。

> `[plugins.scheduler] workers` 配置字段存在（默认 4），但 `SchedulerPlugin::new()` 不读取它
> ——调度器没有 worker 池，每个注册项一个 `tokio` 调度循环
> （`alun-config/src/lib.rs:602-606`、`alun-plugin/src/scheduler.rs:317-326`）。

**NotificationPlugin** (SMTP)：`send_text` / `send_html` 是**同步**方法（内部用 lettre 阻塞发送）。
若 `enabled = false` 或 `smtp_host` / `smtp_user` 为空，则降级为“跳过”模式：只记日志、返回
`Ok(())`，不报错（`alun-plugin/src/notification.rs:32-64`、`:67-117`）。

```rust,ignore
use alun::alun_plugin::notification::NotificationPlugin;

let notif = NotificationPlugin::from_config(&cfg().plugins.notification);
notif.send_text("admin@example.com", "Alert", "Disk usage > 90%")?;
notif.send_html("user@example.com", "验证码", "<h1>Your Code</h1>")?;
let configured = notif.is_configured();
```

**AsyncTaskPlugin** (semaphore-based)：
```rust,ignore
use alun::alun_plugin::async_task::AsyncTaskPlugin;

let pool = AsyncTaskPlugin::new(4);           // worker 数；传 0 自动取 4
pool.submit(async { heavy_work().await; }).await;   // 立即 tokio::spawn，无需先 start()
let n = pool.worker_count();
```
`submit()` 直接 spawn 并由 semaphore 限流；`stop()` 会 `acquire_many(workers)` 等待所有
在跑任务结束（`alun-plugin/src/async_task.rs:24-65`）。

**SidPlugin**：封装 `alun_utils::Sid`，提供 `uuid()` / `uuid7()` / `short()` / `tiny()` /
`tsid()` / `biz_id(prefix)`（`alun-plugin/src/sid_plugin.rs:19-38`）。

**SerialPlugin** (serial number generator):

遵循 `TaskPlugin` 薄封装模式——插件不持有数据库连接或 SQL，持久化逻辑通过
`Arc<dyn SerialGenerator>` 委托给业务方。

```toml
# config.toml
[plugins.serial]
backend = "memory"           # memory | redis | postgres（仅用于日志，见下方说明）

[[plugins.serial.rules]]
key = "order"
format = "ORD{YYYY}{MM}{DD}{SEQ:8}"
cycle = "daily"              # no_cycle | daily | monthly | yearly
initial_value = 1
step = "sequential"          # sequential | random:N
```

> 注意路径：`SerialPlugin` 在子模块 `alun_plugin::serial_plugin` 下，`alun_plugin` 根**没有**
> 再导出；门面侧可用 `alun::SerialPlugin`（需 `plugin` feature，`alun/src/lib.rs:1123-1124`）。
> `SerialRule` / `SerialGenerator` / `SerialError` / `CyclePeriod` / `IncrementStrategy` /
> `MemorySerialBackend` 由 `alun` 根导出（`alun/src/lib.rs:1114-1117`）。

```rust,no_run
use alun::{SerialError, SerialGenerator, SerialPlugin};
use std::sync::Arc;

# fn setup(cfg: &alun::AppConfig) -> Result<SerialPlugin, Box<dyn std::error::Error>> {
// 方式一：零配置 Memory 后端（start() 时自动创建）
let plugin = SerialPlugin::with_memory(cfg.plugins.serial.clone());

// 方式二：自定义后端（Redis/PG/业务自定义）
struct MyRedisSerialBackend;
# #[async_trait::async_trait]
# impl SerialGenerator for MyRedisSerialBackend {
#     async fn generate(&self, _k: &str) -> std::result::Result<String, SerialError> { unimplemented!() }
#     async fn batch_generate(&self, _k: &str, _c: u32) -> std::result::Result<Vec<String>, SerialError> { unimplemented!() }
#     async fn peek(&self, _k: &str) -> std::result::Result<String, SerialError> { unimplemented!() }
#     async fn register_rule(&self, _rule: alun::SerialRule) -> std::result::Result<(), SerialError> { unimplemented!() }
#     async fn remove_rule(&self, _k: &str) -> std::result::Result<(), SerialError> { unimplemented!() }
#     async fn enable_rule(&self, _k: &str) -> std::result::Result<(), SerialError> { unimplemented!() }
#     async fn disable_rule(&self, _k: &str) -> std::result::Result<(), SerialError> { unimplemented!() }
#     async fn query_records(&self, _k: &str, _p: u64, _s: u64) -> std::result::Result<(Vec<alun::SerialRecord>, u64), SerialError> { unimplemented!() }
#     async fn list_rules(&self) -> std::result::Result<Vec<alun::SerialRule>, SerialError> { unimplemented!() }
# }
let backend: Arc<dyn SerialGenerator> = Arc::new(MyRedisSerialBackend);
let plugin = SerialPlugin::new(cfg.plugins.serial.clone(), backend);

// 取回生成器（需 plugin 已 start；持有插件句柄一方调用）
let generator = plugin.generator();
let _ = generator;
Ok(plugin)
# }
```

`start()` 的行为（`alun-plugin/src/serial_plugin.rs:135-172`）：

- 使用 `new()` 注入的生成器；没注入就用 `MemorySerialBackend::new()`。
- 把 `[plugins.serial.rules]` 逐条转换为 `SerialRule` 并 `register_rule(..)`；
  `cycle` 不匹配 `daily`/`monthly`/`yearly` 时取 `CyclePeriod::NoCycle`；
  `step = "random:N"` → `IncrementStrategy::Random { max: N }`，否则
  `IncrementStrategy::Sequential`；`is_enabled` 恒为 `true`。
- **`backend` 字段不参与后端选择**（只写进日志）：实际后端永远是注入的生成器或 memory。

```rust,ignore
#[alun::get("/api/serial/next/{rule_key}")]
async fn next_serial(Path(rule_key): Path<String>) -> ResResult<String> {
    let generator = plugin.generator();                       // start() 之后可用，否则 panic
    let no = generator.generate(&rule_key).await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(no))
}
```

在 Handler 里生成单号：`App::plugin()` 不会把插件自动注入 axum `Extension`，
需自行持有 `SerialPlugin` 句柄（或它的 `Arc<dyn SerialGenerator>`），例如放进自定义
state / `OnceLock`。

| `SerialPlugin` 方法 | 说明 |
|---------------------|------|
| `SerialPlugin::new(config, generator)` | 使用自定义生成器创建（推荐） |
| `SerialPlugin::with_memory(config)` | 零配置内存后端（start 时创建） |
| `plugin.generator()` | 获取 `Arc<dyn SerialGenerator>`；**未 start 时 panic** |

## Registering Custom Plugins

```rust,no_run
use alun::prelude::*;

// `#[alun::plugin]` 只给类型加一个固有方法 `alun_plugin_name()`（暴露类型名），
// 不做任何注册/发现（`alun-macros/src/plugin.rs:6-21`）；
// 真正的注册仍由 `App::plugin()` 完成。
#[alun::plugin]
struct MyCustomPlugin;

#[async_trait::async_trait]
impl Plugin for MyCustomPlugin {
    fn name(&self) -> &str { "my-custom" }
    async fn start(&self) -> Result<()> {
        tracing::info!("Custom plugin started");
        Ok(())
    }
    async fn stop(&self) -> Result<()> { Ok(()) }
    fn depends_on(&self) -> &[&str] { &["db"] }
}

# async fn demo() -> Result<()> {
App::new()?
    .plugin(MyCustomPlugin)
    .scan()
    .start()
    .await
# }
```

> `PluginManager::add_discovered(Vec<Box<dyn Plugin>>)` 是为“编译期发现的插件批量注册”
> 预留的入口（`alun-core/src/plugin.rs:98-101`），但目前**没有任何代码**由 `#[plugin]`
> 宏生成这样的向量（没有分布式切片），所以必须逐个 `App::plugin(..)`。

---

## Caching

Requires `features = ["cache"]`。

### Global Access

```rust,no_run
use alun::prelude::*;

# async fn demo() -> Result<()> {
// 未初始化时 panic
cache().set("key", "value").await?;
cache().set_ex("key", "value", 300).await?;      // 300 秒 TTL
let val: Option<String> = cache().get("key").await?;
let exists: bool = cache().exists("key").await?;
cache().del("key").await?;

// Counter
let count: i64 = cache().incr("api_calls", 1).await?;

// Pattern matching
let keys: Vec<String> = cache().keys("user:*").await?;
let deleted: u64 = cache().delete_pattern("temp:*").await?;

// Stats（同步方法，返回 CacheStats）
let stats = cache().stats();

// Safe accessor
let safe = try_cache();          // Option<&SharedCache>
# let _ = (val, exists, count, keys, deleted, stats, safe); Ok(())
# }
```

`Cache` trait 的完整方法集（`alun-cache/src/lib.rs:32-56`）：
`get` / `set` / `set_ex` / `del` / `exists` / `incr` / `keys` / `delete_pattern` / `stats`。
`set` / `set_ex` 接收 `&T`（`T: Serialize`），`get` 按目标类型反序列化。

### Standalone Usage

```rust,no_run
use alun::prelude::*;                                  // Cache / LocalCache / SharedCache

# async fn demo() -> alun::Result<()> {
let local = LocalCache::new("my-app", 10000, 3600);   // app前缀, 容量10000, 默认TTL 3600s
local.set("key", "value").await?;

// From config（`create_cache` 未被门面转发，需直接声明 alun-cache 依赖）
let cache = alun_cache::create_cache("my-app", &cfg().cache, &cfg().redis).await?;
# let _ = cache;
# Ok(())
# }
```

### Cache Types

`SharedCache::Local(LocalCache)` — in-memory, suitable for single-instance.
`SharedCache::Redis(RedisCache)` — distributed via Redis.

（两个变体直接持有值；`LocalCache`/`RedisCache` 内部均可 `Clone`，`RedisCache` 持 `ConnectionManager`。）

---

## Template Engine

Requires `features = ["template"]`。Uses minijinja (Jinja2 syntax)。

### Global Access

```rust,no_run
use alun::prelude::*;

# fn demo() -> Result<String> {
let html = render_template("page.html", &json!({"title": "Home", "items": []}))
    .map_err(|e| ApiError::internal(e.to_string()))?;

// Safe accessor
let engine = try_template();     // Option<&TemplateEngine>
# let _ = engine;
Ok(html)
# }
```

### Standalone Usage

```rust,no_run
use alun::prelude::*;            // TemplateEngine（需 template feature）

# fn demo() -> alun::Result<()> {
let engine = TemplateEngine::from_dir("templates")?;
let html = engine.render("index.html", &json!({"title": "Alun"}))?;      // From file
let result = engine.render_str("Hello, {{ name }}!", &json!({"name": "World"}))?;  // From string
# let _ = (html, result);
# Ok(())
# }
```

Template files go in `templates/` directory by default.

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [配置系统](config.md) | [启动钩子 / 自定义中间件 / WebPlugin / 异步 Hook](extension-points.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

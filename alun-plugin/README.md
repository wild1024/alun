# alun-plugin

Alun 内置插件集：邮件通知、缓存引导、进程内异步任务池、cron 调度、单号、短 ID。

> 每个插件类型都在自己的子模块里 —— **crate 根没有再导出**，请用完整路径
> （如 `alun_plugin::serial_plugin::SerialPlugin`）。它们都实现 `alun_core::Plugin`，
> 因此既可以由 `PluginManager` 统一管理，也可以直接 `App::plugin(..)` 挂到应用上。

门面侧只依赖 `alun` 时，插件类型从 `alun::prelude::alun_plugin::{…}` 取；单号插件另有门面别名
`alun::SerialPlugin`。

## 模块

### `serial_plugin` —— 单号

- `SerialPlugin` —— `with_memory(config: SerialConfig)`、`new(config, Arc<dyn SerialGenerator>)`、
  `generator() -> Arc<dyn SerialGenerator>`（插件未 start 前调用会 panic）；名字 `"serial"`

### `notification` —— 邮件 / SMTP（lettre）

- `NotificationPlugin` —— `from_config(&NotificationConfig)`、`send_text(to, subject, body)`、
  `send_html(to, subject, html_body)`、`is_configured()`；名字 `"notification"`

### `scheduler` —— cron 风格定时任务（解析 + 执行）

- `SchedulerPlugin` —— `new()`、`with_clock(clock)`、`register(name, cron, desc, runner)`、
  `register_scheduled(name, cron, desc)`、`attach(name, runner) -> bool`、`remove(name)`、
  `list() -> Vec<(String, String, String)>`、`trigger(name)`、`is_running()`；名字 `"scheduler"`
- `CronSchedule` —— `parse(expr) -> Result<CronSchedule, CronError>`、`expression()`、
  `next_after(after: DateTime<Utc>) -> Option<DateTime<Utc>>`
- `CronError` —— `expression()`、`message()`
- `ClockFn` —— `Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>`

`start()` 会编译所有注册项并为每个任务起一个调度循环；cron 表达式非法时 `start()` 直接失败
（带上任务名），不会先跑起来再炸。`stop()`（以及 `Drop`）会通知全部循环并等待退出，任何关闭路径
调用都安全。Cron 语法支持 5 段（`分 时 日 月 周`）或 6 段（前导秒），每字段支持 `*`、`a`、
`a,b`、`a-b`、`*/n`、`a-b/n`；日与周同时限定时按标准「或」语义，`7` 代表周日。
只注册不带处理器的任务（`register_scheduled`）不会 panic —— 到点只会打一条警告日志。

### `cache_plugin` —— 缓存引导

- `CachePlugin` —— `new(app_name, &CacheConfig, &RedisConfig)`、`cache() -> Option<SharedCache>`；名字 `"cache"`

### `async_task` —— 进程内任务池

- `AsyncTaskPlugin` —— `new(workers)`、`async submit(future)`、`worker_count()`；名字 `"async-task"`

### `sid_plugin` —— ID 生成

- `SidPlugin` —— `new()`、`uuid()`、`short()`、`tiny()`、`uuid7()`、`tsid()`、`biz_id(prefix)`；名字 `"sid"`

### 工厂

- `create_plugins_from_config(&AppConfig) -> PluginManager` —— 按 `config.plugins.enabled`
  构造插件管理器（脱离 `App` 的场景用）

## 示例

```rust,ignore
use alun_config::{CacheConfig, RedisConfig, SerialConfig};
use alun_core::PluginManager;
use alun_plugin::cache_plugin::CachePlugin;
use alun_plugin::serial_plugin::SerialPlugin;
use alun_plugin::sid_plugin::SidPlugin;

let manager = PluginManager::new()
    .add(SidPlugin::new())
    .add(CachePlugin::new("myapp", &CacheConfig::default(), &RedisConfig::default()))
    .add(SerialPlugin::with_memory(SerialConfig::default()));

manager.start_all().await?;
```

按配置建齐所有插件：

```rust,ignore
use alun_config::ConfigManager;
use alun_plugin::create_plugins_from_config;

let cm = ConfigManager::load(None);
let manager = create_plugins_from_config(cm.get());
manager.start_all().await?;
```

挂到应用上（这样随应用启停）：

```rust,ignore
use alun::prelude::*;

App::new()?
    .plugin(SidPlugin::new())
    .start()
    .await?;
```

> ⚠️ `[plugins] enabled = [...]` 只是**名单**，框架不会自动读取它 —— 需要自己
> `.plugin(..)` 注册（或 `create_plugins_from_config` 拿 `PluginManager` 后交给 `App`）。

## Feature

本 crate 不声明任何 cargo feature；SMTP 后端用 `lettre` + rustls。

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)
- 速查参考：`skill-alun/references/plugins.md`、`skill-alun/references/extension-points.md`

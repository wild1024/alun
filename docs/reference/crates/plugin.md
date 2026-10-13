---
id: reference.crates.plugin
title: `alun-plugin` — 内置插件
summary: `alun-plugin` — 内置插件——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [plugin, reference, scheduler, serial, sid]
status: active
owner: alun-core-team
source_of_truth: [alun-plugin/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-plugin` — 内置插件

> 本文件是 [`crates.md`](../crates.md) 的 `plugin` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun-plugin/src/`

## `CachePlugin` （[cache\_plugin.rs](../../../alun-plugin/src/cache_plugin.rs)）

缓存生命周期管理器。`start()` 创建 LocalCache/RedisCache，`stop()` 释放。

- `CachePlugin::new(app_name, &cache_config, &redis_config)`
- `plugin.cache() -> Option<SharedCache>` — 取共享缓存实例（未启动时为 `None`）

## `NotificationPlugin` （[notification.rs](../../../alun-plugin/src/notification.rs)）

SMTP 邮件通知（基于 lettre），支持纯文本和 HTML 格式。根据发件域名自动选择传输方式——`@icloud.com` / `@swisscows.email` 走 `STARTTLS`，其余走 `RELAY`。

| 方法                                       | 说明                          |
| ---------------------------------------- | --------------------------- |
| `NotificationPlugin::from_config(&cfg)`  | 从 `NotificationConfig` 构建    |
| `send_text(to, subject, body)`           | 发送纯文本邮件（**同步**，返回 `Result<()>`） |
| `send_html(to, subject, html_body)`      | 发送 HTML 邮件（内部自动生成纯文本备用版本）   |
| `is_configured()`                        | 检查邮件功能是否已配置                 |

## `AsyncTaskPlugin` （[async\_task.rs](../../../alun-plugin/src/async_task.rs)）

基于 Semaphore 的后台任务队列。`AsyncTaskPlugin::new(workers)` 指定 worker 数；`submit(future)`（async）提交任务，`worker_count()` 查询并发度，`stop()` 等待所有任务完成。

## `SchedulerPlugin` （[scheduler.rs](../../../alun-plugin/src/scheduler.rs)）

**真调度**（D027 修复后）：`start()` 编译全部 cron 表达式并为每个注册项起调度循环，到点调用处理函数；
`stop()`（async、幂等）发送关闭信号并等待循环退出，`Drop` 亦会停止。实现 `Default`。

| 方法                                                                  | 说明                                             |
| ------------------------------------------------------------------- | ---------------------------------------------- |
| `SchedulerPlugin::new()` / `with_clock(ClockFn)`                     | 创建（`with_clock` 可注入时钟，便于测试）                      |
| `register(name, cron, desc, runner)`                                | 注册任务；`runner: Fn() -> JoinHandle<()> + Send + Sync`（**返回 JoinHandle**，需自行 `tokio::spawn`） |
| `register_scheduled(name, cron, desc)` / `attach(name, runner) -> bool` | 只登记表达式 / 事后补绑处理函数                              |
| `remove(name)`                                                      | 注销（该任务即时停止，不再触发）                              |
| `list() -> Vec<(String, String, String)>`                           | 列出 `(名称, cron, 描述)`                             |
| `trigger(name) -> Option<JoinHandle<()>>`                           | 手动触发一次；未注册或未绑定处理函数返回 `None`（不 panic）          |
| `is_running()` / `start()` / `stop()`                               | 运行态与生命周期（`stop` 为 async 且幂等）                   |

cron 支持 **5 段**（分 时 日 月 周）与 **6 段**（秒 分 时 日 月 周）；每段支持 `*`、`a`、`a,b`、`a-b`、`*/n`、`a-b/n`，周字段 `0`/`7` 均为周日；非法表达式在 `start()` 返回 `Err(Config)` 且**不半启动**任何任务。
也可单独使用 `CronSchedule::parse(expr)?` / `next_after(t)` / `expression()`。

## `SerialPlugin` （[serial\_plugin.rs](../../../alun-plugin/src/serial_plugin.rs)，**仅子模块路径**）

单号生成插件（需 `plugin` feature，门面再导出为 `alun::SerialPlugin`）。

```rust,ignore
pub struct SerialPlugin;
impl SerialPlugin {
    pub fn new(config: alun_config::SerialConfig, generator: Arc<dyn alun_utils::SerialGenerator>) -> Self;
    pub fn with_memory(config: alun_config::SerialConfig) -> Self;   // 内置 MemorySerialBackend
    pub fn generator(&self) -> Arc<dyn alun_utils::SerialGenerator>; // 未启动时 panic
}
```

## `SidPlugin` （[sid\_plugin.rs](../../../alun-plugin/src/sid_plugin.rs)）

ID 生成插件（实现 `Default`）：`new()`、`uuid()`、`short()`、`tiny()`、`uuid7()`、`tsid()`、`biz_id(prefix)`。

> `alun-plugin` 的 crate 根**没有**再导出以上类型，路径是 `alun_plugin::<模块>::<类型>`（如 `alun_plugin::cache_plugin::CachePlugin`）；唯一例外是 `alun` 门面导出的 `SerialPlugin`。

## `create_plugins_from_config(config)` （[lib.rs](../../../alun-plugin/src/lib.rs)）

```rust,ignore
pub fn create_plugins_from_config(config: &alun_config::AppConfig) -> alun_core::PluginManager;
```

工厂函数——根据 `AppConfig.plugins.enabled` 列表创建所有启用的插件并返回 `PluginManager`（**不返回 `Result`**）。

## 使用示例

```rust,ignore
use alun_plugin::*;                       // create_plugins_from_config
use alun_plugin::scheduler::SchedulerPlugin;
use tokio::task::JoinHandle;

// ── 配置驱动加载（返回 PluginManager，不是 Result） ──
// config.toml:
// [plugins]
// enabled = ["cache", "notification", "async-task", "scheduler"]
let plugin_manager = create_plugins_from_config(&config);

// ── 缓存插件（自动生命周期） ──
// App 启动时 CachePlugin 自动创建 SharedCache，注入全局资源
// Handler 中直接使用 cache() 获取

// ── 定时任务（runner 返回 JoinHandle，不是 future） ──
let scheduler = SchedulerPlugin::new();

scheduler.register(
    "clean_temp_files",                     // 任务名
    "0 */2 * * *",                          // cron: 每2小时
    "清理临时文件",                           // 描述
    || tokio::spawn(async { clean_temp_files().await; }),
);

scheduler.register(
    "daily_report",
    "0 8 * * *",                            // 每天早上8点
    "日报生成",
    || tokio::spawn(async { generate_daily_report().await; }),
);

let jobs: Vec<(String, String, String)> = scheduler.list();   // (名称, cron, 描述)
let _: Option<JoinHandle<()>> = scheduler.trigger("clean_temp_files");   // 手动触发

// ── 邮件通知（同步方法，无 .await） ──
use alun_plugin::notification::NotificationPlugin;
let notif = NotificationPlugin::from_config(&config.notification);
notif.send_text("admin@example.com", "告警", "磁盘使用率 > 90%")?;
notif.send_html("user@example.com", "验证码", "<h1>您的验证码</h1><p><b>123456</b></p>")?;

// ── 异步任务队列 ──
use alun_plugin::async_task::AsyncTaskPlugin;
let task_pool = AsyncTaskPlugin::new(4);                   // 4 个 worker
task_pool.submit(async { heavy_computation().await; }).await;
task_pool.submit(async { send_batch_emails().await; }).await;
task_pool.worker_count();                                  // → 4
// stop() 时等待所有任务完成

// ── 单号生成插件 ──
use alun_plugin::serial_plugin::SerialPlugin;
let serial = SerialPlugin::with_memory(config.plugins.serial.clone());
let generator = serial.generator();                        // Arc<dyn SerialGenerator>

// ── 自定义插件注册 ──
#[alun::plugin]
struct MetricsPlugin;
#[async_trait]
impl Plugin for MetricsPlugin {
    fn name(&self) -> &str { "metrics" }
    async fn start(&self) -> Result<()> {
        tracing::info!("指标收集已启动");
        Ok(())
    }
    async fn stop(&self) -> Result<()> { Ok(()) }
}

App::new()?.plugin(MetricsPlugin).serve("8080").await?;
```

***

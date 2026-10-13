---
id: guides.startup.hooks-plugins
title: 按需装配：启动钩子与插件
summary: on_startup 什么时候用、插件什么时候值得写、以及三者（中间件/钩子/插件）的关系总结。
tags: [getting-started, hooks, plugin, assembly]
status: active
owner: alun-core-team
source_of_truth: [code, examples, config/config.toml]
depends_on: [guides.getting-started]
last_reviewed: 2026-10-06
---

# 按需装配：启动钩子与插件

> 本文件是 [`getting-started.md`](../getting-started.md) 的 `hooks-plugins` 分册；总目录与全流程顺序见 [从零到跑起来](../getting-started.md)。

## 7.3 启动钩子 `on_startup`：什么时候用

**时机**：全局资源（配置、数据库、缓存、模板）已就绪，**插件还没启动**。

**判断标准**：这段初始化「只需要跑一次」「依赖框架已初始化的资源」「没有优雅关闭的需求」——
那就用钩子，别写插件。

```rust,no_run
use alun::prelude::*;
use std::sync::OnceLock;

/// 自定义全局单例（存第三方 SDK 客户端、共享配置等）。
static PAY_BASE_URL: OnceLock<String> = OnceLock::new();

#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        .on_startup(|| async {
            // 此时 cfg() / db() / cache() 都可用
            let base = cfg()
                .custom
                .get("payment")
                .and_then(|v| v.get("base_url"))
                .and_then(|v| v.as_str())
                .unwrap_or("https://pay.example.com")
                .to_string();
            let _ = PAY_BASE_URL.set(base);
        })
        .on_startup(|| async {
            // 可以再注册一个：按注册顺序依次执行
            tracing::info!("支付网关已就绪");
        })
        .scan()
        .start()
        .await
}
```

**可注册多个**：`on_startup` 是**追加**而不是覆盖（内部是 `Vec`，源码注释 `// 追加而非覆盖：
多个模块各自注册启动钩子时互不影响`），启动时按**注册顺序**依次 `await`，日志会打印
`执行 N 个自定义启动钩子（按注册顺序）`。所以「给每个模块各挂一个」现在是正当用法，
不必把所有初始化塞进同一个闭包里。

钩子的闭包签名是 `FnOnce() -> impl Future<Output = ()>`，**返回 `()`** ——
初始化失败请在钩子内部自行处理（记录日志 / `panic`），框架不会因为钩子返回错误而中止启动。

**实战提问：`once_cell` / `OnceLock` / 全局变量到底该在哪个阶段初始化？**
统一放 `on_startup`。原因：`main()` 里做会在 `App::new()` 之前，此时 `cfg()` 还没有配置可用；
写进插件又过度设计（不需要 stop）。

**实战提问：钩子、中间件、插件的执行顺序是？**
配置加载 → 全局资源（DB / 缓存 / 模板）→ **启动钩子（按注册顺序）** → 插件 `start_all()`
（按依赖正序）→ 监听端口。详见 [4.x 启动时到底做了什么](config-basics.md)。

## 7.4 插件：什么时候值得写

用这三条判断，**同时满足才写插件**：

1. 有明确的 `start` / `stop` 生命周期（要优雅关闭、要释放后台任务）；
2. 启动顺序重要（依赖数据库先就绪、依赖别的插件先启动）；
3. 想被复用到多个项目。

只要「启动时初始化一下」，用 [启动钩子](hooks-plugins.md) 就够了。

**内置插件清单**（`[plugins] enabled` 里写名字）：

| 插件名 | 配置段 | 用途 | 什么时候用 |
| ---- | ---- | ---- | ---- |
| `scheduler` | `[plugins.scheduler]` | cron 定时任务调度 | 定时清理、定时对账 |
| `notification` | `[plugins.notification]` | SMTP 邮件通知 | 发验证码、告警邮件 |
| `async-task` | `[plugins.async_task]` | 进程内线程池异步任务 | 请求里不想阻塞的轻量耗时操作 |
| `cache` | `[cache]` / `[redis]` | 缓存资源 | 显式把缓存当插件管理 |
| `serial` | `[plugins.serial]` | 单号生成（`memory` / `redis` / `postgres` 后端） | 订单号、流水号 |
| `sid` | — | 短 ID 生成 | 需要短 ID |

**⚠️ 一个非常关键的事实**：`[plugins] enabled` 只是**插件名单**，
`alun-plugin::create_plugins_from_config(&cfg)` 会按这个名单造出 `PluginManager`；
但 `App` **不会**自动读取它 —— 你必须自己在启动时把插件交给 `App`：

```toml
[plugins]
enabled = ["notification", "serial"]

[plugins.notification]
smtp_host = "smtp.example.com"
smtp_port = 587
smtp_user = "no-reply@example.com"
smtp_pass = ""
from_email = ""
from_name = "系统通知"

[plugins.scheduler]
workers = 4

[plugins.serial]
backend = "memory"           # memory | redis | postgres
```

```rust,no_run
use alun::prelude::*;
use alun_plugin::scheduler::SchedulerPlugin;

#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        // 内置插件：从配置构造后显式注册（配置怎么写，构造就怎么读）
        .plugin(SchedulerPlugin::new())
        .scan()
        .start()
        .await
}
```

**自己写一个插件**：

```rust,no_run
use alun::prelude::*;

/// 示例插件：`start` 里做一次性初始化，`stop` 里释放资源。
#[alun::plugin]
struct MyPlugin;

#[async_trait::async_trait]
impl Plugin for MyPlugin {
    fn name(&self) -> &str {
        "my-plugin"
    }

    async fn start(&self) -> Result<()> {
        // 此时全局资源已就绪，可以访问 db() / cache()
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        // 优雅关闭：停止后台任务、刷缓冲
        Ok(())
    }

    /// 声明依赖的插件名，用于拓扑排序（依赖不存在时会被忽略，不会报错）。
    fn depends_on(&self) -> &[&str] {
        &[]
    }
}

# async fn demo() -> Result<()> {
App::new()?.plugin(MyPlugin).scan().start().await
# }
```

插件规则速记：**名字必须唯一**（重名直接启动失败）；`depends_on` 形成循环会报
`插件循环依赖`；`stop` 按启动的**逆序**执行，且某个插件 stop 失败不会阻断其他插件关闭。

**实战提问：定时任务该用插件还是自己起个 `tokio::spawn`？**
要 cron 语义、要能优雅停止 → 用 `SchedulerPlugin`；
只是「偶尔跑一次」→ 写在业务代码里 `tokio::spawn` 即可，不必引入插件。

**实战提问：发邮件、发短信这类第三方调用放在哪？**
客户端（连接、凭证）放 `on_startup` 初始化；真正发送写在 service 里被 handler 调用。
只有当你需要在关闭时清理连接池时，才升级成插件。

## 7.5 这三者的关系再总结一遍

```text
启动顺序：  配置 → 全局资源（db/cache/template） → on_startup 钩子 → 插件 start → 中间件链 → 监听
              ↑ 所有东西的前提         ↑ 一次性初始化      ↑ 有生命周期的组件  ↑ 每个请求都过
关闭顺序：  ← 插件 stop（逆序）
```

- **配置开关**：框架内置能力，改文件即生效。
- **启动钩子**：一次性初始化，无关闭需求。
- **插件**：有 start/stop 生命周期与依赖顺序。
- **中间件**：请求级横切逻辑（认证、限流、日志、包装）。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [④ 中间件](middleware.md) | [⑥ 自定义配置](custom-config.md) → |

<sub>本分册属于「从零到跑起来（8 步分册）」，共 8 篇。首篇 / 末篇可点「返回总目录」。</sub>

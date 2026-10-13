---
id: guides.startup.custom-config
title: 自定义配置与常见问题
summary: [custom] 配置段怎么写怎么读、类型安全的强类型配置封装、新手常见问题速查与下一步指引。
tags: [getting-started, custom-config, faq]
status: active
owner: alun-core-team
source_of_truth: [code, examples, config/config.toml]
depends_on: [guides.getting-started]
last_reviewed: 2026-10-06
---

# 自定义配置与常见问题

> 本文件是 [`getting-started.md`](../getting-started.md) 的 `custom-config` 分册；总目录与全流程顺序见 [从零到跑起来](../getting-started.md)。

# 8. 第 8 步：自定义配置怎么写、怎么读

框架的 `AppConfig` 是**固定结构**（`server` / `log` / `database` / `cache` / `middleware` / …）。
你自己的配置（支付网关地址、业务阈值、开关）**不要**去改框架结构，统一放在 `[custom]` 段 ——
它的类型是 `HashMap<String, serde_json::Value>`，插件和业务都能读写。

## 8.1 在配置文件里写

```toml
# config/config.toml
[custom.payment]
provider = "alipay"
base_url = "https://openapi.alipay.com"
timeout_secs = 10

[custom.feature_flags]
new_checkout = true
max_cart_items = 50
```

## 8.2 在 Rust 里读：两种方式

**方式一：临时取一两个值（脚本式）。**

```rust,no_run
use alun::prelude::*;

/// 读取 `[custom.payment].timeout_secs`，缺失时用默认值。
fn payment_timeout() -> u64 {
    cfg()
        .custom
        .get("payment")
        .and_then(|v| v.get("timeout_secs"))
        .and_then(|v| v.as_u64())
        .unwrap_or(10)
}
```

**方式二：结构化读取（推荐，配置一多就该这么写）。**
定义一个结构体 + `Deserialize`，读一次缓存复用：

```rust,no_run
use alun::prelude::*;
use serde::Deserialize;

/// `[custom.payment]` 段对应的配置结构体。
#[derive(Debug, Clone, Deserialize, Default)]
struct PaymentConfig {
    /// 支付渠道标识。
    #[serde(default)]
    provider: String,
    /// 网关地址。
    #[serde(default)]
    base_url: String,
    /// 请求超时（秒）。
    #[serde(default)]
    timeout_secs: u64,
}

/// 从 `[custom.payment]` 读取强类型配置；缺失或类型不符时返回默认值。
///
/// 用 `try_config()` 而不是 `cfg()`：配置未初始化时返回默认值而不是 panic。
fn payment_config() -> PaymentConfig {
    try_config()
        .and_then(|cm| cm.get().custom.get("payment").cloned())
        .and_then(|v| serde_json::from_value::<PaymentConfig>(v).ok())
        .unwrap_or_default()
}
```

`#[serde(default)]` 很重要：配置项少写一个就用默认值，避免「配置少了一行直接启动失败」。

## 8.3 全局访问函数速查

| 函数 | 返回 | 什么时候用 |
| ---- | ---- | ---- |
| `cfg()` | `&AppConfig` | 常规读取静态配置（**未初始化会 panic**） |
| `config()` | `&Arc<ConfigManager>` | 需要动态配置读写时 |
| `try_config()` | `Option<&Arc<ConfigManager>>` | 不确定是否初始化（测试、工具函数） |

```rust,no_run
use alun::prelude::*;

# fn demo() {
// 静态配置：启动后不再变化
let listen = cfg().server.listen.clone();
let db_type = cfg().database.r#type.clone();
let jwt_enabled = cfg().middleware.auth.enabled;

// 动态配置：运行时读写，不落盘、不影响静态配置（适合后台改限流阈值这类场景）
config().set_dynamic("rate_limit.requests_per_window", json!(200));
let limit: Option<i64> = config()
    .get_dynamic("rate_limit.requests_per_window")
    .and_then(|v| v.as_i64());
config().remove_dynamic("rate_limit.requests_per_window");

# let _ = (listen, db_type, jwt_enabled, limit);
# }
```

**实战提问：动态配置和静态配置怎么选？**

| 场景 | 用哪个 |
| ---- | ---- |
| 启动后就固定的（端口、库地址、密钥） | 静态（`cfg()`） |
| 后台可调的（限流阈值、功能开关） | 动态（`set_dynamic` / `get_dynamic`） |
| 业务自定义参数（网关地址、业务阈值） | `[custom]` 段（静态读） |

**实战提问：插件怎么读配置？**
同样是 `[custom]` 或插件自己的配置段。例如内置插件就是「配置段 → 构造函数」的模式：
`NotificationPlugin::from_config(&cfg().plugins.notification)`、
`SerialPlugin::with_memory(cfg().plugins.serial.clone())`。
你自己写插件时照这个模式做：**构造函数收配置，配置从 `cfg()` 里取**。

---

# 9. 新手常见问题速查

| 提问 / 现象 | 结论 |
| ---- | ---- |
| 改了配置不生效 | 先 `cargo run -- print-config`；TOML 写错会**静默回退全默认**；profile 里 `enabled = false` 关不掉基础配置的 `true` |
| `db()` 直接 panic | 数据库未初始化（`enabled = false`，或用了 `with_config(default)`）；可选路径改 `try_db()` |
| 端口改了没用 | `[server] listen`，不是 `port`（`[server] port` 会被忽略）；也可用 `ALUN_SERVER_LISTEN` 或 `.serve("8080")` |
| 接口 404 | 忘了 `scan()` / `.module(..)`；`[router] prefix` 与路径不匹配（`!` 开头的绝对路径不拼前缀）；`[static_files] enabled = true` 会接管未匹配请求 |
| 中间件开了没反应 | 检查 `enabled` 与依赖项（认证还需要非空 `jwt_secret`；幂等/防重放需要缓存已初始化） |
| 插件没生效 | `[plugins] enabled` 只是名单，必须在 `App` 上显式 `.plugin(...)` 注册 |
| 想在启动时初始化自己的全局资源 | 用 `on_startup`，此时 `cfg()` / `db()` / `cache()` 已可用 |
| 我的配置段该写在哪 | 一律 `[custom.xxx]`，结构体 + `#[serde(default)]` 读取 |
| 日志去哪了 | `[log] dir` 注释掉则只输出 stdout；`format = "json"` 适合采集 |
| 生产环境必须设置什么 | `ALUN_MIDDLEWARE_AUTH_JWT_SECRET`（或 `FACAI_JWT_SECRET`）、数据库密码、`ALUN_SECRET_KEY`（用了加密密码时） |

---

# 10. 下一步

- **目录组织**：`src/{main.rs, controllers/, models/, services/, plugins/}` + `config/` + `migrations/` + `templates/`
  的推荐结构与命名约定见 [Alun 框架推荐项目架构](../recommended-architecture.md)。
- **动手跑示例**：

```bash
cargo run -p quick-start        # 全功能演示（工具集、脱敏、校验、加密）
cargo run -p example-basic      # 最小配置驱动启动
cargo run -p example-auth       # JWT 登录 / 刷新 / 登出
cargo run -p example-db-crud    # 数据库 CRUD + 导出
```

- **继续深入**：数据库 [database.md](../database.md) 分册、架构设计 [architecture/overview.md](../../architecture/overview.md)、
  各 crate API [04-crates-reference.md](../../reference/crates.md)、依赖关系 [05-dependencies.md](../../reference/dependencies.md)。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [⑤ 钩子与插件](hooks-plugins.md) | [附录：配置全字段参考](config-reference.md) → |

<sub>本分册属于「从零到跑起来（8 步分册）」，共 8 篇。首篇 / 末篇可点「返回总目录」。</sub>

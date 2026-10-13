---
id: guides.startup.middleware
title: 按需装配：中间件
summary: 先看配置能解决多少、还需要什么中间件、自定义中间件怎么写与挂载。
tags: [getting-started, middleware, assembly]
status: active
owner: alun-core-team
source_of_truth: [code, examples, config/config.toml]
depends_on: [guides.getting-started]
last_reviewed: 2026-10-06
---

# 按需装配：中间件

> 本文件是 [`getting-started.md`](../getting-started.md) 的 `middleware` 分册；总目录与全流程顺序见 [从零到跑起来](../getting-started.md)。


这是新手最容易迷糊的一步：同样是「加个功能」，为什么有时改配置就行、有时要写插件、有时要写钩子？
先看这张决策表，再看每一类什么时候用。

## 7.1 决策表：需求 → 用什么

| 你的需求 | 用什么 | 要写的代码量 |
| ---- | ---- | ---- |
| 开 CORS、限流、压缩、请求日志、JWT 认证、权限校验 | **配置文件开关** | 0 行 |
| 全局注入自己的响应头 / 统一包装响应 | **`App::layer` / `App::layer_from_fn`**（整站最外层） | 1 个中间件函数 |
| 中间件里要用「当前登录用户」做判断（数据权限、字段脱敏、按用户埋点） | **`App::route_layer`**（链内，能读身份） | 1 个中间件函数 |
| 启动时初始化自己的全局资源（第三方 SDK 客户端、连接池、业务缓存） | **`on_startup` 启动钩子** | 可注册多个，按注册顺序执行 |
| 有「启动做一次、关闭要收拾」的生命周期，或要按依赖顺序启动 | **插件（`Plugin` trait）** | 1 个结构体 + 3 个方法 |
| 定时任务、SMTP 通知、单号生成、进程内线程池 | **内置插件** | 注册 1 行 + 配置 |
| 纯业务逻辑（查库、算数、调外部 API） | **直接写在 handler / service 里** | — |

## 7.2 中间件：先看配置能解决多少

**能靠配置解决的，一律不要写代码。** 下面这些中间件在配置里 `enabled = true` 就自动入链：

```toml
[middleware]
request_id = false           # 注入 x-request-id（并回写同名响应头）
request_log = false          # 访问日志

[middleware.request_log_config]
exclude_paths = ["/health"]  # 支持精确匹配与通配符前缀（"/swagger*"）
log_duration = true

[middleware.auth]
enabled = false              # 开启后：JWT 认证
jwt_secret = ""              # 生产用 ALUN_MIDDLEWARE_AUTH_JWT_SECRET 注入
ignore_paths = ["/api/login", "/api/register"]
access_token_expire_secs = 7200
refresh_token_expire_secs = 604800

[middleware.cors]
enabled = false
allow_origins = ["https://app.example.com"]
allow_methods = ["GET", "POST"]
allow_headers = ["content-type", "authorization"]
allow_credentials = true

[middleware.compression]
enabled = false
level = 6                    # Gzip 级别 0-9

[middleware.rate_limit]
enabled = false
requests_per_window = 100
window_secs = 60

[middleware.security_headers]
enabled = true               # 默认开启

[middleware.nonce]           # 防重放：请求带 x-nonce，重复返回 409（需 cache feature）
enabled = false
ttl_secs = 300

[middleware.idempotency]     # 幂等键：请求带 x-idempotency-key，重复请求回放首次响应（需 cache feature）
enabled = false
ttl_secs = 86400
```

实际执行顺序（从外到内，已按安全要求排好，不用你操心）：

```text
SecurityHeaders → CORS → RequestLog → RequestId → Auth → Permission
→ RateLimit → Compression → Nonce → Idempotency → handler
```

**实战提问：开了 `[middleware.auth] enabled = true` 却还是能匿名访问？**
检查 `jwt_secret` 是否为空 —— `enabled = true` **且** `jwt_secret` 非空才会挂载认证层。
另外 `[router] prefix` 会自动加到 `ignore_paths` 前面，配 `/api/login` 时要考虑前缀。

**实战提问：权限怎么配？**
两种方式，宏观上记住结论：
- 配置里的 `[middleware.permission] rules` 受 `enabled` 控制；
- `#[alun::permission(path = "...", permission = "...")]` 宏注解**始终挂载**（安全默认，忘开开关也不会裸奔，
  只会在日志里告警）。

```toml
[middleware.permission]
enabled = false
# rules = [
#   { path = "/api/admin/*", methods = ["GET"], permission = "admin:read" },
# ]
```

**实战提问：我要加一个自己的全局中间件（比如统一加 `X-Trace-Id`）？**

```rust,no_run
use alun::prelude::*;
use axum::{extract::Request, middleware::Next, response::Response};

/// 给每个响应补一个自定义响应头。
async fn add_trace_header(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    res.headers_mut()
        .insert("x-trace-id", "trace-123".parse().unwrap_or_else(|_| "trace".parse().unwrap()));
    res
}

# async fn demo() -> Result<()> {
App::new()?
    .layer_from_fn(|router| router.layer(axum::middleware::from_fn(add_trace_header)))
    .scan()
    .start()
    .await
# }
```

### 7.3 自定义层放哪：位置决定能力

框架只有一个「内置链」，但自定义层的挂载点有 5 个，**位置不同、能力不同**。
记住一句话就够：**要记全（含 401/403/429）就用外层，要用身份就用内层。**

| 挂载点 | 代码 | 位置 | 能读身份 | 覆盖范围 |
| ---- | ---- | ---- | ---- | ---- |
| 链外全局层 | `App::layer(L)` / `App::layer_from_fn(f)` | 内置链**之外**、最外层 | ✗ | 整站（含 404、静态文件、`/metrics`） |
| **链内全局层** | `App::route_layer(L)` | 内置链**之内**（Auth/权限之后） | ✓ | 仅匹配到的路由 |
| 子树层 | `AlunRouter::merge_with_layer(prefix, sub, wrap)` | 链内 | ✓ | `prefix` 下的整组路由 |
| 单路由层 | `#[alun::layer(L)]`（写在路由注解**上方**） | 链内 | ✓ | 那一条路由 |
| 单路由层（代码式） | `AlunRouter::add_*_with_layer(path, h, wrap)` | 链内 | ✓ | 那一条路由 |

**为什么会有这个差别？** 因为 axum 的 `Router::layer` 只作用于「调用那一刻已经存在的路由」。
框架在建链**之前**应用 `route_layer`（所以它自然落在链内、且此时还没有 fallback），
在建链**之后**应用 `layer`（所以它包住整站、包括后挂上的 404 与 `/metrics`）。
这在 axum 里叫 `route_layer` 与 `layer` 的区别，本框架只是把它暴露成了两个 builder 方法。

几个结论：

- **操作审计 / 访问日志用外层 `layer`** —— 否则 401 / 403 / 429 会漏记，耗时也测不准；
- **数据权限、字段脱敏、按用户埋点用 `route_layer`** —— 这些必须知道「当前是谁」；
- 只想给一片路由或一条路由加，用 `merge_with_layer` / `#[alun::layer]`，别拉全局；
- 链外层的层里**不要去提取 `AuthClaims`**（那时还没认证，取不到）。

只给单个路由挂 layer 时，也可以直接用 `AlunRouter::add_*_with_layer` 把 layer 包在路由上
（`NonceLayer` / `IdempotencyLayer` 的按路由用法就是这么做的）。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [③ 数据库与第一个接口](first-endpoint.md) | [⑤ 钩子与插件](hooks-plugins.md) → |

<sub>本分册属于「从零到跑起来（8 步分册）」，共 8 篇。首篇 / 末篇可点「返回总目录」。</sub>

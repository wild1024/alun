# 扩展点：启动钩子 / 自定义中间件 / `WebPlugin` / 显式注册

> 一句话：框架给你这几个「往里塞东西」的口子 —— `on_startup`（启动钩子）、
> `layer` / `layer_from_fn` / `route_layer` / `merge_with_layer` / `#[alun::layer]`
> （自定义中间件的 5 个挂载点）、`web_plugin`；
> 路由则有两条通道：`#[alun::get]` + `App::scan()`（**默认主推**）与 `#[alun::routes]` + `App::module()`（可选，库分发/要掌控装配顺序时用）。

## 1. 启动钩子 `App::on_startup(|| async { .. })`

- **可调多次**，追加而非覆盖 —— 多个模块各自注册互不影响；
- 在服务器开始接受连接**之前**执行，适合预热缓存、注册定时任务、建外部连接。

```rust,no_run
use alun::prelude::*;

App::new()?
    .on_startup(|| async { /* 预热缓存 */ })
    .on_startup(|| async { /* 注册第二个钩子 */ })
    .parse_cli()
    .start()
    .await?;
```

## 2. 中间件（外层）：`App::layer(L)` / `App::layer_from_fn(f)`

两者**完全等价**，只是写法不同 —— `layer` 收现成的 `tower::Layer`，
`layer_from_fn` 收一个 `FnOnce(Router) -> Router` 闭包（要做 `nest` / `merge`
这类 Router 级变换时用它）。**可调多次**，两者共用同一条有序序列，**先注册者更靠内层**。

**位置**：内置中间件链**之外**、整站最外层。
**能力边界**：因此**读不到身份**（本层执行时 `AuthLayer` 还没跑，`AuthClaims`
不在请求扩展里）；作为交换，它覆盖**全部**请求 —— 包括 404 fallback、静态文件、
`/metrics`，也就是**能记录到 401 / 403 / 429 与真实端到端耗时**。

```rust,no_run
use alun::prelude::*;

App::new()?
    // 直接挂现成的 tower / tower-http 层（如 ConcurrencyLimitLayer / TimeoutLayer）
    .layer(axum::middleware::from_fn(outer_guard))
    // 需要 Router 级变换（nest / merge）时用闭包形态
    .layer_from_fn(|router| router.layer(axum::middleware::from_fn(outer_guard)))
    .scan()
    .start()
    .await?;

# async fn outer_guard(
#     req: axum::http::Request<axum::body::Body>,
#     next: axum::middleware::Next,
# ) -> axum::response::Response {
#     next.run(req).await
# }
```

> 审计日志 / 访问日志这类「必须记全、且必须在最外层」的横切关注点放这里。
> **不要**指望在这一层读到当前用户 —— 那是下一节的事。

> 旧名 `App::with_middleware_hook(f)` **仍然可用但已废弃**，语义与 `layer_from_fn`
> 完全一致，请迁移（它当年被 doc 写成 `fn(Identity) -> Layer` 是错的：`Identity`
> 参数从来不存在，因为这一层根本看不到身份）。

装现成的 `tower` / `tower-http` 中间件（`ConcurrencyLimitLayer`、`CorsLayer`、`TraceLayer`、
`TimeoutLayer`…）直接用 `App::layer` 即可。两点注意：

- **不要手写 `Service`**，除非确实需要 —— 用 `layer_from_fn` + `axum::middleware::from_fn`
  更短，且能直接拿到 `Request` / `Next`；
- 层**不能返回 tower 错误**（`Error` 必须可转为 `Infallible`）。要拒绝请求就直接返回响应
  （如 `ApiError` 的 `IntoResponse`），这也是框架内置层的统一做法（不引入
  `HandleErrorLayer` 第二条错误路径）。

## 3. 中间件（内层）：`App::route_layer(L)` / `merge_with_layer` / `#[alun::layer]`

`App::route_layer(L)` 语义对标 axum 的 `Router::route_layer`：**只作用于匹配到的路由**，
不含 fallback、不含 `/metrics`；且它在建链**之前**应用，因此位于 Auth / 权限 / 限流
**之内** —— 这一层**能**读到 `AuthClaims` 与租户上下文。

再往下还有两个更细的档位：

| 粒度 | API | 范围 |
|---|---|---|
| 全局（链内） | `App::route_layer(L)` | 所有匹配路由 |
| **子树** | `AlunRouter::merge_with_layer(prefix, sub, wrap)` | `prefix` 下的整组路由 |
| **单条路由** | `#[alun::layer(L)]` 写在路由注解**上方**；或 `AlunRouter::add_get_with_layer(path, h, wrap)` | 那一条路由 |

```rust,no_run
use alun::prelude::*;

// 单条路由：属性宏写在路由注解【上方】——
// 它会摘掉该路由的编译期自注册，改生成「带 layer 的注册」，因此不会重复挂载
#[alun::layer(axum::middleware::from_fn(require_tenant))]
#[alun::get("/orders")]
async fn list_orders(claims: AuthClaims) -> Res<String> {
    Res::ok(claims.0.sub.clone())
}

# async fn require_tenant(
#     req: axum::http::Request<axum::body::Body>,
#     next: axum::middleware::Next,
# ) -> axum::response::Response {
#     next.run(req).await
# }
```

```rust,no_run
use alun::prelude::*;

// 子树：只给 /admin 这一片挂层。
// merge_with_layer 是 AlunRouter 的方法，经 App::routes 拿到 &mut AlunRouter。
App::new()?
    .routes(register_admin)
    .scan()
    .start()
    .await?;

# fn register_admin(r: &mut AlunRouter) {
#     r.merge_with_layer("/admin", admin_routes(), |sub| {
#         sub.route_layer(axum::middleware::from_fn(audit_guard))
#     });
# }
#
# fn admin_routes() -> AlunRouter {
#     let mut r = AlunRouter::new();
#     r.add_get("/ping", || async { Res::ok("pong".to_string()) });
#     r
# }
#
# async fn audit_guard(
#     req: axum::http::Request<axum::body::Body>,
#     next: axum::middleware::Next,
# ) -> axum::response::Response {
#     next.run(req).await
# }
```

### 选哪一档：看你要不要「身份」

| 你的诉求 | 用哪个 | 能读身份 |
|---|---|---|
| 记全所有请求（含 401 / 403 / 429）、测真实耗时 | `App::layer` / `layer_from_fn` | ✗ |
| 按当前用户做数据权限 / 字段脱敏 / 按用户埋点 | `App::route_layer` | ✓ |
| 只给 `/admin/*` 这一片挂层 | `merge_with_layer` | ✓ |
| 只给一条路由挂层 | `#[alun::layer]` / `add_*_with_layer` | ✓ |
| 只是想在 handler 里拿身份 | **不用中间件**，直接写提取器 `AuthClaims` / `Principal` | ✓ |

> 底层原因：axum 的 `Router::layer` 只作用于「调用时刻已存在的路由」，
> `route_layer` 另外排除 fallback。框架把 `layer` 系列放在建链**之后**（覆盖整站），
> 把 `route_layer` 放在建链**之前**（自然落在链内），两条路径的能力差别由此而来。
>
> 链条顺序（外→内）：`SecurityHeaders → CORS → RequestLog → RequestId → Auth → Permission
> → RateLimit → Compression → Nonce → Idempotency →` (**`route_layer` 序列**) `→` (**匹配到的路由**) `→ handler`。
> 而 `App::layer` / `layer_from_fn` 序列整体位于最外层，包住上面这一切（含 404 与 `/metrics`）。

## 4. `WebPlugin`：库形态分发的最佳通道

`WebPlugin` 是 `Plugin` 的**子 trait** —— 在生命周期（`start` / `stop` / `depends_on`）之上，
再贡献三样 Web 装配，三者**都有默认空实现**，所以旧代码 `impl Plugin for X` 改成
`impl WebPlugin for X` 即可平滑升级。

| 方法 | 默认 | 作用 |
|---|---|---|
| `routes(&self, &mut AlunRouter)` | 空 | 贡献路由，与宏收集的路由**合并** |
| `permission_rules() -> Vec<PermissionDef>` | `vec![]` | 贡献权限规则，与 `#[alun::permission]` 合并 |
| `no_auth_paths() -> Vec<NoAuthDef>` | `vec![]` | 贡献免鉴权路径，与 `#[alun::no_auth]` 合并 |

```rust,ignore
use alun::web::{AlunRouter, NoAuthDef, PermissionDef, WebPlugin};

struct AdminPlugin;

#[alun::async_trait]
impl alun::Plugin for AdminPlugin {
    fn name(&self) -> &str { "admin" }
    async fn start(&self) -> alun::Result<()> { Ok(()) }
    async fn stop(&self) -> alun::Result<()> { Ok(()) }
}

impl WebPlugin for AdminPlugin {
    fn routes(&self, router: &mut AlunRouter) {
        router.add_get("/admin/health", || async { "ok" });
    }
}

App::new()?.web_plugin(AdminPlugin).parse_cli().start().await?;
```

`.web_plugin(p)` 一行同时完成「生命周期注册 + Web 装配注册」（内部包了一层
`WebPluginAdapter`，所以原来的 `Plugin` 生命周期照常生效）。

**什么时候该用 `WebPlugin`**：你要分发一个**库 / 插件 crate**，里面带自己的接口。
因为编译期收集（`inventory` / 链接器段）发生在**最终二进制**里 —— 依赖方无法指望收集到
库里的 handler；显式注册才能把路由交到使用方手里。应用自身写的 handler 用宏即可。

## 5. 两条路由通道

| 通道 | 写法 | 适用 |
|---|---|---|
| **编译期收集 ⭐ 默认** | `#[alun::get("/path")]` + `App::scan()` | **应用代码默认走这条**：handler 分散在多个文件，一行 `.scan()` 全收，无需任何注册代码 |
| 显式注册（可选） | `#[alun::routes]` + `App::module(api::register)` | 库 / 插件分发、需完全掌控装配顺序、要 IDE 跳转链到 register |
| 控制器分组 | `#[alun::controller("/prefix")]` 标在 impl 块 | 一组接口共享前缀 / 归属 |

`scan()` 与 `.module()` **并存且合并**（`scan()` 先扫宏收集项，`.module()` 的实例后挂），
可混用，也可以只用其一：

```rust,no_run
use alun::prelude::*;

// ── 通道一：宏 + scan（默认，日常只用这条）──
#[alun::get("/hello")]
async fn hello() -> Res<String> { Res::ok("hi".into()) }

// ── 通道二：显式模块（可选）──
#[alun::routes]
mod api {
    use alun::prelude::*;

    #[alun::get("/users")]
    pub async fn list_users() -> Res<String> { Res::ok("[]".into()) }
}

#[tokio::main]
async fn main() {
    App::new()?
        .scan()                    // hello 在这里被收进来
        .module(api::register)     // register 由 #[alun::routes] 生成
        .parse_cli()
        .start()
        .await?;
}
```

> 纯应用**建议只用 `scan()`**；`.module()` 只在「分发带接口的库」或「装配顺序必须可控」时才需要。

其余装配口子：

| API | 用途 |
|---|---|
| `App::module(f)` | `f: fn(Self) -> Self` 的糖，`f(self)` 直接返回 |
| `App::routes(f)` | `f: fn(&mut AlunRouter)`，只贡献路由 |
| `AlunRouter::merge_with_layer(prefix, sub, wrap)` | 合并子路由，并只给该前缀下的整组路由挂一层（见第 3 节） |
| `App::permission_defs(..)` / `App::no_auth_defs(..)` | 追加权限 / 免鉴权规则（与切片来源合并） |
| `App::into_router()` | 只构建 `Router` 不启服务 —— 测试与内嵌挂载用；装配顺序与 `start()` 一致 |
| `App::web_plugin(p)` | 见上 |

> **历史坑（0.3 已修，与通道选择无关）**：早期路由靠 `linkme` 分布式切片收集，在某些工具链
> （如 windows-gnu + nightly）会**静默失效** —— 编译链接都不报错，表现是全站 404。
> 0.3 已把收集机制换成 `inventory`（同样在编译期工作，不再依赖链接器自定义段，
> 不受 `--gc-sections` 影响），`App::scan()` 因此仍是可靠的默认通道；
> `#[alun::routes]` + `App::module()` 则完全不依赖任何收集机制，是它的显式等价物。

## 6. 行级异步 Hook `#[hook_async(H)]` + `AsyncHook`

与上面「应用级扩展点」不同，这是**写在模型上**的行级扩展：

```rust,ignore
use alun::prelude::*;

#[derive(Model)]
#[table("biz_order")]
#[hook_async(OrderAuditHook)]
struct Order { /* … */ }

struct OrderAuditHook;

#[alun::async_trait]
impl AsyncHook for OrderAuditHook {
    async fn after_insert(row: &Row) -> alun::Result<()> { /* 写审计表 / 推 SSE */ Ok(()) }
}
```

`AsyncHook` 只有三个可选方法（`after_insert(&Row)` / `after_update(&Row)` / `after_delete(&Value)`），
默认都是 `Ok(())`。

> ⚠️ 语义边界：异步 Hook 在**事务提交之后**触发，因此它里面有错**不会回滚已提交的数据**。
> 需要「和主业务同生共死」的逻辑用同步 `Hook`（`impl Hook`）；异步 Hook 只放「可重试、幂等、失败可读日志」的事
> （审计、缓存失效、消息投递、SSE 通知）。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [插件 · 缓存 · 模板](plugins.md) | [异步任务 / 定时任务 / Kafka](task.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

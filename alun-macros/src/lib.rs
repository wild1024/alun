//! Alun proc macros
//!
//! 核心价值：用编译时代码生成替代 Java 的反射 + 注解 + classpath 扫描。

mod plugin;
mod route;
mod routes;
mod task_handler;

use proc_macro::TokenStream;

/// 标记路由控制器类
#[proc_macro_attribute]
pub fn controller(attr: TokenStream, item: TokenStream) -> TokenStream {
    route::controller_impl(attr, item)
}

/// GET 路由
#[proc_macro_attribute]
pub fn get(attr: TokenStream, item: TokenStream) -> TokenStream {
    route::method_route_impl("get", attr, item)
}

/// POST 路由
#[proc_macro_attribute]
pub fn post(attr: TokenStream, item: TokenStream) -> TokenStream {
    route::method_route_impl("post", attr, item)
}

/// PUT 路由
#[proc_macro_attribute]
pub fn put(attr: TokenStream, item: TokenStream) -> TokenStream {
    route::method_route_impl("put", attr, item)
}

/// DELETE 路由
#[proc_macro_attribute]
pub fn delete(attr: TokenStream, item: TokenStream) -> TokenStream {
    route::method_route_impl("delete", attr, item)
}

/// PATCH 路由（部分更新）
///
/// 与 [`put`] 语义一致，仅 HTTP 方法不同：
///
/// ```ignore
/// #[alun::patch("/api/invoices/{id}/status")]
/// async fn update_status(Path(id): Path<String>, Json(req): Json<StatusReq>) -> Res<()> {
///     // ...
/// }
/// ```
///
/// 与 `#[alun::permission]`/`#[alun::no_auth]` 等宏可自由组合（各自独立注册）。
#[proc_macro_attribute]
pub fn patch(attr: TokenStream, item: TokenStream) -> TokenStream {
    route::method_route_impl("patch", attr, item)
}

/// 权限拦截：为处理器添加方法级权限校验
///
/// # 参数
///
/// ```ignore
/// #[alun::permission(path = "/api/admin/stats", method = "GET", permission = "admin:access")]
/// async fn admin_stats() -> Res<Value> { ... }
/// ```
///
/// 编译时收集到 `PERMISSION_ROUTES` 切片，启动时由 `AlunRouter` 统一应用 `RequirePermissionLayer`。
#[proc_macro_attribute]
pub fn permission(attr: TokenStream, item: TokenStream) -> TokenStream {
    route::permission_impl(attr, item)
}

/// 标记无需认证的路由路径
///
/// # 参数
///
/// ```ignore
/// #[alun::no_auth("/api/public")]
/// #[alun::get("/api/public")]
/// async fn public_api() -> Res<Value> { ... }
/// ```
///
/// 编译时收集到 `NO_AUTH_ROUTES` 切片，启动时与配置文件中的 `ignore_paths` 合并。
/// 即使在无需认证的路径上，如果提供了有效 Token，仍然会解析并注入用户信息到 extensions。
#[proc_macro_attribute]
pub fn no_auth(attr: TokenStream, item: TokenStream) -> TokenStream {
    route::no_auth_impl(attr, item)
}

/// 方法级中间件：给同一条路由挂一个 `tower::Layer`
///
/// 必须与路由注解同用，且写在路由注解**上方**：
///
/// ```ignore
/// #[alun::layer(SensitiveMaskLayer::new())]
/// #[alun::get("/admin/export")]
/// async fn export() -> Res<String> { /* ... */ }
/// ```
///
/// # 位置与能力
///
/// 层经 `MethodRouter::route_layer` 挂载，位于**内置中间件链之内**，
/// 因此处理器参数可以直接取到 `AuthClaims` / `Principal`；
/// 代价是看不到内置层的拒绝（401 / 403 / 429）。
///
/// 与 `#[alun::permission]` 可叠加，顺序天然固定：`#[permission]` 走内置链的
/// `PermissionCheckLayer`（更外），本层更内 —— 即「权限检查 → 本层 → 处理器」。
///
/// # 类型约束
///
/// 层不能返回 tower 错误（`Error` 必须可转为 `Infallible`）；需要拒绝请求时
/// 直接返回响应（如 `ApiError`）。多个层请自行组合成一个 `Layer` 传入
/// （如 `LayerStack`），而不是写多条本注解。
#[proc_macro_attribute]
pub fn layer(attr: TokenStream, item: TokenStream) -> TokenStream {
    route::layer_impl(attr, item)
}

/// 模块级**显式**路由注册 —— 不依赖链接器段收集。
///
/// 在模块上标注本属性后，模块内的 `#[alun::get]` / `#[alun::permission]` /
/// `#[alun::no_auth]` 注解会在**宏展开期**被扫描并剥离，同时生成一个
/// `pub fn register(app: App) -> App` 注册函数：
///
/// ```ignore
/// #[alun::routes]
/// mod api {
///     #[alun::get("/users")]
///     pub async fn list_users() -> Res<String> { Res::ok("[]".into()) }
/// }
///
/// App::new()?.module(api::register).start().await?;
/// ```
///
/// 与 `#[alun::get]` + `App::scan()` 的区别：后者依赖 `inventory` 编译期收集
/// 在链接期收集注册项，在部分工具链（windows-gnu）上会静默失效导致路由全部丢失；
/// 本宏是纯 Rust 调用，任何工具链都成立。
#[proc_macro_attribute]
pub fn routes(attr: TokenStream, item: TokenStream) -> TokenStream {
    routes::routes_impl(attr, item)
}

/// 标记类型为插件，编译期自动注册
#[proc_macro_attribute]
pub fn plugin(_attr: TokenStream, item: TokenStream) -> TokenStream {
    plugin::plugin_impl(item)
}

/// 标记 TaskHandler 实现，编译期自动注册到 `inventory` 收集器
///
/// 配合 `HandlerRegistry::from_discovered()` 在运行时批量注册。
///
/// # 参数
///
/// | 参数 | 类型 | 默认值 | 说明 |
/// |------|------|--------|------|
/// | `task_type` | i16 | **必填** | 任务类型标识，唯一 |
/// | `topic` | &str | "task_{task_type}" | Kafka topic |
/// | `priority` | Normal/High/Low/Critical | Normal | 优先级 |
/// | `timeout_seconds` | u64 | 300 | 超时秒数 |
/// | `max_retries` | u32 | 3 | 最大重试次数 |
/// | `description` | &str | "" | 任务描述 |
/// | `dead_letter_topic` | Option<&str> | None | 死信队列 topic |
///
/// # 示例
///
/// ```ignore
/// #[task_handler(task_type = 1, topic = "export", timeout_seconds = 60)]
/// struct ExportHandler;
///
/// #[async_trait]
/// impl TaskHandler for ExportHandler {
///     fn task_type(&self) -> i16 { 1 }
///     async fn execute(&self, payload: Value) -> Result<Value, String> {
///         Ok(json!({"file": "https://..."}))
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn task_handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    task_handler::task_handler_impl(attr, item)
}

//! Alun proc macros
//!
//! 核心价值：用编译时代码生成替代 Java 的反射 + 注解 + classpath 扫描。

mod route;
mod plugin;
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

/// 标记类型为插件，编译期自动注册
#[proc_macro_attribute]
pub fn plugin(_attr: TokenStream, item: TokenStream) -> TokenStream {
    plugin::plugin_impl(item)
}

/// 标记 TaskHandler 实现，编译期自动注册到 TASK_HANDLERS 分布式切片
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

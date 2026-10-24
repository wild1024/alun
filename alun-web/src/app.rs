//! App 构建器——框架唯一入口，配置驱动，一行启动
//!
//! 设计要点：
//! - 全局资源单例，无需 State 注入
//! - `new()` / `from_config()` / `from_config_dir()` 自动初始化日志、数据库、缓存、JWT
//! - `start()` 直接构建 axum Router，无需 with_state
//! - 用户只需编辑 config.toml，无需在 main.rs 中手动初始化任何组件

use axum::Router;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Json};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
#[cfg(feature = "cache")]
use std::time::Duration;
use std::time::Instant;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;
use tracing::info;

use crate::api::{Res, codes};
use crate::middleware as mw;
use crate::resources::*;
use crate::router::AlunRouter;
use alun_config::{AppConfig, ConfigManager};
use alun_core::{PluginManager, Result};

/// 应用运行时设置
#[derive(Clone)]
pub struct AppSettings {
    /// 配置文件目录路径
    pub config_path: Option<String>,
    /// 仅生成默认配置文件后退出
    pub gen_config_only: bool,
    /// 打印当前配置后退出
    pub print_config: bool,
}

/// App 构建器 —— 框架唯一入口，配置驱动，一行启动
///
/// 无需任何泛型，全局资源单例访问。
///
/// # 示例
///
/// ```ignore
/// App::new()?
///     .get("/", || async { Res::ok("Hello") })
///     .start()
///     .await
/// ```
pub struct App {
    /// 路由注册器（内部 axum router）
    router: Option<AlunRouter>,
    /// 插件管理器（拓扑排序启动/关闭）
    pub(crate) plugins: PluginManager,
    /// Web 插件视图（同一批插件实例的 [`crate::WebPlugin`] 侧引用）
    ///
    /// 与 `plugins` 指向**同一实例**：`App::web_plugin` 把 `Arc<P>` 同时放进
    /// 管理器（走生命周期拓扑排序）和这里（走 Web 装配贡献）。分两处存放是因为
    /// `PluginManager` 持有 `Box<dyn Plugin>`，无法再取出具体类型调用 Web 方法。
    pub(crate) web_plugins: Vec<Arc<dyn crate::plugin::WebPlugin>>,
    /// 应用设置（CLI 参数等）
    settings: AppSettings,
    /// 配置管理器（文件加载 + 运行时动态配置）
    config_mgr: Option<Arc<ConfigManager>>,
    /// 编译期嵌入的迁移器（[`App::migrate_embedded`] 注入；`migrate!` 宏产物）。
    ///
    /// 注入后 `[database.migration]` 的目录来源（`path` / `sources` / `seed_path`）
    /// 不再使用——运行时不再依赖 `migrations/` 目录；执行选项仍取自配置。
    #[cfg(feature = "db")]
    embedded_migrator: Option<alun_db::Migrator>,
    /// 全局路由前缀（从 `router.prefix` 读取）
    prefix: String,
    /// 限流存储（跨请求共享，确保限流计数正确）
    rate_limit_store: Arc<RwLock<HashMap<String, mw::IpWindow>>>,
    /// **链外路由器扩展序列**（可注册多个，严格按注册顺序应用）。
    ///
    /// [`App::layer_from_fn`]（自定义闭包）与 [`App::layer`]（现成 tower 中间件）
    /// 都写入同一个序列，**相对顺序即注册顺序**——不再存在"钩子恒先于 Layer"的
    /// 隐式分组（F4：消除双通道）。先注册者更靠内层。
    ///
    /// **关键**：整条序列在 `start()` 中于内置中间件链、`/metrics`、fallback **之后**
    /// 应用，因此全部位于**内置链之外**——早于 `AuthLayer` 执行，请求期读不到
    /// `AuthClaims`。需要身份请改用 [`App::route_layer`]（链内）或提取器
    /// （`AuthClaims` / `Principal`）。
    router_extensions: Vec<Box<dyn FnOnce(Router) -> Router + Send>>,
    /// **链内路由层序列**（[`App::route_layer`]）。
    ///
    /// 与 `router_extensions` 正好相反：在 `build_middleware_chain` **之前**应用到
    /// 路由上，因此位于**全部内置中间件之内**，可读 `AuthClaims` / 租户上下文；
    /// 代价是看不到任何内置层的拒绝（401 / 403 / 429）与其它内置层的耗时。
    route_layers: Vec<Box<dyn FnOnce(Router) -> Router + Send>>,
    /// 自定义启动钩子（**可注册多个**，在全局资源初始化之后、插件启动之前按注册顺序执行）
    startup_hooks: Vec<Box<dyn FnOnce() -> StartupHookFuture + Send>>,
    /// 显式注册模块贡献的权限规则（`#[alun::routes]` 模块 / [`App::permission_defs`]）。
    ///
    /// 与 `#[alun::permission]` 经分布式切片收集到的规则**合并**——后者依赖链接器
    /// 段收集，在部分工具链（windows-gnu）上会静默失效；本字段是不依赖链接器的通道。
    extra_permissions: Vec<crate::PermissionDef>,
    /// 显式注册模块贡献的免认证路径（`#[alun::routes]` 模块 / [`App::no_auth_defs`]）。
    extra_no_auth: Vec<crate::NoAuthDef>,
}

/// 启动钩子返回的异步任务类型（装箱 + `Send`），用别名收敛字段签名。
type StartupHookFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

impl App {
    /// 从默认 `config/` 目录加载配置并构建 App
    ///
    /// 等价于 `App::from_config_dir("config")`。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// App::new()?
    ///     .get("/", || async { Res::ok("Hello") })
    ///     .start()
    ///     .await
    /// ```
    pub fn new() -> Result<Self> {
        Self::from_config_dir("config")
    }

    /// 从默认目录 `config/` 加载配置，自动初始化日志、数据库、缓存
    pub fn from_config() -> Result<Self> {
        Self::new()
    }

    /// 从指定目录加载配置
    pub fn from_config_dir(dir: &str) -> Result<Self> {
        let cm = Arc::new(ConfigManager::load(Some(dir)));
        Self::with_config_manager(cm)
    }

    /// 使用给定的 AppConfig 构建（跳过文件加载）
    pub fn with_config(cfg: AppConfig) -> Result<Self> {
        let cm = ConfigManager {
            static_config: cfg,
            dynamic: parking_lot::RwLock::new(HashMap::new()),
        };
        Self::with_config_manager(Arc::new(cm))
    }

    /// 使用给定的 ConfigManager 构建 App
    ///
    /// 公开方法，用于手动构建 App 的场景
    pub fn with_config_manager(cm: Arc<ConfigManager>) -> Result<Self> {
        let cfg = cm.get();
        alun_log::init_with_sql_logging(&cfg.log, cfg.database.sql_logging);

        let prefix = cfg.router.prefix.clone();

        Ok(Self {
            router: Some(AlunRouter::new()),
            plugins: PluginManager::new(),
            web_plugins: Vec::new(),
            settings: AppSettings {
                config_path: Some("config".into()),
                gen_config_only: false,
                print_config: false,
            },
            config_mgr: Some(cm),
            prefix,
            rate_limit_store: Arc::new(RwLock::new(HashMap::new())),
            router_extensions: Vec::new(),
            route_layers: Vec::new(),
            startup_hooks: Vec::new(),
            extra_permissions: Vec::new(),
            extra_no_auth: Vec::new(),
            #[cfg(feature = "db")]
            embedded_migrator: None,
        })
    }

    /// 解析 CLI 参数（`--gen-config` / `--print-config`）
    ///
    /// 需在 `start()` 之前调用。
    pub fn parse_cli(mut self) -> Self {
        let (gen_config, print_config) = alun_config::env::parse_args();
        self.settings.gen_config_only = gen_config;
        self.settings.print_config = print_config;
        self
    }

    // ── 路由注册 ──

    /// 注册 GET 路由，返回 `Self` 以支持链式调用
    pub fn get<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: axum::handler::Handler<T, ()>,
        T: 'static,
    {
        if let Some(ref mut r) = self.router {
            r.add_get(path, handler);
        }
        self
    }

    /// 注册 POST 路由，返回 `Self` 以支持链式调用
    pub fn post<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: axum::handler::Handler<T, ()>,
        T: 'static,
    {
        if let Some(ref mut r) = self.router {
            r.add_post(path, handler);
        }
        self
    }

    /// 注册 PUT 路由，返回 `Self` 以支持链式调用
    pub fn put<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: axum::handler::Handler<T, ()>,
        T: 'static,
    {
        if let Some(ref mut r) = self.router {
            r.add_put(path, handler);
        }
        self
    }

    /// 注册 PATCH 路由，返回 `Self` 以支持链式调用
    pub fn patch<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: axum::handler::Handler<T, ()>,
        T: 'static,
    {
        if let Some(r) = &mut self.router {
            r.add_patch(path, handler);
        }
        self
    }

    /// 注册 DELETE 路由，返回 `Self` 以支持链式调用
    pub fn delete<H, T>(mut self, path: &str, handler: H) -> Self
    where
        H: axum::handler::Handler<T, ()>,
        T: 'static,
    {
        if let Some(ref mut r) = self.router {
            r.add_delete(path, handler);
        }
        self
    }

    /// 通用路由注册 —— 用字符串指定 HTTP 方法
    ///
    /// # 示例
    ///
    /// ```ignore
    /// App::new()?
    ///     .route("PATCH", "/api/data", patch_handler)
    ///     .start()
    ///     .await
    /// ```
    pub fn route<H, T>(mut self, method: &str, path: &str, handler: H) -> Self
    where
        H: axum::handler::Handler<T, ()>,
        T: 'static,
    {
        if let Some(ref mut r) = self.router {
            r.add_route(method, path, handler);
        }
        self
    }

    /// 路由分组 —— 将一组路由归到同一前缀下
    ///
    /// # 示例
    ///
    /// ```ignore
    /// App::new()?
    ///     .group("/api/v2", |app| {
    ///         app.get("/users", list_users)
    ///            .post("/users", create_user)
    ///     })
    ///     .start()
    ///     .await
    /// ```
    pub fn group(mut self, prefix: &str, f: impl FnOnce(Self) -> Self) -> Self {
        let sub = f(Self {
            router: Some(AlunRouter::new()),
            plugins: PluginManager::new(),
            web_plugins: Vec::new(),
            settings: AppSettings {
                config_path: None,
                gen_config_only: false,
                print_config: false,
            },
            config_mgr: None,
            prefix: String::new(),
            rate_limit_store: Arc::new(RwLock::new(HashMap::new())),
            router_extensions: Vec::new(),
            route_layers: Vec::new(),
            startup_hooks: Vec::new(),
            extra_permissions: Vec::new(),
            extra_no_auth: Vec::new(),
            #[cfg(feature = "db")]
            embedded_migrator: None,
        });
        if let (Some(ref mut r), Some(sub_r)) = (self.router.as_mut(), sub.router) {
            r.merge(prefix, sub_r);
        }
        self
    }

    /// 扫描 `#[get]`、`#[post]`、`#[controller]` 等宏注解，自动注册路由
    ///
    /// 编译期通过 `inventory` 收集所有被宏标注的处理器（不依赖链接器自定义段）。
    ///
    /// # 前缀行为
    ///
    /// 当 `[router] prefix` 非空时，所有扫描到的路由会整体嵌套到该前缀下
    /// （例如宏声明 `/system/user/page` + `prefix = "/api"` → 实际路径
    /// `/api/system/user/page`）；空前缀保持原样注册，向后兼容。
    ///
    /// 通过 [`App::get`] / [`App::post`] 等方法**手动注册**的路由不参与嵌套，
    /// 可用于豁免前缀（如根路径存活探针 `/health`）。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// #[alun::get("/hello")]
    /// async fn hello() -> Res<String> { Res::ok("Hi!") }
    ///
    /// App::new()?
    ///     .scan()
    ///     .start()
    ///     .await
    /// ```
    pub fn scan(mut self) -> Self {
        if self.prefix.is_empty() {
            for register in crate::discovered_routes() {
                if let Some(ref mut r) = self.router {
                    register(r);
                }
            }
            // 插件贡献的路由：与宏收集项合并（顺序 = 先宏后插件）。
            for plugin in &self.web_plugins {
                if let Some(ref mut r) = self.router {
                    plugin.routes(r);
                }
            }
        } else {
            // 前缀非空：先把宏声明的路由收集到独立子路由，再整体 nest 到前缀下
            let prefix = self.prefix.clone();
            let mut sub = AlunRouter::new();
            for register in crate::discovered_routes() {
                register(&mut sub);
            }
            // 插件贡献的路由同样受前缀约束：否则插件路由会意外绕过全局前缀。
            for plugin in &self.web_plugins {
                plugin.routes(&mut sub);
            }
            if let Some(ref mut r) = self.router {
                r.merge(&prefix, sub);
            }
        }
        self
    }

    /// 合并子路由（与 `group` 类似，但接受已构建好的 AlunRouter）
    pub fn merge(mut self, prefix: &str, sub: AlunRouter) -> Self {
        if let Some(ref mut r) = self.router {
            r.merge(prefix, sub);
        }
        self
    }

    /// 注册需要**特定权限**的 GET 路由 —— 方法级权限拦截
    ///
    /// 对标 axum 的 `get(handler).route_layer(RequirePermissionLayer)`。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// App::new()?
    ///     .with_permission("GET", "/api/admin/stats", admin_stats, "admin:access")
    ///     .start()
    ///     .await
    /// ```
    pub fn with_permission<H, T>(
        mut self,
        method: &str,
        path: &str,
        handler: H,
        permission: &str,
    ) -> Self
    where
        H: axum::handler::Handler<T, ()>,
        T: 'static,
    {
        let perm = permission.to_string();
        if let Some(ref mut r) = self.router {
            let wrap = move |mr: axum::routing::MethodRouter<()>| {
                mr.route_layer(mw::RequirePermissionLayer::any(vec![perm]))
            };
            match method.to_uppercase().as_str() {
                "GET" => r.add_get_with_layer(path, handler, wrap),
                "POST" => r.add_post_with_layer(path, handler, wrap),
                "PUT" => r.add_put_with_layer(path, handler, wrap),
                "PATCH" => r.add_patch_with_layer(path, handler, wrap),
                "DELETE" => r.add_delete_with_layer(path, handler, wrap),
                _ => r.add_get_with_layer(path, handler, wrap),
            };
        }
        self
    }

    /// 注册需要**特定角色**的 GET 路由 —— 方法级角色拦截
    ///
    /// # 示例
    ///
    /// ```ignore
    /// App::new()?
    ///     .with_role("GET", "/api/admin/users", list_users, "admin")
    ///     .start()
    ///     .await
    /// ```
    pub fn with_role<H, T>(mut self, method: &str, path: &str, handler: H, role: &str) -> Self
    where
        H: axum::handler::Handler<T, ()>,
        T: 'static,
    {
        let rl = role.to_string();
        if let Some(ref mut r) = self.router {
            let wrap = move |mr: axum::routing::MethodRouter<()>| {
                mr.route_layer(mw::RequireRoleLayer::any(vec![rl]))
            };
            match method.to_uppercase().as_str() {
                "GET" => r.add_get_with_layer(path, handler, wrap),
                "POST" => r.add_post_with_layer(path, handler, wrap),
                "PUT" => r.add_put_with_layer(path, handler, wrap),
                "PATCH" => r.add_patch_with_layer(path, handler, wrap),
                "DELETE" => r.add_delete_with_layer(path, handler, wrap),
                _ => r.add_get_with_layer(path, handler, wrap),
            };
        }
        self
    }

    // ── 扩展（插件） ──

    /// 注册插件（数据库、缓存、Kafka 等）
    ///
    /// 插件在 `start()` 时按拓扑顺序自动启动，在 shutdown 时逆序关闭。
    ///
    /// 若插件还要贡献路由 / 权限 / 免鉴权路径，请改用 [`App::web_plugin`]。
    pub fn plugin<P: alun_core::Plugin + 'static>(mut self, plugin: P) -> Self {
        self.plugins = self.plugins.add(plugin);
        self
    }

    /// 注册 **Web 插件**：生命周期 + 路由 / 权限 / 免鉴权贡献，一次注册全部生效
    ///
    /// 与 [`App::plugin`] 的区别：本方法要求实现 [`crate::WebPlugin`]，
    /// 因而在生命周期之外，还能在装配阶段贡献：
    ///
    /// - [`WebPlugin::routes`] —— 在 `scan()` 时与宏收集的路由合并；
    /// - [`WebPlugin::permission_rules`] —— 与 `#[alun::permission]` 规则合并；
    /// - [`WebPlugin::no_auth_paths`] —— 与 `#[alun::no_auth]` 路径合并。
    ///
    /// 这是**库形态分发**的唯一可靠通道：一个 crate 无法指望使用方的
    /// `inventory` 收集到自己的 handler（收集发生在最终二进制的链接单元里），
    /// 但可以导出 `WebPlugin` 实现，由使用方一行注册。
    ///
    /// 生命周期与装配作用于**同一个实例**（内部以 `Arc` 共享，非两份副本）。
    pub fn web_plugin<P: crate::plugin::WebPlugin + 'static>(mut self, plugin: P) -> Self {
        let shared = Arc::new(plugin);
        self.web_plugins
            .push(shared.clone() as Arc<dyn crate::plugin::WebPlugin>);
        self.plugins = self.plugins.add(crate::plugin::WebPluginAdapter(shared));
        self
    }

    /// 注入编译期嵌入的迁移（`migrate!` 宏产物）。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// App::new()?
    ///     .migrate_embedded(migrate!("migrations"))
    ///     .start()
    ///     .await
    /// ```
    ///
    /// 注入后 `[database.migration]` 的目录来源（`path` / `sources` / `seed_path`）
    /// 不再使用——运行时不再依赖 `migrations/` 目录，部署产物自包含；执行选项
    /// （同步提交 / 单事务 / 指纹校验 / 前缀等）仍由配置统一提供，并**覆盖**
    /// 注入器自带的（默认）选项。
    #[cfg(feature = "db")]
    pub fn migrate_embedded(mut self, migrator: alun_db::Migrator) -> Self {
        self.embedded_migrator = Some(migrator);
        self
    }

    /// 注册自定义启动钩子
    ///
    /// 钩子在全局资源初始化之后、插件启动之前调用，可在此阶段初始化自定义全局资源。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// App::new()?
    ///     .on_startup(|| async {
    ///         init_my_globals();
    ///     })
    ///     .scan()
    ///     .start()
    ///     .await
    /// ```
    pub fn on_startup<F, Fut>(mut self, hook: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        // 追加而非覆盖：多个模块各自注册启动钩子时互不影响
        self.startup_hooks.push(Box::new(|| Box::pin(hook())));
        self
    }

    /// 以闭包（`Router -> Router`）形式挂载**链外**中间件。
    ///
    /// 等价于 axum 的 `Router::layer(from_fn(f))`：`f` 接收整棵已装配好的 Router，
    /// 返回包装后的 Router。需要 `nest` / `merge` 这类 Router 级变换时用它；
    /// 能直接挂 `tower::Layer` 时用 [`App::layer`] 更省事。
    ///
    /// **可注册多个**，且与 [`App::layer`] **共用同一有序序列**：相对顺序即注册顺序，
    /// 先注册者更靠内层（F4 的统一扩展点，不再有"钩子恒先于 Layer"的隐式分组）。
    ///
    /// # 位置与能力（重要）
    ///
    /// 本序列在 `start()` 中于内置中间件链、`/metrics`、fallback **之后**应用，因此：
    ///
    /// - 能看到**全部**拒绝：`AuthLayer` 的 401、`PermissionCheckLayer` 的 403、
    ///   `RateLimitLayer` 的 429，以及 404 / 静态文件；能测到真实端到端耗时；
    /// - **读不到身份**：本层执行时 `AuthLayer` 尚未运行，`AuthClaims` 不在请求扩展中。
    ///
    /// 需要「既是全局层、又能读身份」时用 [`App::route_layer`]；需要在处理器里拿身份
    /// 时用提取器（`AuthClaims` / `Principal`）。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// App::new()?
    ///     .layer_from_fn(|router| {
    ///         router.layer(axum::middleware::from_fn(my_middleware))
    ///     })
    ///     .scan()
    ///     .start()
    ///     .await
    /// ```
    pub fn layer_from_fn<F>(mut self, f: F) -> Self
    where
        F: FnOnce(Router) -> Router + Send + 'static,
    {
        // 追加而非覆盖：避免后注册的静默吃掉先注册的
        self.router_extensions.push(Box::new(f));
        self
    }

    /// 注册自定义中间件注入钩子 —— **已更名为 [`App::layer_from_fn`]**。
    ///
    /// 行为完全一致（同一序列、同一位置、同一能力边界），保留本名仅为兼容既有代码，
    /// 请迁移到 [`App::layer_from_fn`]。
    #[deprecated(note = "已更名为 `App::layer_from_fn`，语义与行为完全一致")]
    pub fn with_middleware_hook<F>(self, hook: F) -> Self
    where
        F: FnOnce(Router) -> Router + Send + 'static,
    {
        self.layer_from_fn(hook)
    }

    /// 直接挂载一个 tower Layer —— 作用于**整站**（含 fallback 与 `/metrics`）。
    ///
    /// 与 [`App::layer_from_fn`] **共用同一有序序列**，无需手写闭包，
    /// 适合直接挂载现成的 `tower` / `tower-http` 中间件：
    ///
    /// ```ignore
    /// App::new()?
    ///     .layer(tower_http::timeout::TimeoutLayer::new(Duration::from_secs(30)))
    ///     .scan()
    ///     .start()
    ///     .await
    /// ```
    ///
    /// # 类型约束
    ///
    /// 泛型约束与 `axum::Router::layer` **完全一致**（作用对象是
    /// `axum::routing::Route`），因此可以直接挂载任何 `tower` / `tower-http`
    /// 现成中间件，无需手写闭包。同理，层**不能返回 tower 错误**（`Error` 必须可转为
    /// `Infallible`）——需要拒绝请求时直接返回响应（如 `ApiError` 的 `IntoResponse`），
    /// 这也是框架内置层的统一做法（不引入 `HandleErrorLayer` 第二条错误路径）。
    ///
    /// # 位置与能力
    ///
    /// 位于**内置链之外**、整站最外层：能看到全部拒绝与真实端到端耗时，但**读不到身份**。
    /// 需要身份请用 [`App::route_layer`] 或提取器；需要只给某个前缀下的一整组路由挂层，
    /// 用 [`AlunRouter::merge_with_layer`](crate::AlunRouter::merge_with_layer)。
    pub fn layer<L>(mut self, layer: L) -> Self
    where
        L: tower::Layer<axum::routing::Route> + Clone + Send + Sync + 'static,
        L::Service: tower::Service<axum::extract::Request> + Clone + Send + Sync + 'static,
        <L::Service as tower::Service<axum::extract::Request>>::Response:
            axum::response::IntoResponse + 'static,
        <L::Service as tower::Service<axum::extract::Request>>::Error:
            Into<std::convert::Infallible> + 'static,
        <L::Service as tower::Service<axum::extract::Request>>::Future: Send + 'static,
    {
        self.router_extensions
            .push(Box::new(move |router: Router| router.layer(layer)));
        self
    }

    /// 挂载**链内**中间件 —— 只作用于匹配到的路由，位于全部内置中间件之内。
    ///
    /// 语义对标 axum 的 `Router::route_layer`：**不含** fallback（404 / 静态文件）、
    /// 不含 `/metrics`；且本层在 `build_middleware_chain` **之前**应用，因此位于
    /// Auth / 权限 / 限流等**全部内置层之内**。
    ///
    /// # 与 [`App::layer`] 的分工
    ///
    /// | | [`App::layer`]（链外） | `route_layer`（链内） |
    /// |---|---|---|
    /// | 覆盖范围 | 整站（含 fallback、`/metrics`） | 仅匹配到的路由 |
    /// | 读 `AuthClaims` / 租户上下文 | ✗ | ✓ |
    /// | 看到 401 / 403 / 429 | ✓ | ✗ |
    /// | 端到端耗时 | 真实（含全部内置层） | 不含内置层开销 |
    ///
    /// 要「按身份做事」的全局中间件（数据权限、字段脱敏、按用户埋点）用本方法；
    /// 审计 / 访问日志必须用 [`App::layer`]，否则丢掉 401 / 403 / 429 的记录。
    ///
    /// 需要只给某个前缀下的一整组路由挂层时，用
    /// [`AlunRouter::merge_with_layer`](crate::AlunRouter::merge_with_layer)；
    /// 只给单条路由挂层用
    /// [`AlunRouter::add_get_with_layer`](crate::AlunRouter::add_get_with_layer) 等。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// App::new()?
    ///     .route_layer(DataScopeLayer::new())
    ///     .scan()
    ///     .start()
    ///     .await
    /// ```
    pub fn route_layer<L>(mut self, layer: L) -> Self
    where
        L: tower::Layer<axum::routing::Route> + Clone + Send + Sync + 'static,
        L::Service: tower::Service<axum::extract::Request> + Clone + Send + Sync + 'static,
        <L::Service as tower::Service<axum::extract::Request>>::Response:
            axum::response::IntoResponse + 'static,
        <L::Service as tower::Service<axum::extract::Request>>::Error:
            Into<std::convert::Infallible> + 'static,
        <L::Service as tower::Service<axum::extract::Request>>::Future: Send + 'static,
    {
        self.route_layers
            .push(Box::new(move |router: Router| router.layer(layer)));
        self
    }

    // ── 显式模块注册（双保险：不依赖任何编译期收集机制）──
    //
    // 背景：`#[alun::get]` + `App::scan()` 通过 `inventory` 在编译期收集路由。
    // 该机制不依赖链接器自定义段，任何工具链都成立（F19：早前用的
    // 早前的 `linkme::distributed_slice` 在 windows-gnu 下会被 `--gc-sections` 整段回收，
    // 导致切片为空、路由全部丢失且无报错，现已弃用）。
    //
    // 下面这套 API 是**完全显式**的等价通道：由 `#[alun::routes]` 宏扫描模块
    // 生成一个 `fn(App) -> App` 的注册函数，`App::module()` 直接调用它。
    // 适用于：需要完全掌控装配顺序，或库形态分发（不想依赖任何收集机制）。

    /// 挂载一个**显式注册模块**。
    ///
    /// 模块注册函数由 `#[alun::routes]` 宏生成，形如 `fn(app: App) -> App`：
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
    pub fn module<F>(self, register: F) -> Self
    where
        F: FnOnce(Self) -> Self,
    {
        register(self)
    }

    /// 显式注册一组路由（不依赖任何宏）。
    ///
    /// 适合在循环 / 条件中动态拼装路由的场景；与 [`App::scan`] 可混用。
    ///
    /// ```ignore
    /// fn my_routes(r: &mut AlunRouter) {
    ///     r.add_get("/ping", || async { Res::ok("pong") });
    /// }
    /// App::new()?.routes(my_routes).start().await?;
    /// ```
    pub fn routes(mut self, register: fn(&mut AlunRouter)) -> Self {
        if let Some(router) = self.router.as_mut() {
            register(router);
        }
        self
    }

    /// 追加权限规则（与 `#[alun::permission]` 收集到的规则**合并**）。
    ///
    /// 由 `#[alun::routes]` 宏生成的模块会自动调用本方法，通常无需手写。
    pub fn permission_defs(mut self, defs: Vec<crate::PermissionDef>) -> Self {
        self.extra_permissions.extend(defs);
        self
    }

    /// 追加免认证路径（与 `#[alun::no_auth]` 收集到的路径**合并**）。
    ///
    /// 由 `#[alun::routes]` 宏生成的模块会自动调用本方法，通常无需手写。
    pub fn no_auth_defs(mut self, defs: Vec<crate::NoAuthDef>) -> Self {
        self.extra_no_auth.extend(defs);
        self
    }

    /// 构建并返回完整的 `axum::Router`（含全部中间件链），**不启动服务器**。
    ///
    /// 两个用途：
    /// 1. **测试**：直接对 Router 发请求，无需绑定端口（配合 `tower::ServiceExt::oneshot`）；
    /// 2. **嵌入**：把 alun 应用作为子路由挂到已有的 axum 应用上。
    ///
    /// 装配顺序与 [`App::start`] **完全一致**（链内层 → 中间件链 → `/metrics` →
    /// 404 兜底 → 链外层），因此自定义层（[`App::route_layer`] / [`App::layer`] /
    /// [`App::layer_from_fn`]）在本方法产出的 Router 上同样生效——
    /// 「测试通过」与「线上行为」不会出现两套。
    ///
    /// 唯一的差别：**静态文件服务不在此挂载**（`ServeDir` 建立目录属于文件系统副作用，
    /// 不适合测试 / 嵌入路径）。需要时请由嵌入方自行 `fallback_service`。
    ///
    /// 注意：本方法不初始化全局资源（`db()` / `cache()` 等仍是未初始化状态）。
    /// 若需要在测试中使用这些全局资源，请先调用 [`crate::resources::set_db`] 等。
    /// 同理，`AuthClaims` 提取器的**通道 2**（回退验签）依赖全局配置，
    /// 在未调用 [`crate::resources::set_config`] 的场景下请走通道 1（先经过认证层）。
    pub fn into_router(mut self) -> Router {
        let inner = self.router.take().unwrap_or_default().into_axum();

        // 1. 链内层（与 `start()` 同序）：必须在建链之前应用，才能位于内置链之内
        let mut router = inner;
        for layer in std::mem::take(&mut self.route_layers) {
            router = layer(router);
        }

        // 2. 内置中间件链
        let mut router = self.build_middleware_chain(router);

        // 3. 指标端点（不穿中间件）
        router = self.attach_metrics_endpoint(router);

        // 4. 404 兜底
        if let Some(ref cm) = self.config_mgr {
            let cfg = cm.get();
            if cfg.router.not_found.enabled {
                router = router.fallback(Self::handle_not_found);
            }
        }

        // 5. 链外层（与 `start()` 同序）
        let extensions = std::mem::take(&mut self.router_extensions);
        for ext in extensions {
            router = ext(router);
        }

        router
    }

    /// F10：把 `/metrics` 端点挂到 Router 上。
    ///
    /// 必须在**中间件链构建完成之后**调用：axum 的 `layer` 只作用于调用时已
    /// 存在的路由，因此这条后加的路由不穿鉴权 / 限流 / 请求日志——抓取器
    /// 无需 Token。代价是访问控制由端点自己按 `[metrics].allow_ips` 完成。
    fn attach_metrics_endpoint(&self, router: Router) -> Router {
        let enabled = self
            .config_mgr
            .as_ref()
            .map(|cm| cm.get().metrics.enabled)
            .unwrap_or(false);

        #[cfg(feature = "metrics-prometheus")]
        {
            if enabled && let Some(cm) = &self.config_mgr {
                return crate::metrics::attach_endpoint(router, &cm.get().metrics);
            }
        }

        // fail-loud：配置声明了端点，但产物没编译导出 feature。
        // 静默忽略会让运维以为"指标已经在采集"，直到排查时才发现端点 404。
        #[cfg(not(feature = "metrics-prometheus"))]
        if enabled {
            tracing::warn!(
                "[metrics].enabled = true，但产物未启用 `metrics-prometheus` feature：\
                 `/metrics` 端点不会挂载。请确认 Cargo.toml 的 features"
            );
        }

        router
    }

    /// 启动应用——无参，端口从 config.toml 的 `[server]` 节读取
    pub async fn start(mut self) -> Result<()> {
        let startup_start = Instant::now();

        if self.settings.gen_config_only {
            let dir = self.settings.config_path.as_deref().unwrap_or("config");
            ConfigManager::generate_default(dir)?;
            return Ok(());
        }

        if self.config_mgr.is_none() {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "alun=info".into()),
                )
                .try_init();
        }

        if self.settings.print_config
            && let Some(ref cm) = self.config_mgr
            && let Ok(toml_str) = toml::to_string_pretty(cm.get())
        {
            println!("{}", toml_str);
        }

        // 1.0. F10：提前**并行**初始化 Prometheus recorder。
        //      `build_recorder` 内的 quanta 首次校准最长可占 200ms（本机实测
        //      1ms ↔ 208ms 双峰），而它与数据库连接 / 迁移 / 启动钩子没有依赖。
        //      在初始化全局资源**之前** spawn，可将这段固定开销藏进数据库握手
        //      与迁移耗时里；真正需要端点就绪时再 await（见 2.5）。
        #[cfg(feature = "metrics-prometheus")]
        let metrics_task = self
            .config_mgr
            .as_ref()
            .map(|cm| cm.get().metrics.clone())
            .filter(|mc| mc.enabled)
            .map(|mc| {
                let task_cfg = mc.clone();
                (
                    mc,
                    tokio::spawn(async move { crate::metrics::install_prometheus(&task_cfg) }),
                )
            });

        // 1. 先初始化全局资源（数据库、缓存、配置、模板）
        //    编译期嵌入的迁移器（`migrate_embedded` 注入）随初始化一并传入。
        #[cfg(feature = "db")]
        let embedded_migrator = self.embedded_migrator.take();
        #[cfg(not(feature = "db"))]
        let embedded_migrator = ();
        if let Some(ref cm) = self.config_mgr {
            Self::init_global_resources(cm, embedded_migrator).await?;
        }

        // 1.1. 同步多租户全局开关：`#[tenant]` 属性仅在 `[tenant].enabled = true` 时生效，
        // 否则完全惰性（存量系统零回归）；写路径另有独立灰度开关，见 1.2。
        #[cfg(feature = "db")]
        if let Some(ref cm) = self.config_mgr {
            let tenant = cm.get().tenant.clone();
            alun_db::tenant::set_tenant_enabled(tenant.enabled);
            alun_db::tenant::set_tenant_write_enabled(tenant.write_enabled);
            // 1.2. PostgreSQL schema-per-tenant（库级隔离）：仅在 PostgreSQL 生效，
            // 由执行器在借出连接后按租户 `SET search_path`。
            #[cfg(feature = "postgres")]
            {
                alun_db::tenant::set_tenant_schema_enabled(tenant.schema_per_tenant);
                alun_db::tenant::set_tenant_schema_prefix(&tenant.schema_prefix);
                if tenant.enabled && tenant.schema_per_tenant {
                    tracing::info!(
                        "多租户 schema-per-tenant 已启用 prefix={}（仅 PostgreSQL 生效）",
                        tenant.schema_prefix
                    );
                    // F17：`#[model(schema = "...")]`（静态 schema）与 schema_per_tenant
                    // （按租户动态 search_path）是两条并存机制，优先级此前只存在于文档。
                    // 在**启动期**显式告警，避免运维/开发误判隔离层级。
                    tracing::warn!(
                        "检测到 `schema_per_tenant = true`：请确认它不与模型上的 \
                         `#[model(schema = \"...\")]` 混用。二者语义为——静态 `#[model(schema)]` \
                         用于把表固定到某 schema（跨 schema 查询时该表按 cross-tenant 语义处理）；\
                         动态 `schema_per_tenant` 按租户在**连接级** `SET search_path`，优先级更高。\
                         通常应二选一；确需混用时请逐一核对每张表的 schema 归属。详见 docs/07、docs/09。"
                    );
                }
            }
            if tenant.enabled && !tenant.write_enabled {
                tracing::warn!(
                    "多租户写路径租户补齐已关闭（[tenant].write_enabled = false）：\
                     写操作不注入 / 不校验租户，仅用于迁移观察期，请确认后尽快打开"
                );
            }
        }

        // 1.5. 执行自定义启动钩子（可初始化自定义全局资源）
        //      按注册顺序执行；多模块各自注册时互不覆盖。
        let startup_hooks = std::mem::take(&mut self.startup_hooks);
        if !startup_hooks.is_empty() {
            info!(
                "执行 {} 个自定义启动钩子（按注册顺序）",
                startup_hooks.len()
            );
        }
        for hook in startup_hooks {
            (hook)().await;
        }

        // 2. 再启动插件，此时插件可以安全访问全局资源
        self.plugins
            .check_duplicate_names()
            .map_err(alun_core::Error::Config)?;
        let plugins_started = Instant::now();
        self.plugins.start_all().await?;
        info!(
            elapsed_ms = plugins_started.elapsed().as_millis(),
            "插件启动完成"
        );

        // 2.5. F10：Prometheus recorder 就绪处理（初始化已在 1.0 与启动流程并行）。
        #[cfg(feature = "metrics-prometheus")]
        if let Some((mc, task)) = metrics_task {
            if mc.blocking_startup {
                let wait_started = Instant::now();
                match task.await {
                    Ok(Ok(())) => {
                        let scope = if mc.allow_ips.is_empty() {
                            "仅本机回环（[metrics].allow_ips 为空）".to_string()
                        } else {
                            format!("白名单 {}", mc.allow_ips.join(", "))
                        };
                        info!(
                            "指标端点已启用 path={} 访问范围：{}（等待就绪 {}ms，初始化与资源装配并行）",
                            mc.path,
                            scope,
                            wait_started.elapsed().as_millis()
                        );
                    }
                    Ok(Err(e)) => return Err(e),
                    Err(join_err) => {
                        return Err(alun_core::Error::Config(format!(
                            "metrics 初始化任务异常退出: {join_err}"
                        )));
                    }
                }
            } else {
                // 非阻塞模式：初始化继续在后台完成，监听不等待。
                // 安装完成前 `/metrics` 返回 404；安装失败只记日志（端点保持不可用）。
                tokio::spawn(async move {
                    match task.await {
                        Ok(Ok(())) => info!(
                            "指标端点后台就绪 path={}（未阻塞启动，安装前的指标不采集）",
                            mc.path
                        ),
                        Ok(Err(e)) => {
                            tracing::error!(error = %e, "指标端点后台初始化失败，/metrics 将保持 404")
                        }
                        Err(join_err) => {
                            tracing::error!(error = %join_err, "metrics 初始化任务异常退出")
                        }
                    }
                });
            }
        }

        // 3. 构建 Router 和中间件（分段打点，E 方案：定位构建热点）
        let router_build_started = Instant::now();
        let router = self.router.take().unwrap_or_default();
        let mut axum_router: Router = router.into_axum();
        let into_axum_ms = router_build_started.elapsed().as_millis();

        // 链内层（`App::route_layer`）：必须在 `build_middleware_chain` **之前**应用，
        // 这样后续添加的内置层会包裹在它外面，本层便位于 Auth / 权限 / 限流之内，
        // 从而能读到 `AuthClaims` 与租户上下文。
        // 同时此刻尚无 fallback 与 `/metrics`，因此本层天然只作用于匹配到的路由
        // （与 axum `Router::route_layer` 语义一致）。
        let route_layers = std::mem::take(&mut self.route_layers);
        if !route_layers.is_empty() {
            info!(
                "应用链内层 {} 个（仅匹配路由，位于内置链之内，可读身份）",
                route_layers.len()
            );
        }
        for layer in route_layers {
            axum_router = layer(axum_router);
        }
        let route_layers_ms = router_build_started.elapsed().as_millis();

        axum_router = self.build_middleware_chain(axum_router);
        let middleware_chain_ms = router_build_started.elapsed().as_millis();
        // 端点必须在链之后挂载（不穿中间件，见方法文档）
        axum_router = self.attach_metrics_endpoint(axum_router);
        let metrics_endpoint_ms = router_build_started.elapsed().as_millis();

        // 静态文件服务 + 自定义 404 处理
        if let Some(ref cm) = self.config_mgr {
            let cfg = cm.get();
            if cfg.static_files.enabled {
                let static_path = cfg.static_files.path.clone();
                std::fs::create_dir_all(&static_path).ok();
                info!("静态文件服务就绪 path={}", static_path);
                axum_router = axum_router.fallback_service(ServeDir::new(&static_path));
            } else if cfg.router.not_found.enabled {
                axum_router = axum_router.fallback(Self::handle_not_found);
            }
        }
        let static_files_ms = router_build_started.elapsed().as_millis();

        // 链外 Router 扩展序列：按注册顺序应用。
        // 多模块各自注册时不再互相覆盖（历史上 `Option` 会静默丢弃先注册者）；
        // `layer_from_fn` 与 `layer` 共用此序列，相对顺序即注册顺序（F4）。
        // 注意：整条序列在此处应用 ⇒ 全部位于内置链之外，早于 AuthLayer，
        // 因此读不到 `AuthClaims`（需要身份请用提取器或 `App::route_layer`）。
        let extensions = std::mem::take(&mut self.router_extensions);
        if !extensions.is_empty() {
            info!(
                "应用 Router 扩展 {} 个（均位于内置链之外，内部按注册顺序，先注册者更靠内层）",
                extensions.len()
            );
        }
        for ext in extensions {
            axum_router = ext(axum_router);
        }
        let router_extensions_ms = router_build_started.elapsed().as_millis();
        info!(
            into_axum_ms,
            route_layers_ms,
            middleware_chain_ms,
            metrics_endpoint_ms,
            static_files_ms,
            router_extensions_ms,
            "Router 构建分段耗时（累计毫秒）"
        );

        let bind_addr = self
            .config_mgr
            .as_ref()
            .map(|cm| cm.get().server.listen.clone())
            .unwrap_or_else(|| "0.0.0.0:0".to_string());

        let socket_addr = parse_addr(&bind_addr)?;
        let display_addr = resolve_display_addr(socket_addr);
        let app_name = self
            .config_mgr
            .as_ref()
            .map(|cm| cm.get().app_name.as_str())
            .unwrap_or("Alun");
        info!("{} 启动 -> http://{}", app_name, display_addr);
        if let Some(cm) = &self.config_mgr {
            info!(
                "  profile={}, request_id={} log={} cors={} compression={} rate_limit={} jwt_auth={} idempotency={} nonce={} static_files={} not_found={}",
                cm.get().profile,
                cm.get().middleware.request_id,
                cm.get().middleware.request_log,
                cm.get().middleware.cors.enabled,
                cm.get().middleware.compression.enabled,
                cm.get().middleware.rate_limit.enabled,
                cm.get().middleware.auth.enabled,
                cm.get().middleware.idempotency.enabled,
                cm.get().middleware.nonce.enabled,
                cm.get().static_files.enabled,
                cm.get().router.not_found.enabled,
            );
        }

        let startup_ms = startup_start.elapsed().as_millis();
        info!("{} 启动完成, 耗时 {}ms", app_name, startup_ms);

        let listener = tokio::net::TcpListener::bind(socket_addr).await?;
        axum::serve(
            listener,
            axum_router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await?;

        self.plugins.stop_all().await;
        Ok(())
    }

    /// 指定监听地址启动（用于需要自定义地址的场景）
    pub async fn serve(self, addr: impl Into<String>) -> Result<()> {
        let mut s = self;
        let addr_str = addr.into();
        if s.config_mgr.is_none() {
            let default_cfg = AppConfig {
                server: alun_config::ServerConfig {
                    listen: addr_str.clone(),
                },
                ..Default::default()
            };
            s.config_mgr = Some(Arc::new(ConfigManager {
                static_config: default_cfg,
                dynamic: parking_lot::RwLock::new(HashMap::new()),
            }));
        } else if let Some(ref cm) = s.config_mgr {
            let mut cfg = cm.get().clone();
            cfg.server.listen = addr_str.clone();
            s.config_mgr = Some(Arc::new(ConfigManager {
                static_config: cfg,
                dynamic: parking_lot::RwLock::new(HashMap::new()),
            }));
        }
        s.start().await
    }

    /// 初始化全局资源（数据库、缓存、配置、模板）
    ///
    /// `embedded_migrator`：由 [`App::migrate_embedded`] 注入的编译期嵌入迁移器
    /// （`migrate!` 宏产物）；`db` 特性关闭时为占位参数（该路径不使用迁移）。
    async fn init_global_resources(
        cm: &Arc<ConfigManager>,
        #[cfg(feature = "db")] embedded_migrator: Option<alun_db::Migrator>,
        #[cfg(not(feature = "db"))] _embedded_migrator: (),
    ) -> Result<()> {
        set_config(cm.clone()).map_err(|e| alun_core::Error::Config(e.to_string()))?;
        let cfg = cfg();

        #[cfg(feature = "db")]
        if cfg.database.enabled {
            // 新版 alun-db 采用 `Db::connect(url)` 建连 + 连接测试，
            // URL 由配置层（db_url 模块）针对 postgres/mysql/sqlite 构造。
            let db_config = match crate::db_url::build_config(&cfg.database) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("数据库连接 URL 构造失败: {}", e);
                    return Err(alun_core::Error::Config(format!(
                        "数据库连接 URL 构造失败: {}",
                        e
                    )));
                }
            };

            // `connect_with` 需同时借 url 并 move config，故先克隆 url 引用
            let url = db_config.url.clone();
            match alun_db::Db::connect_with(&url, db_config).await {
                Ok(db) => {
                    info!("数据库连接成功");

                    if cfg.database.migration.enabled && cfg.database.migration.auto_migrate {
                        // 迁移来源：`sources` 非空时**取代** `path`（多目录合并 + 子目录递归）；
                        // 两者都为空则退化为空清单，业务方可自行注入编程式迁移。
                        let migration_cfg = &cfg.database.migration;
                        let sources: Vec<std::path::PathBuf> = if !migration_cfg.sources.is_empty()
                        {
                            migration_cfg
                                .sources
                                .iter()
                                .map(|dir| dir.trim())
                                .filter(|dir| !dir.is_empty())
                                .map(std::path::PathBuf::from)
                                .collect()
                        } else if migration_cfg.path.trim().is_empty() {
                            Vec::new()
                        } else {
                            vec![std::path::PathBuf::from(migration_cfg.path.trim())]
                        };

                        // 策略字符串解析失败必须中止启动：拼错的策略名绝不能静默降级
                        let divergent = match alun_db::Divergent::from_policy(
                            &migration_cfg.divergent_policy,
                        ) {
                            Ok(divergent) => divergent,
                            Err(e) => {
                                tracing::error!("迁移策略配置有误: {}", e);
                                return Err(alun_core::Error::Config(format!(
                                    "迁移策略配置有误: {}",
                                    e
                                )));
                            }
                        };
                        let baseline_version = match migration_cfg.baseline_version.trim() {
                            "" => None,
                            raw => Some(alun_db::Version::new(raw)),
                        };

                        let options = alun_db::MigratorOptions {
                            split_statements: migration_cfg.statement_split,
                            up_down_sections: migration_cfg.up_down_sections,
                            version_prefix: migration_cfg.version_prefix.clone(),
                            name_separator: migration_cfg.name_separator.clone(),
                            repeatable_prefix: migration_cfg.repeatable_prefix.clone(),
                            seed_prefix: migration_cfg.seed_prefix.clone(),
                            baseline_prefix: migration_cfg.baseline_prefix.clone(),
                            transaction_default: migration_cfg.transaction_default,
                            validate_checksum: migration_cfg.validate_checksum,
                            divergent,
                            out_of_order: migration_cfg.out_of_order,
                            import_legacy: migration_cfg.import_legacy,
                            baseline_on_migrate: migration_cfg.baseline_on_migrate,
                            baseline_version,
                            synchronous_commit: {
                                let raw = migration_cfg.synchronous_commit.trim();
                                (!raw.is_empty()).then(|| raw.to_string())
                            },
                            single_transaction: migration_cfg.single_transaction,
                            placeholders: migration_cfg.placeholders.clone(),
                            // 启动路径**绝不**允许破坏性操作：`reset` / `fresh`
                            // 只能由代码显式开启，避免配置失误重建生产库
                            is_destructive_allowed: false,
                        };
                        // 来源：优先使用 [`App::migrate_embedded`] 注入的编译期嵌入迁移
                        // （`migrate!` 宏产物，运行时不再依赖 migrations/ 目录）；否则
                        // 按配置的目录来源构建。两条路径共用同一 options（配置统一覆盖）。
                        let migrator = match embedded_migrator {
                            Some(injected) => {
                                info!("迁移来源：编译期嵌入（migrate! 宏），跳过目录扫描");
                                injected.options(options)
                            }
                            None => {
                                let mut built = alun_db::Migrator::new().options(options);
                                for dir in &sources {
                                    built = built.source_dir(dir.clone());
                                }
                                // 种子目录：未配置即不跑；配置了就必须存在（缺失由 Migrator 报错）
                                if !migration_cfg.seed_path.trim().is_empty() {
                                    built = built
                                        .source_seed(migration_cfg.seed_path.trim().to_string());
                                }
                                built
                            }
                        };

                        match migrator.run(&db).await {
                            Ok(n) => info!("数据库迁移完成: 应用 {} 个迁移", n),
                            Err(e) => {
                                tracing::error!("数据库迁移失败: {}", e);
                                return Err(alun_core::Error::Config(format!(
                                    "数据库迁移失败: {}",
                                    e
                                )));
                            }
                        }
                    }

                    set_db(db).map_err(|e| alun_core::Error::Config(e.to_string()))?;
                }
                Err(e) => {
                    tracing::error!("数据库连接失败: {}", e);
                    return Err(alun_core::Error::Config(format!("数据库连接失败: {}", e)));
                }
            }
        }

        #[cfg(feature = "cache")]
        if cfg.cache.r#type != "local" || cfg.cache.max_capacity > 0 {
            match alun_cache::create_cache(&cfg.app_name, &cfg.cache, &cfg.redis).await {
                Ok(c) => {
                    set_cache(c).map_err(|e| alun_core::Error::Config(e.to_string()))?;
                }
                Err(e) => {
                    tracing::warn!("缓存初始化失败: {}，将不使用缓存", e);
                }
            }
        }

        #[cfg(feature = "template")]
        {
            match alun_template::TemplateEngine::from_dir(&cfg.template.path) {
                Ok(engine) => {
                    info!("模板引擎就绪 path={}", cfg.template.path);
                    set_template(engine).map_err(|e| alun_core::Error::Config(e.to_string()))?;
                }
                Err(e) => {
                    tracing::warn!("模板引擎初始化失败: {}，将使用空引擎", e);
                    let _ = set_template(alun_template::TemplateEngine::new());
                }
            }
        }

        // 上传目录：自动创建，路径存入全局资源
        {
            let upload_path = &cfg.upload.path;
            std::fs::create_dir_all(upload_path).map_err(|e| {
                alun_core::Error::Config(format!("创建上传目录失败 '{}': {}", upload_path, e))
            })?;
            set_upload_path(upload_path.clone())
                .map_err(|e| alun_core::Error::Config(e.to_string()))?;
            info!(
                "上传目录就绪 path={} max_size_mb={}",
                upload_path, cfg.upload.max_size_mb
            );
        }

        // 下载目录：自动创建，路径存入全局资源
        {
            let download_path = &cfg.download.path;
            std::fs::create_dir_all(download_path).map_err(|e| {
                alun_core::Error::Config(format!("创建下载目录失败 '{}': {}", download_path, e))
            })?;
            set_download_path(download_path.clone())
                .map_err(|e| alun_core::Error::Config(e.to_string()))?;
            info!("下载目录就绪 path={}", download_path);
        }

        Ok(())
    }

    /// 自定义 404 处理：当请求路径无匹配路由时，返回 JSON 格式的统一错误响应
    ///
    /// 配置项 `[router.not_found]` 可控制是否启用及自定义消息内容。
    ///
    /// 这里用 `try_config()` 而非 `cfg()`：本处理函数会随
    /// [`App::into_router`] 一起下发给**未初始化全局资源**的测试 / 嵌入场景
    /// （`into_router` 的文档明确说明这一点）。若此处直接 `cfg()`，
    /// 一次普通的 404 就会 panic —— 「资源不存在」绝不应该变成进程崩溃。
    async fn handle_not_found() -> impl IntoResponse {
        let msg = crate::resources::try_config()
            .map(|cm| cm.get().router.not_found.message.clone())
            .unwrap_or_else(|| "资源不存在".to_string());
        (
            StatusCode::NOT_FOUND,
            Json(Res::<()>::fail(codes::NOT_FOUND, msg)),
        )
    }

    /// 构建中间件链
    ///
    /// axum 的 `router.layer()` 语义：后添加的 layer 包裹在外层。
    /// 因此添加顺序 = 从内到外，执行顺序 = 从外到内（后添加的先收到请求、后处理响应）。
    ///
    /// 执行顺序（从外到内，与下方 layer 添加顺序相反）：
    ///   SecurityHeaders → CORS → RequestLog → RequestId → Auth → Permission
    ///   → RateLimit → Compression → Nonce → Idempotency（按配置启用）→ handler
    ///
    /// 其中 Auth 在 RateLimit 外层：未认证请求先得 401，不消耗限流额度。
    ///
    /// 关键：CORS 和 SecurityHeaders 必须在 Auth 外层，
    /// 否则认证中间件返回 401 时短路，CORS 头缺失导致浏览器报 ERR_FAILED。
    fn build_middleware_chain(&self, router: Router) -> Router {
        // 编译期注册的权限规则（`#[alun::permission]` / `#[alun::routes]` 模块）
        // + 插件贡献的规则（F5）。
        // 收集为 `Vec` 后传入：规则来自 `inventory`（非切片），故不能直接借用。
        // 前置规则收集耗时（debug 级：排查装配热点用）
        let collect_started = std::time::Instant::now();
        let mut macro_rules = crate::discovered_permissions();
        for plugin in &self.web_plugins {
            macro_rules.extend(plugin.permission_rules());
        }
        let collect_ms = collect_started.elapsed().as_millis();
        let rule_count = macro_rules.len();
        let router = self.build_middleware_chain_with(router, &macro_rules);
        tracing::debug!(collect_ms, rule_count, "middleware_chain 前置规则收集完成");
        router
    }

    /// `build_middleware_chain` 的实现体；宏注解规则由调用方注入
    ///
    /// 生产入口传入编译期收集的权限规则（由 `#[permission]` 宏经 `inventory`
    /// 提交，见 [`crate::discovered_permissions`]），单元测试可传入合成规则，
    /// 从而覆盖"有规则 / 无规则"两条装配分支。
    fn build_middleware_chain_with(
        &self,
        mut router: Router,
        macro_rules: &[crate::PermissionDef],
    ) -> Router {
        if let Some(cm) = &self.config_mgr {
            let cfg = cm.get();
            // 逐层应用耗时（debug 级：排查装配热点，函数尾部汇总输出）
            let mut layer_timings: Vec<(&'static str, u128)> = Vec::new();

            // ── 请求体上限（最内层，覆盖全部路由）──
            //
            // axum 默认只放行 2 MiB。需要 multipart 或大 JSON 的业务一旦超出，body 流会在
            // 读取（`field.bytes()` / `Bytes`）中途被掐断，表现为「上传失败」这类难定位的
            // 错误。此处统一由 `[upload].max_size_mb` 推导（含 multipart 开销余量，见
            // `UploadConfig::max_request_body_bytes`）：业务不必各自挂载，且与业务侧
            // `Content-Length` 前置校验同源，不会出现「前置校验放行、硬顶却先掐断」的漂移。
            //
            // 位置任意：该层只往请求扩展里写入上限配置，与实际执行顺序无关；放在最内层
            // 可覆盖全部路由，且外层拒绝（401 / 403 / 429）不必先解析请求体。
            {
                let limit = cfg.upload.max_request_body_bytes();
                let t_apply = std::time::Instant::now();
                router = router.layer(axum::extract::DefaultBodyLimit::max(limit));
                layer_timings.push(("body_limit", t_apply.elapsed().as_millis()));
                info!(
                    "请求体上限就绪 max_request_bytes={}（由 [upload].max_size_mb={} + multipart 余量推导）",
                    limit, cfg.upload.max_size_mb
                );
            }

            // ── 内层（先添加）：业务逻辑中间件 ──

            // 幂等键 / Nonce 防重放：由配置声明式接入默认链（D017），未配置时完全不改变链。
            //
            // 位置：最内层（压缩之内、限流/认证/权限之外层会先处理请求）。
            // 必须在 Compression 之内：否则幂等层收到的是已压缩的响应体，
            // 缓存后再回放会丢失 `Content-Encoding` 头 → 客户端解压失败。
            // 处于认证/权限之内：未认证请求无法探测或消费他人的幂等键/nonce。
            #[cfg(feature = "cache")]
            {
                if cfg.middleware.idempotency.enabled {
                    match try_cache() {
                        Some(c) => {
                            let t_apply = std::time::Instant::now();
                            router = router.layer(mw::IdempotencyLayer::new(
                                Arc::new(c.clone()),
                                Duration::from_secs(cfg.middleware.idempotency.ttl_secs),
                            ));
                            layer_timings.push(("idempotency", t_apply.elapsed().as_millis()));
                            info!(
                                "幂等键中间件已启用 ttl={}s",
                                cfg.middleware.idempotency.ttl_secs
                            );
                        }
                        None => tracing::warn!(
                            "[middleware.idempotency] enabled=true 但缓存未初始化，跳过装配"
                        ),
                    }
                }

                if cfg.middleware.nonce.enabled {
                    match try_cache() {
                        Some(c) => {
                            let t_apply = std::time::Instant::now();
                            router = router.layer(mw::NonceLayer::new(
                                Arc::new(c.clone()),
                                Duration::from_secs(cfg.middleware.nonce.ttl_secs),
                            ));
                            layer_timings.push(("nonce", t_apply.elapsed().as_millis()));
                            info!(
                                "Nonce 防重放中间件已启用 ttl={}s",
                                cfg.middleware.nonce.ttl_secs
                            );
                        }
                        None => tracing::warn!(
                            "[middleware.nonce] enabled=true 但缓存未初始化，跳过装配"
                        ),
                    }
                }
            }

            if cfg.middleware.compression.enabled {
                let t_apply = std::time::Instant::now();
                router = router.layer(CompressionLayer::new().gzip(true));
                layer_timings.push(("compression", t_apply.elapsed().as_millis()));
            }

            if cfg.middleware.rate_limit.enabled {
                let rl_layer = mw::RateLimitLayer {
                    requests_per_window: cfg.middleware.rate_limit.requests_per_window,
                    window_secs: cfg.middleware.rate_limit.window_secs,
                    store: self.rate_limit_store.clone(),
                };
                let t_apply = std::time::Instant::now();
                router = router.layer(rl_layer);
                layer_timings.push(("rate_limit", t_apply.elapsed().as_millis()));
            }

            // 权限校验中间件：合并配置文件规则 + 宏注解规则
            // 先添加 perm_layer，再添加 auth_layer，确保 auth 在 perm 外侧包裹，
            // 这样请求先从 auth 层进入，AuthClaims 已注入 extensions，然后 perm 层才能读取
            //
            // 装配策略（D007）：
            // - 存在 `#[permission]` 宏注解规则 → **始终**挂载，不受 `enabled` 影响。
            //   注解与处理器写在同一个文件里，是编译期契约；不能让"配置里忘了打开开关"
            //   导致受保护端点裸奔（安全默认）。`enabled = false` 时额外告警提示。
            // - 仅配置文件 `rules` → 仍由 `[middleware.permission] enabled` 控制，
            //   保持既有部署与文档（`enabled = false` 关闭校验）的行为不变。
            // - 无任何规则 → 完全不挂载，与修复前一致（零开销、响应不变）。
            // 宏注解规则的路径与路由宏同源（均为「不含前缀」的声明），
            // 因此这里统一拼接全局前缀后再参与匹配，保证与嵌套后的实际请求路径一致。
            let perm_asm_started = std::time::Instant::now();
            let mut perm_layer =
                mw::PermissionCheckLayer::from_config(&cfg.middleware.permission.rules);
            // 两条来源合并：链接期切片（`#[permission]`）+ 显式注册（`#[alun::routes]`）。
            // 后者不依赖链接器段收集，因此在切片失效的工具链上仍然生效。
            for def in macro_rules.iter().chain(self.extra_permissions.iter()) {
                perm_layer.push_rule(mw::PermissionRule {
                    path: with_prefix(&self.prefix, def.path),
                    methods: vec![def.method.to_string()],
                    permission: def.permission.to_string(),
                });
            }
            layer_timings.push(("perm_assembly", perm_asm_started.elapsed().as_millis()));
            let macro_rule_count = macro_rules.len() + self.extra_permissions.len();
            let config_rule_count = cfg.middleware.permission.rules.len();
            let enabled = cfg.middleware.permission.enabled;

            if macro_rule_count > 0 && !enabled {
                tracing::warn!(
                    "检测到 {} 条 #[permission] 规则，但 [middleware.permission] enabled = false；\
                     已按安全默认挂载权限校验层（如需豁免请移除对应的宏注解）",
                    macro_rule_count
                );
            }

            if perm_layer.has_rules() && (macro_rule_count > 0 || enabled) {
                info!(
                    "检测到 {} 条 #[permission] 规则 + {} 条配置规则 → 已启用权限校验层",
                    macro_rule_count, config_rule_count
                );
                let t_apply = std::time::Instant::now();
                router = router.layer(perm_layer);
                layer_timings.push(("permission", t_apply.elapsed().as_millis()));
            }

            // 多租户上下文中间件：由 `[tenant].enabled` 声明式接入。
            //
            // 必须先于 auth_layer 添加（成为认证层的**内层**）：请求先经认证层校验签名
            // 并注入 `AuthClaims`，租户层才据此信任载荷中的租户 claim；顺序颠倒会让
            // 未验证的载荷被采信，导致租户越权。
            #[cfg(feature = "db")]
            if cfg.tenant.enabled {
                let tenant_ignore: Vec<String> = cfg
                    .tenant
                    .ignore_paths
                    .iter()
                    .map(|p| with_prefix(&self.prefix, p))
                    .collect();
                let t_apply = std::time::Instant::now();
                router = router.layer(mw::TenantLayer {
                    claim: cfg.tenant.claim.clone(),
                    ignore_paths: tenant_ignore,
                });
                layer_timings.push(("tenant", t_apply.elapsed().as_millis()));
                info!(
                    "多租户中间件已启用 claim={} column={} write_enabled={}",
                    cfg.tenant.claim, cfg.tenant.column, cfg.tenant.write_enabled
                );
            }

            if cfg.middleware.auth.enabled && !cfg.middleware.auth.jwt_secret.is_empty() {
                let auth_asm_started = std::time::Instant::now();
                let mut ignore: Vec<String> = cfg
                    .middleware
                    .auth
                    .ignore_paths
                    .iter()
                    .map(|p| with_prefix(&self.prefix, p))
                    .collect();

                // 合并无需认证路径，三条来源：
                //   ① `#[alun::no_auth]` 编译期注册  ② 显式注册  ③ 插件贡献（F5）
                let discovered_no_auth = crate::discovered_no_auth();
                let plugin_no_auth: Vec<crate::NoAuthDef> = self
                    .web_plugins
                    .iter()
                    .flat_map(|p| p.no_auth_paths())
                    .collect();
                for def in discovered_no_auth
                    .iter()
                    .chain(self.extra_no_auth.iter())
                    .chain(plugin_no_auth.iter())
                {
                    let path_with_prefix = with_prefix(&self.prefix, def.path);
                    if !ignore.contains(&path_with_prefix) {
                        ignore.push(path_with_prefix);
                    }
                }

                #[cfg(feature = "cache")]
                let cache = try_cache().cloned();
                let auth_layer = mw::AuthLayer {
                    jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                    ignore_paths: ignore,
                    #[cfg(feature = "cache")]
                    cache,
                };
                layer_timings.push(("auth_assembly", auth_asm_started.elapsed().as_millis()));
                let t_apply = std::time::Instant::now();
                router = router.layer(auth_layer);
                layer_timings.push(("auth", t_apply.elapsed().as_millis()));
            }

            // ── 外层（后添加）：请求追踪 + 跨域 + 安全头 ──
            // 必须在 Auth 之后添加（即 Auth 的外层），
            // 确保认证 401 响应也携带 CORS 头和安全头。

            if cfg.middleware.request_id {
                let t_apply = std::time::Instant::now();
                router = router.layer(mw::RequestIdLayer);
                layer_timings.push(("request_id", t_apply.elapsed().as_millis()));
            }

            if cfg.middleware.request_log {
                let log_cfg = &cfg.middleware.request_log_config;
                let prefix_excluded: Vec<String> = log_cfg
                    .exclude_paths
                    .iter()
                    .map(|p| with_prefix(&self.prefix, p))
                    .collect();
                let log_layer = mw::RequestLogLayer {
                    exclude_paths: prefix_excluded,
                    log_duration: log_cfg.log_duration,
                };
                let t_apply = std::time::Instant::now();
                router = router.layer(log_layer);
                layer_timings.push(("request_log", t_apply.elapsed().as_millis()));
            }

            if cfg.middleware.cors.enabled {
                let mut cors = CorsLayer::new();
                if !cfg.middleware.cors.allow_origins.is_empty() {
                    let origins: Vec<HeaderValue> = cfg
                        .middleware
                        .cors
                        .allow_origins
                        .iter()
                        .filter_map(|o| o.parse().ok())
                        .collect();
                    cors = cors.allow_origin(AllowOrigin::list(origins));
                } else {
                    cors = cors.allow_origin(AllowOrigin::any());
                }
                if !cfg.middleware.cors.allow_methods.is_empty() {
                    let methods: Vec<Method> = cfg
                        .middleware
                        .cors
                        .allow_methods
                        .iter()
                        .filter_map(|m| m.parse().ok())
                        .collect();
                    cors = cors.allow_methods(methods);
                }
                if !cfg.middleware.cors.allow_headers.is_empty() {
                    let headers: Vec<axum::http::HeaderName> = cfg
                        .middleware
                        .cors
                        .allow_headers
                        .iter()
                        .filter_map(|h| h.parse().ok())
                        .collect();
                    cors = cors.allow_headers(headers);
                } else {
                    cors = cors.allow_headers(tower_http::cors::AllowHeaders::any());
                }
                if cfg.middleware.cors.allow_credentials {
                    cors = cors.allow_credentials(true);
                }
                cors = cors.max_age(std::time::Duration::from_secs(
                    cfg.middleware.cors.max_age_secs,
                ));
                let t_apply = std::time::Instant::now();
                router = router.layer(cors);
                layer_timings.push(("cors", t_apply.elapsed().as_millis()));
            }

            // F10：HTTP 指标埋点。
            //
            // 位置：安全头之内、CORS / 请求日志之外——这样被限流（429）与
            // 鉴权拒绝（401）的请求同样进入 SLO 统计；如果把埋点放在链的最内层，
            // 这些"被框架拦下"的请求会从指标里消失，故障时会得出
            // "成功率 100%"的错觉。
            //
            // 排除 `/metrics` 自身：否则 Prometheus 的定期抓取会被算作业务请求，
            // 抓取频率与真实流量混在一条序列里。
            #[cfg(feature = "metrics")]
            if cfg.metrics.http {
                let mut excluded = vec![with_prefix(&self.prefix, "/health")];
                excluded.push(cfg.metrics.path.clone());
                let t_apply = std::time::Instant::now();
                router = router.layer(crate::metrics::MetricsLayer {
                    exclude_paths: excluded,
                });
                layer_timings.push(("metrics_http", t_apply.elapsed().as_millis()));
            }

            // 安全头：最后添加（最外层），确保所有响应（含 401/403/500）都携带安全头
            if cfg.middleware.security_headers.enabled {
                let t_apply = std::time::Instant::now();
                router = router.layer(mw::SecurityHeadersLayer::new(
                    cfg.middleware.security_headers.clone(),
                ));
                layer_timings.push(("security_headers", t_apply.elapsed().as_millis()));
            }
            tracing::debug!("middleware_chain 逐层耗时（毫秒）: {:?}", layer_timings);
        }
        router
    }
}

impl Default for App {
    fn default() -> Self {
        Self {
            router: Some(AlunRouter::new()),
            plugins: PluginManager::new(),
            web_plugins: Vec::new(),
            settings: AppSettings {
                config_path: Some("config".into()),
                gen_config_only: false,
                print_config: false,
            },
            config_mgr: None,
            prefix: String::new(),
            rate_limit_store: Arc::new(RwLock::new(HashMap::new())),
            router_extensions: Vec::new(),
            route_layers: Vec::new(),
            startup_hooks: Vec::new(),
            extra_permissions: Vec::new(),
            extra_no_auth: Vec::new(),
            #[cfg(feature = "db")]
            embedded_migrator: None,
        }
    }
}

// ── 辅助函数 ───────────────────────────────────────

fn parse_addr(addr: &str) -> alun_core::Result<SocketAddr> {
    let addr = match addr {
        a if a.starts_with(':') => format!("0.0.0.0{}", a),
        a if !a.contains(':') => format!("0.0.0.0:{}", a),
        a => a.to_string(),
    };
    addr.parse()
        .map_err(|e| alun_core::Error::Config(format!("无效地址 '{}': {}", addr, e)))
}

fn resolve_display_addr(addr: SocketAddr) -> String {
    if addr.ip().is_unspecified() {
        format!("127.0.0.1:{}", addr.port())
    } else {
        addr.to_string()
    }
}

/// 把中间件路径条目拼接全局前缀。
///
/// 约定（与 `[middleware.request_log_config]` / `[middleware.auth]` 的配置说明一致）：
/// 条目默认是「不含前缀」的声明，会被拼上 `[router] prefix`；以 `!` 开头表示
/// **绝对路径**（不参与前缀拼接），用于豁免前缀的路径（如根路径 `/health`）。
///
/// # 参数
///
/// - `prefix`：全局路由前缀（`[router] prefix`）。
/// - `path`：配置中的路径条目。
///
/// # 返回值
///
/// 拼接前缀后的完整匹配路径；`path` 以 `!` 开头时返回去掉 `!` 的原值。
fn with_prefix(prefix: &str, path: &str) -> String {
    match path.strip_prefix('!') {
        Some(absolute) => absolute.to_string(),
        None => format!("{}{}", prefix, path),
    }
}

/// 等待关闭信号，两种信号走**同一**优雅关闭流程
///
/// - Unix：`SIGINT`（Ctrl-C）与 `SIGTERM`（`kill`/容器/进程管理器发送）
/// - Windows：`Ctrl-C` 与控制台关闭/注销事件（`CTRL_CLOSE`/`CTRL_SHUTDOWN`）
///
/// `axum::serve(...).with_graceful_shutdown(shutdown_signal())` 依赖本函数返回，
/// 因此任一信号触发后都会进入同一关闭路径（停止接收新连接 → 等待在途请求 → `stop_all`）。
async fn shutdown_signal() {
    let signal = wait_for_shutdown(sigint_signal(), terminate_signal()).await;
    info!("收到 {} 信号，优雅退出中...", signal);
}

/// 任一关闭信号触发即返回，值为触发的信号名
///
/// 抽出信号等待逻辑便于测试注入不同的信号源。
async fn wait_for_shutdown<A, B>(sigint: A, terminate: B) -> &'static str
where
    A: Future<Output = &'static str>,
    B: Future<Output = &'static str>,
{
    tokio::select! {
        s = sigint => s,
        s = terminate => s,
    }
}

/// `SIGINT` / `Ctrl-C`
async fn sigint_signal() -> &'static str {
    tokio::signal::ctrl_c().await.expect("Ctrl-C 注册失败");
    "SIGINT"
}

/// `SIGTERM`（Unix）/ 控制台关闭事件（Windows）
async fn terminate_signal() -> &'static str {
    #[cfg(unix)]
    {
        let mut sig = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM 注册失败");
        sig.recv().await;
        "SIGTERM"
    }

    #[cfg(windows)]
    {
        let mut close = tokio::signal::windows::ctrl_close().expect("CTRL_CLOSE 注册失败");
        let mut shutdown = tokio::signal::windows::ctrl_shutdown().expect("CTRL_SHUTDOWN 注册失败");
        tokio::select! {
            _ = close.recv() => "CTRL_CLOSE",
            _ = shutdown.recv() => "CTRL_SHUTDOWN",
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        // 其他平台无等价信号：永不触发，仅保留 SIGINT 关闭路径
        std::future::pending::<&'static str>().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ── D015：SIGINT 与 SIGTERM 走同一条优雅关闭路径 ──

    /// 向自身进程发送信号（Unix）
    #[cfg(unix)]
    fn send_signal(sig: &str) {
        let pid = std::process::id().to_string();
        let status = std::process::Command::new("kill")
            .args([sig, &pid])
            .status()
            .expect("执行 kill 失败");
        assert!(status.success(), "kill {} {} 失败", sig, pid);
    }

    /// SIGTERM 必须触发关闭路径；SIGINT（既有 ctrl_c 行为）同样必须触发
    ///
    /// 两个阶段串行执行：任一阶段都只发送一种信号，
    /// 避免并发测试互相唤醒造成假通过（修复前缺少 SIGTERM 监听 → 阶段 1 超时失败）。
    #[cfg(unix)]
    #[tokio::test]
    async fn sigterm_and_sigint_both_complete_shutdown_path() {
        // 阶段 1：SIGTERM → 生产入口 shutdown_signal() 与 terminate_signal() 都必须返回
        let production = tokio::spawn(shutdown_signal());
        let terminate = tokio::spawn(terminate_signal());
        // 等 tokio 完成信号处理器注册（signal() 在首次 poll 时注册）
        tokio::time::sleep(Duration::from_millis(300)).await;
        send_signal("-TERM");

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), terminate)
                .await
                .expect("SIGTERM 未唤醒 terminate_signal")
                .expect("terminate_signal 任务 panic"),
            "SIGTERM"
        );
        tokio::time::timeout(Duration::from_secs(10), production)
            .await
            .expect("SIGTERM 未触发 shutdown_signal 返回（优雅关闭路径未走通）")
            .expect("shutdown_signal 任务 panic");

        // 阶段 2：SIGINT（Ctrl-C）行为保持
        let interrupt = tokio::spawn(sigint_signal());
        tokio::time::sleep(Duration::from_millis(300)).await;
        send_signal("-INT");

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(10), interrupt)
                .await
                .expect("SIGINT 未唤醒 ctrl_c（既有 Ctrl-C 行为被破坏）")
                .expect("sigint_signal 任务 panic"),
            "SIGINT"
        );
    }

    /// 两条信号分支都汇聚到同一个 `wait_for_shutdown` 返回值 → 同一关闭流程
    #[tokio::test]
    async fn either_signal_completes_same_shutdown_flow() {
        let terminated = wait_for_shutdown(std::future::pending(), async { "SIGTERM" }).await;
        assert_eq!(terminated, "SIGTERM");

        let interrupted = wait_for_shutdown(async { "SIGINT" }, std::future::pending()).await;
        assert_eq!(interrupted, "SIGINT");
    }

    // ── D017：幂等 / Nonce 层由配置声明式接入默认链 ──

    #[cfg(feature = "cache")]
    fn init_cache() {
        // 全局缓存单例：重复初始化返回 Err 可忽略（其他测试可能已设置）
        let _ = crate::resources::set_cache(alun_cache::SharedCache::Local(
            alun_cache::LocalCache::new("alun-web-test", 1024, 60),
        ));
    }

    #[cfg(feature = "cache")]
    fn counting_router(hits: Arc<std::sync::atomic::AtomicUsize>) -> Router {
        use std::sync::atomic::Ordering;
        let handler = move || {
            let hits = hits.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                Json(serde_json::json!({ "created": true }))
            }
        };
        Router::new().route("/api/order/create", axum::routing::post(handler))
    }

    #[cfg(feature = "cache")]
    async fn post(
        router: &Router,
        idem_key: Option<&str>,
        nonce: Option<&str>,
    ) -> (StatusCode, String) {
        use axum::body::Body;
        use tower::ServiceExt;

        let mut builder = axum::http::Request::builder()
            .method("POST")
            .uri("/api/order/create");
        if let Some(k) = idem_key {
            builder = builder.header("x-idempotency-key", k);
        }
        if let Some(n) = nonce {
            builder = builder.header("x-nonce", n);
        }

        let resp = router
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .expect("oneshot");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .expect("读取响应体");
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    /// `[middleware.idempotency] enabled = true` → 默认链内已挂载幂等层
    #[cfg(feature = "cache")]
    #[tokio::test]
    async fn idempotency_layer_wired_by_config() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        init_cache();

        let mut cfg = AppConfig::default();
        cfg.middleware.idempotency.enabled = true;
        let app = App::with_config(cfg).expect("App::with_config");

        let hits = Arc::new(AtomicUsize::new(0));
        let router = app.build_middleware_chain(counting_router(hits.clone()));

        let (s1, b1) = post(&router, Some("cfg-on"), None).await;
        let (s2, b2) = post(&router, Some("cfg-on"), None).await;

        assert_eq!(s1, StatusCode::OK);
        assert_eq!(s2, StatusCode::OK);
        assert_eq!(b1, b2, "重放响应体必须与首次一致（命中缓存）");
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "配置启用幂等层后，同一键的第二次请求不得再执行 handler"
        );
    }

    /// 未配置（默认 enabled=false）时链不变：同键请求照旧各执行一次
    #[cfg(feature = "cache")]
    #[tokio::test]
    async fn idempotency_absent_by_default() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        init_cache();

        let app = App::with_config(AppConfig::default()).expect("App::with_config");
        let hits = Arc::new(AtomicUsize::new(0));
        let router = app.build_middleware_chain(counting_router(hits.clone()));

        let (s1, _) = post(&router, Some("cfg-off"), None).await;
        let (s2, _) = post(&router, Some("cfg-off"), None).await;

        assert_eq!(s1, StatusCode::OK);
        assert_eq!(s2, StatusCode::OK);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            2,
            "未配置时必须保持原行为：无幂等层，两次请求都执行 handler"
        );
    }

    /// `[middleware.nonce] enabled = true` → 默认链内已挂载 Nonce 防重放层
    #[cfg(feature = "cache")]
    #[tokio::test]
    async fn nonce_layer_wired_by_config() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        init_cache();

        let mut cfg = AppConfig::default();
        cfg.middleware.nonce.enabled = true;
        let app = App::with_config(cfg).expect("App::with_config");

        let hits = Arc::new(AtomicUsize::new(0));
        let router = app.build_middleware_chain(counting_router(hits.clone()));

        let (s1, _) = post(&router, None, Some("nonce-cfg")).await;
        let (s2, _) = post(&router, None, Some("nonce-cfg")).await;

        assert_eq!(s1, StatusCode::OK, "首次 nonce 请求应放行");
        assert_eq!(
            s2,
            StatusCode::CONFLICT,
            "同一 nonce 重复使用必须被拒（409）"
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "重复 nonce 不得进入 handler"
        );
    }

    // ── 请求体上限：由 `[upload].max_size_mb` 推导并自动装配 `DefaultBodyLimit` ──

    /// 上限由 `[upload].max_size_mb` 推导，且**确实替代**了 axum 内建的 2 MiB。
    ///
    /// 两个断言缺一不可：
    /// - 超过推导上限 → 413（证明层已装配）；
    /// - 3 MiB（超过 axum 内建 2 MiB、远小于推导上限）→ 200（证明生效的是推导值）。
    ///   只断言前者时，未装配的产物靠 axum 默认限制同样会返回 413，测试无法区分。
    #[tokio::test]
    async fn body_limit_is_derived_from_upload_config() {
        use axum::body::{Body, Bytes};
        use tower::ServiceExt;

        // 回显请求体长度的处理器：`Bytes` 是受 `DefaultBodyLimit` 约束的提取器
        async fn echo_len(body: Bytes) -> String {
            body.len().to_string()
        }

        // 发送指定字节数的裸请求体，返回响应状态
        async fn send(router: &Router, bytes: usize) -> StatusCode {
            let request = axum::http::Request::builder()
                .method("POST")
                .uri("/api/alun-body-limit-echo")
                .body(Body::from(vec![b'x'; bytes]))
                .expect("构造请求");
            router
                .clone()
                .oneshot(request)
                .await
                .expect("oneshot")
                .status()
        }

        let cfg = AppConfig::default();
        let limit = cfg.upload.max_request_body_bytes();
        let app = App::with_config(cfg).expect("App::with_config");
        let router = app.build_middleware_chain(
            Router::new().route("/api/alun-body-limit-echo", axum::routing::post(echo_len)),
        );

        assert_eq!(
            send(&router, 3 * 1024 * 1024).await,
            StatusCode::OK,
            "3 MiB 在推导上限（{limit} 字节）之内必须放行；返回 413 说明仍在用 axum 内建的 2 MiB"
        );
        assert_eq!(
            send(&router, limit + 1).await,
            StatusCode::PAYLOAD_TOO_LARGE,
            "超过推导上限（{limit} 字节）必须 413"
        );
    }

    // ── D007：`#[permission]` 宏注解规则默认生效（安全默认） ──

    /// 测试用 JWT 密钥
    const PERM_TEST_SECRET: &str = "alun-web-permission-test-secret";

    /// 与 `#[alun::permission(...)]` 宏展开产物等价：以 `inventory` 提交一条全局规则。
    ///
    /// 路径刻意与其它用例隔离，避免影响同一测试二进制内其他装配断言。
    inventory::submit! {
        crate::PermissionDef {
            path: "/api/perm-rule-demo/orders",
            method: "GET",
            permission: "order:read",
        }
    }

    /// 单个带权限规则路径的 handler 路由
    fn demo_router(path: &'static str) -> Router {
        Router::new().route(
            path,
            axum::routing::get(|| async { Json(serde_json::json!({ "ok": true })) }),
        )
    }

    /// `GET path`，可选携带 Bearer Token
    async fn get_with_bearer(
        router: &Router,
        path: &str,
        token: Option<&str>,
    ) -> (StatusCode, String) {
        use axum::body::Body;
        use tower::ServiceExt;

        let mut builder = axum::http::Request::builder().method("GET").uri(path);
        if let Some(t) = token {
            builder = builder.header("authorization", format!("Bearer {t}"));
        }

        let resp = router
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .expect("oneshot");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .expect("读取响应体");
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    /// 不依赖全局资源单例签发 access token
    fn access_token(cfg: &AppConfig, permissions: &[&str]) -> String {
        let cm = Arc::new(ConfigManager {
            static_config: cfg.clone(),
            dynamic: parking_lot::RwLock::new(HashMap::new()),
        });
        let perms: Vec<String> = permissions.iter().map(|p| p.to_string()).collect();
        crate::jwt::JWT::with_config(cm)
            .create_access_token("u-1", Some("alice"), &["user".to_string()], &perms)
            .expect("签发测试 access token")
    }

    /// 开启认证层（与 permission 开关无关），用于区分 401/403 并验证 AuthClaims 已注入
    fn cfg_with_auth() -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.middleware.auth.enabled = true;
        cfg.middleware.auth.jwt_secret = PERM_TEST_SECRET.to_string();
        cfg
    }

    /// ① 默认配置（`permission.enabled` 缺省 false）+ 存在 `#[permission]` 规则
    /// → 校验层必须挂载：已认证但缺少权限的请求被拒（修复前裸奔返回 200）
    #[tokio::test]
    async fn permission_rule_enforced_under_default_config() {
        let cfg = cfg_with_auth();
        assert!(
            !cfg.middleware.permission.enabled,
            "本用例前提：permission.enabled 保持默认 false"
        );

        let app = App::with_config(cfg.clone()).expect("App::with_config");
        // 生产入口 `build_middleware_chain` 读取编译期注册的权限规则（含上面提交的规则）
        let router = app.build_middleware_chain(demo_router("/api/perm-rule-demo/orders"));

        let (unauthenticated, body) =
            get_with_bearer(&router, "/api/perm-rule-demo/orders", None).await;
        assert_eq!(
            unauthenticated,
            StatusCode::UNAUTHORIZED,
            "缺 Token 时应由认证层拒绝: {body}"
        );

        let weak = access_token(&cfg, &[]);
        let (denied, body) =
            get_with_bearer(&router, "/api/perm-rule-demo/orders", Some(&weak)).await;
        assert_eq!(
            denied,
            StatusCode::FORBIDDEN,
            "默认配置下带 #[permission] 的路由必须校验权限（修复前为 200）: {body}"
        );

        let allowed = access_token(&cfg, &["order:read"]);
        let (ok, body) =
            get_with_bearer(&router, "/api/perm-rule-demo/orders", Some(&allowed)).await;
        assert_eq!(ok, StatusCode::OK, "持有权限的请求不得被误伤: {body}");
    }

    /// ①b 默认配置（认证层也未开启）→ 无 AuthClaims，校验层按既有语义返回 403
    #[tokio::test]
    async fn permission_rule_denied_without_auth_layer_under_default_config() {
        let app = App::with_config(AppConfig::default()).expect("App::with_config");
        let router = app.build_middleware_chain(demo_router("/api/perm-rule-demo/orders"));

        let (status, body) = get_with_bearer(&router, "/api/perm-rule-demo/orders", None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "未注入 AuthClaims 时校验层应拒绝（与既有校验层行为一致）: {body}"
        );
    }

    /// ② 默认配置 + **无** 任何规则 → 不挂载校验层，链路与响应完全不变
    #[tokio::test]
    async fn no_permission_rules_leaves_chain_untouched() {
        let app = App::with_config(AppConfig::default()).expect("App::with_config");
        let router = app.build_middleware_chain_with(demo_router("/api/plain-demo/orders"), &[]);

        let (status, body) = get_with_bearer(&router, "/api/plain-demo/orders", None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "无任何权限规则时不得挂载校验层，请求应直达 handler: {body}"
        );
    }

    /// ③ 显式 `enabled = true` 时既有行为保持：持有权限放行、缺少权限 403
    #[tokio::test]
    async fn explicit_enabled_true_still_enforces_macro_rules() {
        let mut cfg = cfg_with_auth();
        cfg.middleware.permission.enabled = true;
        let app = App::with_config(cfg.clone()).expect("App::with_config");

        let rule = crate::PermissionDef {
            path: "/api/perm-on/orders",
            method: "GET",
            permission: "order:read",
        };
        let router = app.build_middleware_chain_with(demo_router("/api/perm-on/orders"), &[rule]);

        let allowed = access_token(&cfg, &["order:read"]);
        let (ok, body) = get_with_bearer(&router, "/api/perm-on/orders", Some(&allowed)).await;
        assert_eq!(ok, StatusCode::OK, "enabled=true 时持有权限应放行: {body}");

        let weak = access_token(&cfg, &[]);
        let (denied, body) = get_with_bearer(&router, "/api/perm-on/orders", Some(&weak)).await;
        assert_eq!(
            denied,
            StatusCode::FORBIDDEN,
            "enabled=true 时缺少权限必须拒绝: {body}"
        );
    }

    /// 仅配置文件规则时仍由 `enabled` 控制 —— 既有部署行为不变（默认关闭即不挂载）
    #[tokio::test]
    async fn config_only_rules_remain_gated_by_enabled_flag() {
        let rule = alun_config::PermissionRule {
            path: "/api/cfg-rule/orders".into(),
            methods: vec!["GET".into()],
            permission: "order:read".into(),
        };

        let mut off = AppConfig::default();
        off.middleware.permission.rules = vec![rule.clone()];
        let app_off = App::with_config(off).expect("App::with_config");
        let router_off =
            app_off.build_middleware_chain_with(demo_router("/api/cfg-rule/orders"), &[]);
        let (status, body) = get_with_bearer(&router_off, "/api/cfg-rule/orders", None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "enabled=false 时仅配置规则不得挂载（保持既有行为）: {body}"
        );

        let mut on = AppConfig::default();
        on.middleware.permission.rules = vec![rule];
        on.middleware.permission.enabled = true;
        let app_on = App::with_config(on).expect("App::with_config");
        let router_on =
            app_on.build_middleware_chain_with(demo_router("/api/cfg-rule/orders"), &[]);
        let (status, body) = get_with_bearer(&router_on, "/api/cfg-rule/orders", None).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "enabled=true 时配置规则必须生效: {body}"
        );
    }
}

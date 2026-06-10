
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
use tower_http::cors::{CorsLayer, AllowOrigin};
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use parking_lot::RwLock;
use tracing::info;

use crate::router::AlunRouter;
use crate::middleware as mw;
use alun_core::{PluginManager, Result};
use alun_core::api::{codes, Res};
use alun_config::{AppConfig, ConfigManager};
use crate::resources::*;

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
    plugins: PluginManager,
    /// 应用设置（CLI 参数等）
    settings: AppSettings,
    /// 配置管理器（文件加载 + 运行时动态配置）
    config_mgr: Option<Arc<ConfigManager>>,
    /// 全局路由前缀（从 `router.prefix` 读取）
    prefix: String,
    /// 限流存储（跨请求共享，确保限流计数正确）
    rate_limit_store: Arc<RwLock<HashMap<String, mw::IpWindow>>>,
    /// 自定义中间件注入钩子
    custom_middleware_hook: Option<Box<dyn FnOnce(Router) -> Router + Send>>,
    /// 自定义启动钩子（在全局资源初始化之后、插件启动之前执行）
    startup_hook: Option<Box<dyn FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send>>,
}

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
        alun_log::init(&cfg.log);

        let prefix = cfg.router.prefix.clone();

        Ok(Self {
            router: Some(AlunRouter::new()),
            plugins: PluginManager::new(),
            settings: AppSettings {
                config_path: Some("config".into()),
                gen_config_only: false,
                print_config: false,
            },
            config_mgr: Some(cm),
            prefix,
            rate_limit_store: Arc::new(RwLock::new(HashMap::new())),
            custom_middleware_hook: None,
            startup_hook: None,
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
            settings: AppSettings {
                config_path: None,
                gen_config_only: false,
                print_config: false,
            },
            config_mgr: None,
            prefix: String::new(),
            rate_limit_store: Arc::new(RwLock::new(HashMap::new())),
            custom_middleware_hook: None,
            startup_hook: None,
        });
        if let (Some(ref mut r), Some(sub_r)) = (self.router.as_mut(), sub.router) {
            r.merge(prefix, sub_r);
        }
        self
    }

    /// 扫描 `#[get]`、`#[post]`、`#[controller]` 等宏注解，自动注册路由
    ///
    /// 编译期通过 linkme 分布式切片收集所有被宏标注的处理器。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// #[alun::get("/api/hello")]
    /// async fn hello() -> Res<String> { Res::ok("Hi!") }
    ///
    /// App::new()?
    ///     .scan()
    ///     .start()
    ///     .await
    /// ```
    pub fn scan(mut self) -> Self {
        for register in crate::ROUTES {
            if let Some(ref mut r) = self.router {
                register(r);
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
        mut self, method: &str, path: &str, handler: H, permission: &str,
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
    pub fn with_role<H, T>(
        mut self, method: &str, path: &str, handler: H, role: &str,
    ) -> Self
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
    pub fn plugin<P: alun_core::Plugin + 'static>(mut self, plugin: P) -> Self {
        self.plugins = self.plugins.add(plugin);
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
        self.startup_hook = Some(Box::new(|| Box::pin(hook())));
        self
    }

    /// 注册自定义中间件注入钩子
    ///
    /// 钩子在 `build_middleware_chain` 之后调用，可在此阶段通过 `axum::middleware::from_fn` 注入自定义中间件。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// App::new()?
    ///     .with_middleware_hook(|router| {
    ///         router.layer(axum::middleware::from_fn(my_middleware))
    ///     })
    ///     .scan()
    ///     .start()
    ///     .await
    /// ```
    pub fn with_middleware_hook<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(Router) -> Router + Send + 'static,
    {
        self.custom_middleware_hook = Some(Box::new(hook));
        self
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
                .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "alun=info".into()))
                .try_init();
        }

        if self.settings.print_config {
            if let Some(ref cm) = self.config_mgr {
                if let Ok(toml_str) = toml::to_string_pretty(cm.get()) {
                    println!("{}", toml_str);
                }
            }
        }

        // 1. 先初始化全局资源（数据库、缓存、配置、模板）
        if let Some(ref cm) = self.config_mgr {
            Self::init_global_resources(cm).await?;
        }

        // 1.5. 执行自定义启动钩子（可初始化自定义全局资源）
        if let Some(hook) = self.startup_hook.take() {
            (hook)().await;
        }

        // 2. 再启动插件，此时插件可以安全访问全局资源
        self.plugins.check_duplicate_names()
            .map_err(alun_core::Error::Config)?;
        self.plugins.start_all().await?;

        // 3. 构建 Router 和中间件
        let router = self.router.take().unwrap_or_default();
        let mut axum_router: Router = router.into_axum();
        axum_router = self.build_middleware_chain(axum_router);

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

        if let Some(hook) = self.custom_middleware_hook.take() {
            axum_router = hook(axum_router);
        }

        let bind_addr = self.config_mgr
            .as_ref()
            .map(|cm| cm.get().server.listen.clone())
            .unwrap_or_else(|| "0.0.0.0:0".to_string());

        let socket_addr = parse_addr(&bind_addr)?;
        let display_addr = resolve_display_addr(socket_addr);
        let app_name = self.config_mgr.as_ref().map(|cm| cm.get().app_name.as_str()).unwrap_or("Alun");
        info!("{} 启动 -> http://{}", app_name, display_addr);
        if let Some(cm) = &self.config_mgr {
            info!(
                "  profile={}, request_id={} log={} cors={} compression={} rate_limit={} jwt_auth={} static_files={} not_found={}",
                cm.get().profile,
                cm.get().middleware.request_id,
                cm.get().middleware.request_log,
                cm.get().middleware.cors.enabled,
                cm.get().middleware.compression.enabled,
                cm.get().middleware.rate_limit.enabled,
                cm.get().middleware.auth.enabled,
                cm.get().static_files.enabled,
                cm.get().router.not_found.enabled,
            );
        }

        let startup_ms = startup_start.elapsed().as_millis();
        info!("{} 启动完成, 耗时 {}ms", app_name, startup_ms);

        let listener = tokio::net::TcpListener::bind(socket_addr).await?;
        axum::serve(listener, axum_router.into_make_service_with_connect_info::<SocketAddr>())
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
                    ..Default::default()
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
    async fn init_global_resources(cm: &Arc<ConfigManager>) -> Result<()> {
        set_config(cm.clone()).map_err(|e| alun_core::Error::Config(e.to_string()))?;
        let cfg = cfg();

        #[cfg(feature = "db")]
        if cfg.database.enabled {
            match alun_db::factory::create_db(&cfg.database).await {
                Ok(db) => {
                    info!("数据库连接成功");

                    if cfg.database.migration.enabled && cfg.database.migration.auto_migrate {
                        let migrator = alun_db::migrate::Migrator::new(db.clone(), cfg.database.migration.clone());
                        match migrator.run().await {
                            Ok(records) => info!("数据库迁移完成: {:?}", records.iter().map(|r| &r.version).collect::<Vec<_>>()),
                            Err(e) => {
                                tracing::error!("数据库迁移失败: {}", e);
                                return Err(alun_core::Error::Config(format!("数据库迁移失败: {}", e)));
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
            info!("上传目录就绪 path={} max_size_mb={}", upload_path, cfg.upload.max_size_mb);
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
    async fn handle_not_found() -> impl IntoResponse {
        let msg = cfg().router.not_found.message.clone();
        (StatusCode::NOT_FOUND, Json(Res::<()>::fail(codes::NOT_FOUND, msg)))
    }

    /// 构建中间件链
    fn build_middleware_chain(
        &self,
        mut router: Router,
    ) -> Router {
        if let Some(ref cm) = self.config_mgr {
            let cfg = cm.get();

            // 安全头：最先注入，确保所有响应都携带
            if cfg.middleware.security_headers.enabled {
                router = router.layer(mw::SecurityHeadersLayer::new(
                    cfg.middleware.security_headers.clone(),
                ));
            }

            if cfg.middleware.request_log {
                let log_cfg = &cfg.middleware.request_log_config;
                let prefix_excluded: Vec<String> = log_cfg.exclude_paths
                    .iter().map(|p| format!("{}{}", self.prefix, p)).collect();
                let log_layer = mw::RequestLogLayer {
                    exclude_paths: prefix_excluded,
                    log_duration: log_cfg.log_duration,
                };
                router = router.layer(log_layer);
            }

            if cfg.middleware.request_id {
                router = router.layer(mw::RequestIdLayer);
            }

            if cfg.middleware.cors.enabled {
                let mut cors = CorsLayer::new();
                if !cfg.middleware.cors.allow_origins.is_empty() {
                    let origins: Vec<HeaderValue> = cfg.middleware.cors.allow_origins
                        .iter().filter_map(|o| o.parse().ok()).collect();
                    cors = cors.allow_origin(AllowOrigin::list(origins));
                } else {
                    cors = cors.allow_origin(AllowOrigin::any());
                }
                if !cfg.middleware.cors.allow_methods.is_empty() {
                    let methods: Vec<Method> = cfg.middleware.cors.allow_methods
                        .iter().filter_map(|m| m.parse().ok()).collect();
                    cors = cors.allow_methods(methods);
                }
                if !cfg.middleware.cors.allow_headers.is_empty() {
                    let headers: Vec<axum::http::HeaderName> = cfg.middleware.cors.allow_headers
                        .iter().filter_map(|h| h.parse().ok()).collect();
                    cors = cors.allow_headers(headers);
                } else {
                    cors = cors.allow_headers(tower_http::cors::AllowHeaders::any());
                }
                if cfg.middleware.cors.allow_credentials {
                    cors = cors.allow_credentials(true);
                }
                cors = cors.max_age(std::time::Duration::from_secs(cfg.middleware.cors.max_age_secs));
                router = router.layer(cors);
            }

            if cfg.middleware.compression.enabled {
                router = router.layer(CompressionLayer::new().gzip(true));
            }

            if cfg.middleware.rate_limit.enabled {
                let rl_layer = mw::RateLimitLayer {
                    requests_per_window: cfg.middleware.rate_limit.requests_per_window,
                    window_secs: cfg.middleware.rate_limit.window_secs,
                    store: self.rate_limit_store.clone(),
                };
                router = router.layer(rl_layer);
            }

            // 权限校验中间件：合并配置文件规则 + 宏注解规则
            // 先添加 perm_layer，再添加 auth_layer，确保 auth 在 perm 外侧包裹，
            // 这样请求先从 auth 层进入，AuthClaims 已注入 extensions，然后 perm 层才能读取
            let mut perm_layer = mw::PermissionCheckLayer::from_config(&cfg.middleware.permission.rules);
            perm_layer = perm_layer.with_macro_rules(&crate::PERMISSION_ROUTES);
            if cfg.middleware.permission.enabled && perm_layer.has_rules() {
                router = router.layer(perm_layer);
            }

            if cfg.middleware.auth.enabled && !cfg.middleware.auth.jwt_secret.is_empty() {
                let mut ignore: Vec<String> = cfg.middleware.auth.ignore_paths
                    .iter().map(|p| format!("{}{}", self.prefix, p)).collect();
                
                // 合并宏注解标记的无需认证路径
                for def in crate::NO_AUTH_ROUTES {
                    let path_with_prefix = format!("{}{}", self.prefix, def.path);
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
                router = router.layer(auth_layer);
            }
        }
        router
    }
}

impl Default for App {
    fn default() -> Self {
        Self {
            router: Some(AlunRouter::new()),
            plugins: PluginManager::new(),
            settings: AppSettings {
                config_path: Some("config".into()),
                gen_config_only: false,
                print_config: false,
            },
            config_mgr: None,
            prefix: String::new(),
            rate_limit_store: Arc::new(RwLock::new(HashMap::new())),
            custom_middleware_hook: None,
            startup_hook: None,
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

async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.expect("Ctrl-C 注册失败");
    info!("收到关闭信号，优雅退出中...");
}


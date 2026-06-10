//! 配置系统：TOML 加载、静态/动态配置、生成默认文件
//!
//! 设计要点：
//! - `profile` → 多环境 Profile 切换
//! - Settings/Routes/Plugins → 统一 AppConfig struct
//!
//! alun 特性：
//! - TOML 格式（结构清晰，强于 properties）
//! - 静态配置（启动加载）+ 动态配置（运行时读写）
//! - `gen-config` 命令行参数一键生成默认配置

use std::path::Path;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use parking_lot::RwLock;
use tracing::info;

pub mod env;
pub use env::{detect_profile, parse_args, merge_env_overrides};

/// 完整应用配置——Settings + Routes + Plugins 三合一
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// 应用名称
    #[serde(default = "default_app_name")]
    pub app_name: String,

    /// 当前激活的 profile（dev/prod/test）
    #[serde(default = "default_profile")]
    pub profile: String,

    /// 服务器配置
    #[serde(default)]
    pub server: ServerConfig,

    /// 日志配置
    #[serde(default)]
    pub log: LogConfig,

    /// 数据库配置
    #[serde(default)]
    pub database: DatabaseConfig,

    /// Redis 配置
    #[serde(default)]
    pub redis: RedisConfig,

    /// 缓存配置
    #[serde(default)]
    pub cache: CacheConfig,

    /// 中间件配置
    #[serde(default)]
    pub middleware: MiddlewareConfig,

    /// 路由配置
    #[serde(default)]
    pub router: RouterConfig,

    /// 插件配置
    #[serde(default)]
    pub plugins: PluginsConfig,

    /// 上传配置
    #[serde(default)]
    pub upload: UploadConfig,

    /// 下载配置
    #[serde(default)]
    pub download: DownloadConfig,

    /// 模板配置
    #[serde(default)]
    pub template: TemplateConfig,

    /// 静态文件配置
    #[serde(default)]
    pub static_files: StaticConfig,

    /// 自定义配置（供插件运行时读写）
    #[serde(default)]
    pub custom: HashMap<String, serde_json::Value>,
}

/// 服务器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 监听地址
    #[serde(default = "default_listen")]
    pub listen: String,
}

/// 日志配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    /// 日志级别：trace/debug/info/warn/error
    #[serde(default = "default_log_level")]
    pub level: String,

    /// 输出格式：text/json
    #[serde(default = "default_log_format")]
    pub format: String,

    /// 输出目录（同时输出到文件），默认不输出
    #[serde(default)]
    pub dir: Option<String>,

    /// 文件名前缀
    #[serde(default = "default_log_prefix")]
    pub file_prefix: String,
}

/// 数据库配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// 是否启用
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// 数据库类型：postgres/mysql/sqlite
    #[serde(default = "default_db_type")]
    pub r#type: String,

    /// 主机
    #[serde(default = "default_host")]
    pub host: String,

    /// 端口
    pub port: Option<u16>,

    /// 数据库名
    #[serde(default)]
    pub name: String,

    /// 用户名
    #[serde(default)]
    pub user: String,

    /// 密码（支持明文或 base64 密文，server.key 解密）
    #[serde(default)]
    pub password: String,

    /// 密码是否加密
    #[serde(default)]
    pub password_encrypted: bool,

    /// 最大连接数
    #[serde(default = "default_pool_size")]
    pub max_connections: u32,

    /// 最小空闲连接
    #[serde(default = "default_min_idle")]
    pub min_connections: u32,

    /// 连接超时（秒）
    #[serde(default = "default_timeout")]
    pub connect_timeout: u64,

    /// 启用 SQL 日志
    #[serde(default)]
    pub sql_logging: bool,

    /// 慢查询阈值（毫秒）
    #[serde(default)]
    pub slow_query_ms: u64,

    /// 迁移配置
    #[serde(default)]
    pub migration: MigrationConfig,
}

/// Redis 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedisConfig {
    /// 是否启用 Redis
    #[serde(default)]
    pub enabled: bool,

    /// Redis 连接 URL（如 `redis://127.0.0.1:6379`）
    #[serde(default = "default_redis_url")]
    pub url: String,

    /// 最大连接数
    #[serde(default = "default_pool_size")]
    pub max_connections: u32,
}

/// 缓存配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// 缓存类型：`local`（内存）或 `redis`（远端）
    #[serde(default = "default_cache_type")]
    pub r#type: String,

    /// 本地缓存最大容量（条目数）
    #[serde(default = "default_cache_capacity")]
    pub max_capacity: u64,

    /// 默认 TTL（秒），0 表示永不过期
    #[serde(default = "default_ttl")]
    pub default_ttl: u64,
}

/// 中间件配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MiddlewareConfig {
    /// 请求 ID 中间件
    #[serde(default)]
    pub request_id: bool,

    /// 日志中间件
    #[serde(default)]
    pub request_log: bool,

    /// 请求日志配置
    #[serde(default)]
    pub request_log_config: RequestLogConfig,

    /// 认证中间件
    #[serde(default)]
    pub auth: AuthMiddlewareConfig,

    /// CORS 配置
    #[serde(default)]
    pub cors: CorsConfig,

    /// 压缩配置
    #[serde(default)]
    pub compression: CompressConfig,

    /// IP 限流配置
    #[serde(default)]
    pub rate_limit: RateLimitConfig,

    /// 安全头配置
    #[serde(default)]
    pub security_headers: SecurityHeadersConfig,

    /// 权限校验配置
    #[serde(default)]
    pub permission: PermissionConfig,
}

/// 请求日志中间件配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLogConfig {
    /// 不记录日志的路径列表（如 "/api/health"） 注意不含prefix前缀
    #[serde(default)]
    pub exclude_paths: Vec<String>,

    /// 是否记录请求耗时
    #[serde(default = "default_true")]
    pub log_duration: bool,
}

/// 认证中间件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthMiddlewareConfig {
    #[serde(default)]
    pub enabled: bool,

    /// 白名单路径 注意不含prefix前缀
    #[serde(default)]
    pub ignore_paths: Vec<String>,

    /// JWT secret
    #[serde(default)]
    pub jwt_secret: String,

    /// Access Token 过期（秒）
    #[serde(default = "default_access_token_expire")]
    pub access_token_expire_secs: u64,

    /// Refresh Token 过期（秒）
    #[serde(default = "default_refresh_token_expire")]
    pub refresh_token_expire_secs: u64,
}

/// CORS 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorsConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub allow_origins: Vec<String>,

    #[serde(default)]
    pub allow_methods: Vec<String>,

    #[serde(default)]
    pub allow_headers: Vec<String>,

    #[serde(default = "default_true")]
    pub allow_credentials: bool,

    #[serde(default = "default_cors_max_age")]
    pub max_age_secs: u64,
}

/// 响应压缩配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressConfig {
    /// 是否启用 gzip 压缩
    #[serde(default)]
    pub enabled: bool,

    /// 压缩级别 0-9（0 不压缩，9 最高压缩率）
    #[serde(default = "default_compress_level")]
    pub level: u32,
}

/// IP 限流配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    #[serde(default)]
    pub enabled: bool,

    /// 每个窗口允许的请求数
    #[serde(default = "default_rate_limit_requests")]
    pub requests_per_window: u64,

    /// 窗口大小（秒）
    #[serde(default = "default_rate_limit_window")]
    pub window_secs: u64,
}

/// 安全响应头配置
///
/// 默认全部开启，通过 `enabled = false` 可关闭整个中间件，
/// 或按需关闭单个 header。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityHeadersConfig {
    /// 是否启用安全头中间件
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// X-Content-Type-Options: nosniff
    #[serde(default = "default_true")]
    pub nosniff: bool,

    /// X-Frame-Options: DENY
    #[serde(default = "default_true")]
    pub frame_options: bool,

    /// Strict-Transport-Security (HSTS)
    #[serde(default = "default_true")]
    pub hsts: bool,

    /// HSTS max-age（秒），默认 1 年
    #[serde(default = "default_hsts_max_age")]
    pub hsts_max_age_secs: u64,

    /// HSTS 是否包含子域名
    #[serde(default = "default_true")]
    pub hsts_include_subdomains: bool,

    /// Content-Security-Policy
    #[serde(default = "default_true")]
    pub csp: bool,

    /// CSP 指令值（默认 `default-src 'self'`）
    #[serde(default = "default_csp_value")]
    pub csp_value: String,

    /// Referrer-Policy
    #[serde(default = "default_true")]
    pub referrer_policy: bool,

    /// Referrer-Policy 值（默认 `strict-origin-when-cross-origin`）
    #[serde(default = "default_referrer_policy_value")]
    pub referrer_policy_value: String,

    /// Permissions-Policy（可选，默认关闭）
    #[serde(default)]
    pub permissions_policy: bool,

    /// Permissions-Policy 指令值
    #[serde(default = "default_permissions_policy_value")]
    pub permissions_policy_value: String,
}

/// 权限校验中间件配置
///
/// 支持两种方式定义权限规则：
/// 1. 配置文件 `rules`：路径模式匹配，灵活但需要重启
/// 2. 宏注解 `#[permission]`：编译期绑定，与处理器同在一个文件，直观
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionConfig {
    /// 是否启用权限全局开关（关闭后所有权限校验跳过）
    #[serde(default)]
    pub enabled: bool,

    /// 路径级权限规则
    #[serde(default)]
    pub rules: Vec<PermissionRule>,
}

/// 单条路径权限规则
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRule {
    /// 匹配路径，支持前缀匹配（如 "/api/admin" 匹配所有 /api/admin/* 请求）
    pub path: String,
    /// 限定 HTTP 方法（空表示所有方法）
    #[serde(default)]
    pub methods: Vec<String>,
    /// 所需权限标识（如 "admin:access", "user:write"）
    pub permission: String,
}

/// 路由配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    /// 全局路由前缀
    #[serde(default)]
    pub prefix: String,
    /// 404 处理配置
    #[serde(default)]
    pub not_found: NotFoundConfig,
}

/// 404 处理配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotFoundConfig {
    /// 是否启用自定义 404 响应（返回 JSON 格式的统一错误响应）
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 自定义 404 提示消息
    #[serde(default = "default_not_found_msg")]
    pub message: String,
}

/// 数据库迁移配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationConfig {
    /// 是否启用迁移
    #[serde(default)]
    pub enabled: bool,

    /// 迁移文件目录
    #[serde(default = "default_migration_path")]
    pub path: String,

    /// 启动时自动运行迁移
    #[serde(default)]
    pub auto_migrate: bool,
}

/// 上传配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadConfig {
    /// 上传文件存储目录
    #[serde(default = "default_upload_path")]
    pub path: String,

    /// 最大文件大小（MB）
    #[serde(default = "default_max_size")]
    pub max_size_mb: u64,
}

/// 下载配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadConfig {
    /// 下载文件存储目录
    #[serde(default = "default_download_path")]
    pub path: String,
}

/// 模板配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateConfig {
    /// 模板文件目录
    #[serde(default = "default_template_path")]
    pub path: String,
}

/// 静态文件配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaticConfig {
    /// 静态文件目录
    #[serde(default = "default_static_path")]
    pub path: String,

    /// 是否启用静态文件服务
    #[serde(default)]
    pub enabled: bool,
}

/// 插件配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    /// 启用的插件列表
    #[serde(default)]
    pub enabled: Vec<String>,

    /// 通知插件
    #[serde(default)]
    pub notification: NotificationConfig,

    /// 异步任务插件
    #[serde(default)]
    pub async_task: AsyncTaskConfig,

    /// 定时任务插件
    #[serde(default)]
    pub scheduler: SchedulerConfig,

    /// 单号生成器插件
    #[serde(default)]
    pub serial: SerialConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotificationConfig {
    /// 是否启用
    #[serde(default)]
    pub enabled: bool,
    /// 邮箱 SMTP 服务器
    #[serde(default)]
    pub smtp_host: String,
    /// SMTP 端口（默认 587）
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    /// SMTP 登录用户名
    #[serde(default)]
    pub smtp_user: String,
    /// SMTP 登录密码
    #[serde(default)]
    pub smtp_pass: String,
    /// 发件人邮箱地址（与 smtp_user 可能不同，如代理发信场景）
    #[serde(default)]
    pub from_email: String,
    /// 发件人显示名称
    #[serde(default)]
    pub from_name: String,
}

fn default_smtp_port() -> u16 { 587 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AsyncTaskConfig {
    /// 工作线程数（默认 4）
    #[serde(default = "default_workers")]
    pub workers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SchedulerConfig {
    /// 调度器工作线程数（默认 4）
    #[serde(default = "default_workers")]
    pub workers: usize,
}

/// 单号生成器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialConfig {
    /// 后端类型："memory"（默认）、"redis"、"postgres"
    #[serde(default = "default_serial_backend")]
    pub backend: String,
    /// 静态规则列表
    #[serde(default)]
    pub rules: Vec<SerialRuleConfig>,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self { backend: "memory".into(), rules: Vec::new() }
    }
}

fn default_serial_backend() -> String { "memory".into() }

/// 单号规则配置（TOML 可序列化）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialRuleConfig {
    /// 规则唯一标识
    pub key: String,
    /// 单号格式，如 "ORD{YYYY}{MM}{DD}{SEQ:8}"
    pub format: String,
    /// 循环周期："no_cycle"、"daily"、"monthly"、"yearly"
    #[serde(default = "default_cycle")]
    pub cycle: String,
    /// 计数器初始值
    #[serde(default = "default_initial")]
    pub initial_value: u64,
    /// 增量策略："sequential"（默认）或 "random:max"
    #[serde(default = "default_step_strategy")]
    pub step: String,
}

fn default_cycle() -> String { "no_cycle".into() }
fn default_initial() -> u64 { 1 }
fn default_step_strategy() -> String { "sequential".into() }

// ──── 默认值函数 ────────────────────────────────────

fn default_app_name() -> String { "Alun".into() }
fn default_profile() -> String { "dev".into() }
fn default_listen() -> String { "8023".into() }
fn default_log_level() -> String { "info".into() }
fn default_log_format() -> String { "text".into() }
fn default_log_prefix() -> String { "alun".into() }
fn default_db_type() -> String { "postgres".into() }
fn default_host() -> String { "localhost".into() }
fn default_true() -> bool { true }
fn default_pool_size() -> u32 { 10 }
fn default_min_idle() -> u32 { 2 }
fn default_timeout() -> u64 { 10 }
fn default_workers() -> usize { 4 }
fn default_redis_url() -> String { "redis://127.0.0.1:6379".into() }
fn default_cache_type() -> String { "local".into() }
fn default_cache_capacity() -> u64 { 10000 }
fn default_ttl() -> u64 { 3600 }
fn default_access_token_expire() -> u64 { 7200 }
fn default_refresh_token_expire() -> u64 { 604800 }
fn default_cors_max_age() -> u64 { 86400 }
fn default_compress_level() -> u32 { 6 }
fn default_rate_limit_requests() -> u64 { 100 }
fn default_rate_limit_window() -> u64 { 60 }
fn default_hsts_max_age() -> u64 { 31536000 }
fn default_csp_value() -> String { "default-src 'self'".into() }
fn default_referrer_policy_value() -> String { "strict-origin-when-cross-origin".into() }
fn default_permissions_policy_value() -> String {
    "camera=(), microphone=(), geolocation=()".into()
}
fn default_migration_path() -> String { "migrations".into() }
fn default_upload_path() -> String { "uploads".into() }
fn default_download_path() -> String { "downloads".into() }
fn default_template_path() -> String { "templates".into() }
fn default_static_path() -> String { "static".into() }
fn default_not_found_msg() -> String { "请求的资源不存在".into() }
fn default_max_size() -> u64 { 10 }

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            app_name: default_app_name(),
            profile: default_profile(),
            server: ServerConfig::default(),
            log: LogConfig::default(),
            database: DatabaseConfig::default(),
            redis: RedisConfig::default(),
            cache: CacheConfig::default(),
            middleware: MiddlewareConfig::default(),
            router: RouterConfig::default(),
            plugins: PluginsConfig::default(),
            upload: UploadConfig::default(),
            download: DownloadConfig::default(),
            template: TemplateConfig::default(),
            static_files: StaticConfig::default(),
            custom: HashMap::new(),
        }
    }
}

impl Default for ServerConfig { fn default() -> Self { Self { listen: default_listen() } } }
impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: default_log_format(),
            dir: None,
            file_prefix: default_log_prefix(),
        }
    }
}
impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            enabled: false, r#type: default_db_type(), host: default_host(),
            port: None, name: String::new(), user: String::new(), password: String::new(),
            password_encrypted: false,
            max_connections: default_pool_size(), min_connections: default_min_idle(),
            connect_timeout: default_timeout(), sql_logging: false, slow_query_ms: 0,
            migration: MigrationConfig::default(),
        }
    }
}
impl Default for RedisConfig { fn default() -> Self { Self { enabled: false, url: default_redis_url(), max_connections: default_pool_size() } } }
impl Default for CacheConfig { fn default() -> Self { Self { r#type: default_cache_type(), max_capacity: default_cache_capacity(), default_ttl: default_ttl() } } }
impl Default for MiddlewareConfig {
    fn default() -> Self {
        Self {
            request_id: false, request_log: false,
            request_log_config: RequestLogConfig::default(),
            auth: AuthMiddlewareConfig::default(),
            cors: CorsConfig::default(),
            compression: CompressConfig::default(),
            rate_limit: RateLimitConfig::default(),
            security_headers: SecurityHeadersConfig::default(),
            permission: PermissionConfig::default(),
        }
    }
}
impl Default for RequestLogConfig {
    fn default() -> Self { Self { exclude_paths: vec![], log_duration: true } }
}
impl Default for AuthMiddlewareConfig { fn default() -> Self { Self { enabled: false, ignore_paths: vec![], jwt_secret: String::new(), access_token_expire_secs: default_access_token_expire(), refresh_token_expire_secs: default_refresh_token_expire() } } }
impl Default for CorsConfig { fn default() -> Self { Self { enabled: false, allow_origins: vec![], allow_methods: vec![], allow_headers: vec![], allow_credentials: true, max_age_secs: default_cors_max_age() } } }
impl Default for CompressConfig { fn default() -> Self { Self { enabled: false, level: default_compress_level() } } }
impl Default for RateLimitConfig { fn default() -> Self { Self { enabled: false, requests_per_window: default_rate_limit_requests(), window_secs: default_rate_limit_window() } } }
impl Default for SecurityHeadersConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            nosniff: true, frame_options: true,
            hsts: true, hsts_max_age_secs: default_hsts_max_age(),
            hsts_include_subdomains: true,
            csp: true, csp_value: default_csp_value(),
            referrer_policy: true, referrer_policy_value: default_referrer_policy_value(),
            permissions_policy: false, permissions_policy_value: default_permissions_policy_value(),
        }
    }
}
impl Default for PermissionConfig { fn default() -> Self { Self { enabled: false, rules: vec![] } } }
impl Default for PermissionRule { fn default() -> Self { Self { path: String::new(), methods: vec![], permission: String::new() } } }
impl Default for RouterConfig { fn default() -> Self { Self { prefix: String::new(), not_found: NotFoundConfig::default() } } }
impl Default for NotFoundConfig { fn default() -> Self { Self { enabled: true, message: default_not_found_msg() } } }
impl Default for MigrationConfig { fn default() -> Self { Self { enabled: false, path: default_migration_path(), auto_migrate: false } } }
impl Default for UploadConfig { fn default() -> Self { Self { path: default_upload_path(), max_size_mb: default_max_size() } } }
impl Default for DownloadConfig { fn default() -> Self { Self { path: default_download_path() } } }
impl Default for TemplateConfig { fn default() -> Self { Self { path: default_template_path() } } }
impl Default for StaticConfig { fn default() -> Self { Self { path: default_static_path(), enabled: false } } }
impl Default for PluginsConfig { fn default() -> Self { Self { enabled: vec![], notification: NotificationConfig::default(), async_task: AsyncTaskConfig::default(), scheduler: SchedulerConfig::default(), serial: SerialConfig::default() } } }

// ──── 配置管理器 ────────────────────────────────────

/// 配置管理器——持有静态配置 + 允许运行时覆盖
pub struct ConfigManager {
    /// 静态配置（启动时加载，不可变）
    pub static_config: AppConfig,
    /// 动态配置（运行时可通过插件修改）
    pub dynamic: RwLock<HashMap<String, serde_json::Value>>,
}

impl ConfigManager {
    /// 从 config/config.toml 加载，若不存在则用默认值
    pub fn load(config_dir: Option<&str>) -> Self {
        let dir = config_dir.unwrap_or("config");
        let profile = detect_profile();

        let mut cfg = Self::load_file(dir, &profile);

        // 环境变量覆盖
        merge_env_overrides(&mut cfg);

        info!("配置加载完成 profile={}, listen={}", cfg.profile, cfg.server.listen);

        Self {
            static_config: cfg,
            dynamic: RwLock::new(HashMap::new()),
        }
    }

    fn load_file(dir: &str, profile: &str) -> AppConfig {
        // 1. 尝试 config/config.toml
        let base_path = Path::new(dir).join("config.toml");
        let mut cfg = if base_path.exists() {
            let content = fs::read_to_string(&base_path)
                .unwrap_or_else(|_| String::new());
            toml::from_str(&content).unwrap_or_default()
        } else {
            AppConfig::default()
        };

        // 2. 叠加 config.toml 中的 main_env 路径
        //    或 config/config-{profile}.toml
        let profile_path = Path::new(dir).join(format!("config-{}.toml", profile));
        if profile_path.exists() {
            if let Ok(content) = fs::read_to_string(&profile_path) {
                if let Ok(profile_cfg) = toml::from_str::<AppConfig>(&content) {
                    merge_configs(&mut cfg, &profile_cfg);
                }
            }
        }

        cfg.profile = profile.to_string();
        cfg
    }

    /// 获取静态配置引用
    pub fn get(&self) -> &AppConfig {
        &self.static_config
    }

    /// 获取动态配置值
    pub fn get_dynamic(&self, key: &str) -> Option<serde_json::Value> {
        self.dynamic.read().get(key).cloned()
    }

    /// 设置动态配置
    pub fn set_dynamic(&self, key: &str, value: serde_json::Value) {
        self.dynamic.write().insert(key.to_string(), value);
    }

    /// 删除动态配置
    pub fn remove_dynamic(&self, key: &str) {
        self.dynamic.write().remove(key);
    }

    /// 生成默认配置文件到 config/config.toml
    pub fn generate_default(dir: &str) -> std::io::Result<()> {
        let config_dir = Path::new(dir);
        fs::create_dir_all(config_dir)?;

        let cfg = AppConfig::default();
        let toml_str = toml::to_string_pretty(&cfg)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

        let header = r#"# Alun 默认配置文件
# 修改后保存即可生效（需重启服务）
#
# 使用 --gen-config 参数可重新生成此文件到 config/config.toml
# 多环境：创建 config/config-dev.toml, config/config-prod.toml
#         通过环境变量或命令行 --profile=prod 指定

"#;

        fs::write(config_dir.join("config.toml"), format!("{}{}", header, toml_str))?;
        info!("默认配置文件已生成到 {}/config.toml", dir);
        Ok(())
    }
}

/// 合并两个配置（profile 覆盖 base 中有值的字段）
fn merge_configs(base: &mut AppConfig, overlay: &AppConfig) {
    if overlay.server.listen != default_listen() { base.server.listen = overlay.server.listen.clone(); }
    if overlay.log.level != default_log_level() { base.log.level = overlay.log.level.clone(); }
    if overlay.log.format != default_log_format() { base.log.format = overlay.log.format.clone(); }
    if overlay.log.dir.is_some() { base.log.dir = overlay.log.dir.clone(); }
    if overlay.log.file_prefix != default_log_prefix() { base.log.file_prefix = overlay.log.file_prefix.clone(); }
    if overlay.database.host != default_host() || !overlay.database.name.is_empty() {
        base.database = overlay.database.clone();
    }
    if overlay.redis.url != default_redis_url() { base.redis = overlay.redis.clone(); }
    if overlay.cache.r#type != default_cache_type() { base.cache = overlay.cache.clone(); }
    if overlay.router.prefix != String::new() { base.router.prefix = overlay.router.prefix.clone(); }
    if overlay.router.not_found.message != default_not_found_msg() {
        base.router.not_found.message = overlay.router.not_found.message.clone();
    }
    if !overlay.router.not_found.enabled {
        base.router.not_found.enabled = false;
    }
    if overlay.upload.path != default_upload_path() { base.upload = overlay.upload.clone(); }
    if overlay.download.path != default_download_path() { base.download = overlay.download.clone(); }
    if overlay.template.path != default_template_path() { base.template = overlay.template.clone(); }
    if overlay.static_files.path != default_static_path() { base.static_files = overlay.static_files.clone(); }

    // 中间件采用字段级合并：仅覆盖 profile 文件中显式配置的项
    merge_middleware(&mut base.middleware, &overlay.middleware);

    // 插件采用完全替换（数组无法字段级合并）
    if !overlay.plugins.enabled.is_empty() {
        base.plugins = overlay.plugins.clone();
    }
    for (k, v) in &overlay.custom { base.custom.insert(k.clone(), v.clone()); }
}

fn merge_middleware(base: &mut MiddlewareConfig, overlay: &MiddlewareConfig) {
    let default_mw = MiddlewareConfig::default();
    if overlay.request_id != default_mw.request_id { base.request_id = overlay.request_id; }
    if overlay.request_log != default_mw.request_log { base.request_log = overlay.request_log; }
    if overlay.request_log_config.log_duration != default_mw.request_log_config.log_duration {
        base.request_log_config.log_duration = overlay.request_log_config.log_duration;
    }
    if !overlay.request_log_config.exclude_paths.is_empty() {
        base.request_log_config.exclude_paths = overlay.request_log_config.exclude_paths.clone();
    }
    if overlay.auth.enabled != default_mw.auth.enabled { base.auth.enabled = overlay.auth.enabled; }
    if overlay.auth.jwt_secret != default_mw.auth.jwt_secret { base.auth.jwt_secret = overlay.auth.jwt_secret.clone(); }
    if overlay.auth.access_token_expire_secs != 0 { base.auth.access_token_expire_secs = overlay.auth.access_token_expire_secs; }
    if overlay.auth.refresh_token_expire_secs != 0 { base.auth.refresh_token_expire_secs = overlay.auth.refresh_token_expire_secs; }
    if !overlay.auth.ignore_paths.is_empty() { base.auth.ignore_paths = overlay.auth.ignore_paths.clone(); }
    if overlay.cors.enabled != default_mw.cors.enabled { base.cors.enabled = overlay.cors.enabled; }
    if !overlay.cors.allow_origins.is_empty() { base.cors.allow_origins = overlay.cors.allow_origins.clone(); }
    if overlay.compression.enabled != default_mw.compression.enabled { base.compression.enabled = overlay.compression.enabled; }
    if overlay.rate_limit.enabled != default_mw.rate_limit.enabled { base.rate_limit.enabled = overlay.rate_limit.enabled; }
    if overlay.rate_limit.requests_per_window != 0 { base.rate_limit.requests_per_window = overlay.rate_limit.requests_per_window; }
    if overlay.rate_limit.window_secs != 0 { base.rate_limit.window_secs = overlay.rate_limit.window_secs; }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_serialization() {
        let cfg = AppConfig::default();
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        assert!(toml_str.contains("listen = \"8023\""));
        assert!(toml_str.contains("level = \"info\""));
    }
}

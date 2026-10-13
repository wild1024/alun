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

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::info;

pub mod env;
pub use env::{detect_profile, merge_env_overrides, parse_args};

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

    /// 多租户配置
    #[serde(default)]
    pub tenant: TenantConfig,

    /// 可观测性配置（`[metrics]`，F10）
    #[serde(default)]
    pub metrics: MetricsConfig,

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

    /// SSL 连接模式（postgres 专用）
    ///
    /// 可选值：`disable` / `prefer` / `require` / `verify-ca` / `verify-full`
    /// - `prefer`（默认）：尝试 SSL，失败回退明文，兼容无 SSL 旧环境
    /// - `require`：强制 SSL，生产环境推荐（需数据库端启用 SSL）
    #[serde(default = "default_ssl_mode")]
    pub ssl_mode: String,

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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

    /// 幂等键配置（`[middleware.idempotency]`）
    ///
    /// 启用后 `App` 自动把 `IdempotencyLayer` 接入默认中间件链，
    /// 无需业务代码手工挂载（需 `alun-web` 的 `cache` feature 且缓存已初始化）。
    #[serde(default)]
    pub idempotency: IdempotencyConfig,

    /// Nonce 防重放配置（`[middleware.nonce]`）
    ///
    /// 启用后 `App` 自动把 `NonceLayer` 接入默认中间件链（需 `cache` feature）。
    #[serde(default)]
    pub nonce: NonceConfig,
}

/// 多租户配置（`[tenant]`）。
///
/// 未配置或 `enabled = false` 时框架行为与单租户完全一致（零回归）：
/// 既不注入租户条件，也不做 fail-closed 守卫。
///
/// 启用后 `App` 会自动把 `TenantLayer` 挂到认证层**之内**（请求先经认证、
/// 再进入租户上下文），业务代码无需任何改动。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantConfig {
    /// 是否启用多租户隔离（默认 `false`）。
    #[serde(default)]
    pub enabled: bool,

    /// 是否启用**写路径**的租户补齐 / 校验（默认 `true`）。
    ///
    /// 仅在 `enabled = true` 时生效。迁移观察期可置为 `false`：读路径照常隔离，
    /// 写路径退化为「不补列、不校验、不追加 WHERE」，与既有服务层手写的
    /// `AND tenant_id = ?` 并存，确认读写无异常后再打开。
    ///
    /// > 关闭期间写入的租户值来自业务代码，框架不再兜底防越权，请勿长期保持关闭。
    #[serde(default = "default_true")]
    pub write_enabled: bool,

    /// JWT claim 中承载租户 ID 的字段名（默认 `tenant_id`）。
    ///
    /// 仅当请求已被认证层验证通过时才会读取该 claim（未验证的载荷一律忽略，
    /// 避免伪造租户）。
    #[serde(default = "default_tenant_claim")]
    pub claim: String,

    /// 默认租户列名（默认 `tenant_id`），供迁移 / 脚手架工具生成租户列时使用。
    ///
    /// 运行时隔离以模型上的 `#[tenant]` 声明为准，本项不参与隔离判定。
    #[serde(default = "default_tenant_column")]
    pub column: String,

    /// 跳过租户上下文注入的路径列表（如登录、健康检查），不含路由前缀。
    #[serde(default)]
    pub ignore_paths: Vec<String>,

    /// 是否启用 PostgreSQL **schema-per-tenant**（库级）隔离（默认 `false`）。
    ///
    /// 仅在 `enabled = true` 且数据库为 PostgreSQL 时生效：借出连接后按当前租户
    /// 执行 `SET search_path TO "<schema_prefix><租户ID>"`，把不同租户的数据落在
    /// 各自 schema 中，与行级 `tenant_id` 过滤形成纵深防御；SQLite / MySQL 忽略该项。
    ///
    /// > 需由运维预先建好各租户 schema 及其内表；框架只负责切换 `search_path`。
    #[serde(default)]
    pub schema_per_tenant: bool,

    /// 租户 schema 名前缀（默认 `tenant_`），最终 schema 名为 `前缀 + 租户ID`。
    ///
    /// 仅当 `schema_per_tenant = true` 时生效；空串按默认值 `tenant_` 处理。
    #[serde(default = "default_tenant_schema_prefix")]
    pub schema_prefix: String,
}

/// 可观测性配置（`[metrics]`，F10）
///
/// 控制两件事：**是否采集**与**导出端点是否暴露**。
///
/// # 安全默认
///
/// - `enabled` 默认 `false`：`/metrics` **默认不存在**，不对外暴露任何运行数据；
/// - `allow_ips` 默认为空 → 只允许**本机回环**（`127.0.0.1` / `::1`）。
///   需要 Prometheus 从外部抓取时必须显式列出抓取器 IP——这是刻意的
///   "默认私有、显式放开"，避免"开了 metrics 就等于把内部数据发到公网"。
///
/// # 与编译期 feature 的关系
///
/// 埋点代码由 `metrics` / `metrics-prometheus` feature 编译进产物；本配置段
/// 是**运行时开关**。若配置 `enabled = true` 但产物未编译对应 feature，
/// `App` 会在启动期 `warn`（fail-loud），而不是静默无效。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// 是否启用导出端点（默认 `false`）。
    ///
    /// 仅控制**端点**是否挂载；埋点本身随 feature 编译，未启用端点时仍可被
    /// 业务自行安装的 recorder 采集。
    #[serde(default)]
    pub enabled: bool,

    /// 导出端点路径（默认 `/metrics`）。
    ///
    /// 以 `/` 开头时按**绝对路径**注册，不参与 `[router].prefix` 拼接
    /// （Prometheus 抓取配置通常写死 `/metrics`）。
    #[serde(default = "default_metrics_path")]
    pub path: String,

    /// 允许访问端点的客户端 IP 白名单（默认**空** = 仅本机回环）。
    ///
    /// 取值示例：`["127.0.0.1", "10.0.0.9"]`。判定取 `X-Forwarded-For` /
    /// `X-Real-IP`（与请求日志一致），回退到 TCP 对端地址。
    #[serde(default)]
    pub allow_ips: Vec<String>,

    /// 是否启用 HTTP 自动埋点（默认 `true`）。
    ///
    /// 关闭后只保留 DB / 缓存 / 多租户安全指标。
    #[serde(default = "default_true")]
    pub http: bool,
}

/// 幂等键配置（`[middleware.idempotency]`）
///
/// 客户端携带 `x-idempotency-key` 请求头参与幂等；重复键直接返回首次响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyConfig {
    /// 是否启用（默认 `false`）
    #[serde(default)]
    pub enabled: bool,

    /// 幂等键有效期（秒），默认 86400（= 24 小时）
    #[serde(default = "default_idempotency_ttl")]
    pub ttl_secs: u64,
}

/// Nonce 防重放配置（`[middleware.nonce]`）
///
/// 客户端携带 `X-Nonce` 请求头，服务端校验后立即消费（一次性）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonceConfig {
    /// 是否启用（默认 `false`）
    #[serde(default)]
    pub enabled: bool,

    /// nonce 有效期（秒），默认 300（= 5 分钟）
    #[serde(default = "default_nonce_ttl")]
    pub ttl_secs: u64,
}

/// 请求日志中间件配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestLogConfig {
    /// 不记录日志的路径列表（如 "/health"）
    ///
    /// 默认按「不含 prefix 前缀」处理，运行时会拼上 `[router] prefix`；
    /// 以 `!` 开头表示绝对路径，不参与前缀拼接（用于豁免前缀的路径）。
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

    /// 白名单路径（如 "/system/auth/login"）
    ///
    /// 默认按「不含 prefix 前缀」处理，运行时会拼上 `[router] prefix`；
    /// 以 `!` 开头表示绝对路径，不参与前缀拼接（用于豁免前缀的路径，如 `"!/health"`）。
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionConfig {
    /// 是否启用**配置文件规则**校验（`rules` 中的路径规则）
    ///
    /// 默认 `false`：配置文件规则不生效。关闭它只跳过 `rules`，
    /// **不会**跳过 `#[permission]` 宏注解规则——宏注解是编译期契约，
    /// 只要存在注解规则就必然挂载校验层（安全默认，见 alun-ledger D007）。
    #[serde(default)]
    pub enabled: bool,

    /// 路径级权限规则
    #[serde(default)]
    pub rules: Vec<PermissionRule>,
}

/// 单条路径权限规则
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouterConfig {
    /// 全局路由前缀（如 "/api"）
    ///
    /// 非空时，`App::scan()` 收集到的所有宏声明路由会整体嵌套到该前缀下，
    /// 因此宏内路径按「不含前缀」书写；空前缀保持原样注册（向后兼容）。
    /// 中间件中的 `ignore_paths` / `exclude_paths` 等条目亦按相对路径拼接本前缀，
    /// 需要豁免前缀的路径请以 `!` 开头书写。
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

    /// 迁移文件目录（`V{n}__Name.sql` / `V{n}__Name.{up,down}.sql`）
    #[serde(default = "default_migration_path")]
    pub path: String,

    /// 启动时自动运行迁移
    #[serde(default)]
    pub auto_migrate: bool,

    /// 额外迁移来源目录（非空时**取代** `path`，支持多目录合并与子目录递归）
    #[serde(default)]
    pub sources: Vec<String>,

    /// 种子数据目录；空串表示不跑种子（目录不存在则报错，不静默跳过）
    #[serde(default)]
    pub seed_path: String,

    /// 是否按方言拆分单文件内的多语句（`01_base_tables_init.sql` 这类单文件迁移必需）
    #[serde(default = "default_true")]
    pub statement_split: bool,

    /// 是否识别单文件内的 `-- +migrate Up` / `-- +migrate Down` 双段式
    #[serde(default = "default_true")]
    pub up_down_sections: bool,

    /// 版本号前缀；空串表示自动识别（`V0002__x` 与 `01_x` 均可）
    #[serde(default)]
    pub version_prefix: String,

    /// 版本与名称之间的分隔符；空串表示自动判定（优先 `__`，回退 `_`）
    #[serde(default)]
    pub name_separator: String,

    /// 可重复迁移前缀（默认 `R`）；空串表示关闭该种类
    #[serde(default = "default_repeatable_prefix")]
    pub repeatable_prefix: String,

    /// 种子迁移前缀（默认 `S`）；空串表示关闭该种类
    #[serde(default = "default_seed_prefix")]
    pub seed_prefix: String,

    /// 基线标记前缀（默认 `B`）；空串表示关闭该种类
    #[serde(default = "default_baseline_prefix")]
    pub baseline_prefix: String,

    /// 是否校验已应用迁移的内容指纹
    #[serde(default = "default_true")]
    pub validate_checksum: bool,

    /// 指纹不一致时的策略：`abort`（默认）| `warn` | `ignore`
    #[serde(default = "default_divergent_policy")]
    pub divergent_policy: String,

    /// 是否允许乱序补迁（版本号低于已应用最大值的待应用迁移）
    ///
    /// `false`（默认）下补迁仍然可用，但会打 `WARN`；`true` 表示已声明知情。
    #[serde(default)]
    pub out_of_order: bool,

    /// 迁移默认是否包裹事务（`-- +no-transaction` 可就地覆盖）
    #[serde(default = "default_true")]
    pub transaction_default: bool,

    /// 是否导入旧框架 `_sqlx_migrations` 的已应用记录（存量库接入）
    #[serde(default)]
    pub import_legacy: bool,

    /// 是否在迁移前自动执行基线标记（存量裸库接入）
    #[serde(default)]
    pub baseline_on_migrate: bool,

    /// 基线版本；`<=` 该版本的迁移视为已应用（仅 `baseline_on_migrate` 时生效）
    #[serde(default)]
    pub baseline_version: String,

    /// 脚本占位符：替换迁移 SQL 中的 `${key}`（空表 = 不做任何替换）
    ///
    /// 替换发生在分段与语句拆分之前，因此指纹覆盖替换后的 SQL：改动占位符取值
    /// 会被 `DbError::MigrationChecksum` 检出，而不会静默执行不同的 SQL。
    #[serde(default)]
    pub placeholders: std::collections::HashMap<String, String>,
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

fn default_smtp_port() -> u16 {
    587
}

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
        Self {
            backend: "memory".into(),
            rules: Vec::new(),
        }
    }
}

fn default_serial_backend() -> String {
    "memory".into()
}

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

fn default_cycle() -> String {
    "no_cycle".into()
}
fn default_initial() -> u64 {
    1
}
fn default_step_strategy() -> String {
    "sequential".into()
}

// ──── 默认值函数 ────────────────────────────────────

fn default_app_name() -> String {
    "Alun".into()
}
fn default_profile() -> String {
    "dev".into()
}
fn default_listen() -> String {
    "8023".into()
}
fn default_log_level() -> String {
    "info".into()
}
fn default_log_format() -> String {
    "text".into()
}
fn default_log_prefix() -> String {
    "alun".into()
}
fn default_db_type() -> String {
    "postgres".into()
}
fn default_host() -> String {
    "localhost".into()
}
fn default_true() -> bool {
    true
}
fn default_metrics_path() -> String {
    "/metrics".into()
}
fn default_ssl_mode() -> String {
    "prefer".into()
}
fn default_pool_size() -> u32 {
    10
}
fn default_min_idle() -> u32 {
    2
}
fn default_timeout() -> u64 {
    10
}
fn default_workers() -> usize {
    4
}
fn default_redis_url() -> String {
    "redis://127.0.0.1:6379".into()
}
fn default_cache_type() -> String {
    "local".into()
}
fn default_cache_capacity() -> u64 {
    10000
}
fn default_ttl() -> u64 {
    3600
}
fn default_access_token_expire() -> u64 {
    7200
}
fn default_refresh_token_expire() -> u64 {
    604800
}
fn default_cors_max_age() -> u64 {
    86400
}
fn default_compress_level() -> u32 {
    6
}
fn default_rate_limit_requests() -> u64 {
    100
}
fn default_rate_limit_window() -> u64 {
    60
}
fn default_idempotency_ttl() -> u64 {
    86400
}
fn default_nonce_ttl() -> u64 {
    300
}
fn default_tenant_claim() -> String {
    "tenant_id".into()
}
fn default_tenant_column() -> String {
    "tenant_id".into()
}
fn default_tenant_schema_prefix() -> String {
    "tenant_".into()
}
fn default_hsts_max_age() -> u64 {
    31536000
}
fn default_csp_value() -> String {
    "default-src 'self'".into()
}
fn default_referrer_policy_value() -> String {
    "strict-origin-when-cross-origin".into()
}
fn default_permissions_policy_value() -> String {
    "camera=(), microphone=(), geolocation=()".into()
}
fn default_migration_path() -> String {
    "migrations".into()
}
/// 指纹不一致时的默认策略：中止（保留强校验语义）。
fn default_divergent_policy() -> String {
    "abort".into()
}
fn default_repeatable_prefix() -> String {
    "R".into()
}
fn default_seed_prefix() -> String {
    "S".into()
}
fn default_baseline_prefix() -> String {
    "B".into()
}
fn default_upload_path() -> String {
    "uploads".into()
}
fn default_download_path() -> String {
    "downloads".into()
}
fn default_template_path() -> String {
    "templates".into()
}
fn default_static_path() -> String {
    "static".into()
}
fn default_not_found_msg() -> String {
    "请求的资源不存在".into()
}
fn default_max_size() -> u64 {
    10
}

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
            tenant: TenantConfig::default(),
            metrics: MetricsConfig::default(),
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

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
        }
    }
}
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
            enabled: false,
            r#type: default_db_type(),
            host: default_host(),
            port: None,
            name: String::new(),
            user: String::new(),
            password: String::new(),
            password_encrypted: false,
            max_connections: default_pool_size(),
            min_connections: default_min_idle(),
            connect_timeout: default_timeout(),
            sql_logging: false,
            slow_query_ms: 0,
            ssl_mode: default_ssl_mode(),
            migration: MigrationConfig::default(),
        }
    }
}
impl Default for RedisConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: default_redis_url(),
            max_connections: default_pool_size(),
        }
    }
}
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            r#type: default_cache_type(),
            max_capacity: default_cache_capacity(),
            default_ttl: default_ttl(),
        }
    }
}
impl Default for IdempotencyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_secs: default_idempotency_ttl(),
        }
    }
}
impl Default for NonceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_secs: default_nonce_ttl(),
        }
    }
}
impl Default for TenantConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            write_enabled: true,
            claim: default_tenant_claim(),
            column: default_tenant_column(),
            ignore_paths: Vec::new(),
            schema_per_tenant: false,
            schema_prefix: default_tenant_schema_prefix(),
        }
    }
}
impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: default_metrics_path(),
            allow_ips: Vec::new(),
            http: true,
        }
    }
}

impl Default for RequestLogConfig {
    fn default() -> Self {
        Self {
            exclude_paths: vec![],
            log_duration: true,
        }
    }
}
impl Default for AuthMiddlewareConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ignore_paths: vec![],
            jwt_secret: String::new(),
            access_token_expire_secs: default_access_token_expire(),
            refresh_token_expire_secs: default_refresh_token_expire(),
        }
    }
}
impl Default for CorsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_origins: vec![],
            allow_methods: vec![],
            allow_headers: vec![],
            allow_credentials: true,
            max_age_secs: default_cors_max_age(),
        }
    }
}
impl Default for CompressConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            level: default_compress_level(),
        }
    }
}
impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            requests_per_window: default_rate_limit_requests(),
            window_secs: default_rate_limit_window(),
        }
    }
}
impl Default for SecurityHeadersConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            nosniff: true,
            frame_options: true,
            hsts: true,
            hsts_max_age_secs: default_hsts_max_age(),
            hsts_include_subdomains: true,
            csp: true,
            csp_value: default_csp_value(),
            referrer_policy: true,
            referrer_policy_value: default_referrer_policy_value(),
            permissions_policy: false,
            permissions_policy_value: default_permissions_policy_value(),
        }
    }
}
impl Default for NotFoundConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            message: default_not_found_msg(),
        }
    }
}
impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: default_migration_path(),
            auto_migrate: false,
            sources: vec![],
            seed_path: String::new(),
            statement_split: true,
            up_down_sections: true,
            version_prefix: String::new(),
            name_separator: String::new(),
            repeatable_prefix: default_repeatable_prefix(),
            seed_prefix: default_seed_prefix(),
            baseline_prefix: default_baseline_prefix(),
            validate_checksum: true,
            divergent_policy: default_divergent_policy(),
            out_of_order: false,
            transaction_default: true,
            import_legacy: false,
            baseline_on_migrate: false,
            baseline_version: String::new(),
            placeholders: std::collections::HashMap::new(),
        }
    }
}
impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            path: default_upload_path(),
            max_size_mb: default_max_size(),
        }
    }
}
impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            path: default_download_path(),
        }
    }
}
impl Default for TemplateConfig {
    fn default() -> Self {
        Self {
            path: default_template_path(),
        }
    }
}
impl Default for StaticConfig {
    fn default() -> Self {
        Self {
            path: default_static_path(),
            enabled: false,
        }
    }
}
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

        info!(
            "配置加载完成 profile={}, listen={}",
            cfg.profile, cfg.server.listen
        );

        Self {
            static_config: cfg,
            dynamic: RwLock::new(HashMap::new()),
        }
    }

    fn load_file(dir: &str, profile: &str) -> AppConfig {
        // 1. 尝试 config/config.toml
        let base_path = Path::new(dir).join("config.toml");
        let mut cfg = if base_path.exists() {
            let content = fs::read_to_string(&base_path).unwrap_or_else(|_| String::new());
            toml::from_str(&content).unwrap_or_default()
        } else {
            AppConfig::default()
        };

        // 2. 叠加 config.toml 中的 main_env 路径
        //    或 config/config-{profile}.toml
        let profile_path = Path::new(dir).join(format!("config-{}.toml", profile));
        if profile_path.exists()
            && let Ok(content) = fs::read_to_string(&profile_path)
            && let Ok(profile_cfg) = toml::from_str::<AppConfig>(&content)
        {
            merge_configs(&mut cfg, &profile_cfg);
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
        let toml_str =
            toml::to_string_pretty(&cfg).map_err(|e| std::io::Error::other(e.to_string()))?;

        let header = r#"# Alun 默认配置文件
# 修改后保存即可生效（需重启服务）
#
# 使用 --gen-config 参数可重新生成此文件到 config/config.toml
# 多环境：创建 config/config-dev.toml, config/config-prod.toml
#         通过环境变量或命令行 --profile=prod 指定

"#;

        fs::write(
            config_dir.join("config.toml"),
            format!("{}{}", header, toml_str),
        )?;
        info!("默认配置文件已生成到 {}/config.toml", dir);
        Ok(())
    }
}

/// 合并两个配置（profile 覆盖 base 中有值的字段）
fn merge_configs(base: &mut AppConfig, overlay: &AppConfig) {
    if overlay.server.listen != default_listen() {
        base.server.listen = overlay.server.listen.clone();
    }
    if overlay.log.level != default_log_level() {
        base.log.level = overlay.log.level.clone();
    }
    if overlay.log.format != default_log_format() {
        base.log.format = overlay.log.format.clone();
    }
    if overlay.log.dir.is_some() {
        base.log.dir = overlay.log.dir.clone();
    }
    if overlay.log.file_prefix != default_log_prefix() {
        base.log.file_prefix = overlay.log.file_prefix.clone();
    }
    // 数据库采用字段级合并：仅覆盖 profile 文件中显式配置的项
    // 修复：此前仅当 host/name 变化时才整体替换 database 段，
    // 导致 ssl_mode/max_connections 等字段在 host 走 env 覆盖时被忽略
    merge_database(&mut base.database, &overlay.database);
    if overlay.redis.url != default_redis_url() {
        base.redis = overlay.redis.clone();
    }
    if overlay.cache.r#type != default_cache_type() {
        base.cache = overlay.cache.clone();
    }
    if overlay.router.prefix != String::new() {
        base.router.prefix = overlay.router.prefix.clone();
    }
    if overlay.router.not_found.message != default_not_found_msg() {
        base.router.not_found.message = overlay.router.not_found.message.clone();
    }
    if !overlay.router.not_found.enabled {
        base.router.not_found.enabled = false;
    }
    if overlay.upload.path != default_upload_path() {
        base.upload = overlay.upload.clone();
    }
    if overlay.download.path != default_download_path() {
        base.download = overlay.download.clone();
    }
    if overlay.template.path != default_template_path() {
        base.template = overlay.template.clone();
    }
    if overlay.static_files.path != default_static_path() {
        base.static_files = overlay.static_files.clone();
    }

    // 中间件采用字段级合并：仅覆盖 profile 文件中显式配置的项
    merge_middleware(&mut base.middleware, &overlay.middleware);

    // 可观测性：开启类布尔取"或"，路径与白名单按"显式配置才覆盖"
    if overlay.metrics.enabled {
        base.metrics.enabled = true;
    }
    if overlay.metrics.path != default_metrics_path() {
        base.metrics.path = overlay.metrics.path.clone();
    }
    if !overlay.metrics.allow_ips.is_empty() {
        base.metrics.allow_ips = overlay.metrics.allow_ips.clone();
    }
    if !overlay.metrics.http {
        base.metrics.http = false;
    }

    // 插件采用完全替换（数组无法字段级合并）
    if !overlay.plugins.enabled.is_empty() {
        base.plugins = overlay.plugins.clone();
    }
    for (k, v) in &overlay.custom {
        base.custom.insert(k.clone(), v.clone());
    }
}

/// 数据库配置字段级合并
///
/// 仅覆盖 profile 文件中显式配置的字段（与默认值不同的字段），
/// 未在 profile 中设置的字段保留 base 配置的值。
fn merge_database(base: &mut DatabaseConfig, overlay: &DatabaseConfig) {
    if overlay.enabled {
        base.enabled = true;
    }
    if overlay.r#type != default_db_type() {
        base.r#type = overlay.r#type.clone();
    }
    if overlay.host != default_host() {
        base.host = overlay.host.clone();
    }
    if overlay.port.is_some() {
        base.port = overlay.port;
    }
    if !overlay.name.is_empty() {
        base.name = overlay.name.clone();
    }
    if !overlay.user.is_empty() {
        base.user = overlay.user.clone();
    }
    if !overlay.password.is_empty() {
        base.password = overlay.password.clone();
    }
    if overlay.password_encrypted {
        base.password_encrypted = true;
    }
    if overlay.max_connections != default_pool_size() {
        base.max_connections = overlay.max_connections;
    }
    if overlay.min_connections != default_min_idle() {
        base.min_connections = overlay.min_connections;
    }
    if overlay.connect_timeout != default_timeout() {
        base.connect_timeout = overlay.connect_timeout;
    }
    if overlay.sql_logging {
        base.sql_logging = true;
    }
    if overlay.slow_query_ms != 0 {
        base.slow_query_ms = overlay.slow_query_ms;
    }
    if overlay.ssl_mode != default_ssl_mode() {
        base.ssl_mode = overlay.ssl_mode.clone();
    }
    if overlay.migration.enabled {
        base.migration.enabled = true;
    }
    if overlay.migration.path != default_migration_path() {
        base.migration.path = overlay.migration.path.clone();
    }
    if overlay.migration.auto_migrate {
        base.migration.auto_migrate = true;
    }
}

fn merge_middleware(base: &mut MiddlewareConfig, overlay: &MiddlewareConfig) {
    let default_mw = MiddlewareConfig::default();
    if overlay.request_id != default_mw.request_id {
        base.request_id = overlay.request_id;
    }
    if overlay.request_log != default_mw.request_log {
        base.request_log = overlay.request_log;
    }
    if overlay.request_log_config.log_duration != default_mw.request_log_config.log_duration {
        base.request_log_config.log_duration = overlay.request_log_config.log_duration;
    }
    if !overlay.request_log_config.exclude_paths.is_empty() {
        base.request_log_config.exclude_paths = overlay.request_log_config.exclude_paths.clone();
    }
    if overlay.auth.enabled != default_mw.auth.enabled {
        base.auth.enabled = overlay.auth.enabled;
    }
    if overlay.auth.jwt_secret != default_mw.auth.jwt_secret {
        base.auth.jwt_secret = overlay.auth.jwt_secret.clone();
    }
    if overlay.auth.access_token_expire_secs != 0 {
        base.auth.access_token_expire_secs = overlay.auth.access_token_expire_secs;
    }
    if overlay.auth.refresh_token_expire_secs != 0 {
        base.auth.refresh_token_expire_secs = overlay.auth.refresh_token_expire_secs;
    }
    if !overlay.auth.ignore_paths.is_empty() {
        base.auth.ignore_paths = overlay.auth.ignore_paths.clone();
    }
    if overlay.cors.enabled != default_mw.cors.enabled {
        base.cors.enabled = overlay.cors.enabled;
    }
    if !overlay.cors.allow_origins.is_empty() {
        base.cors.allow_origins = overlay.cors.allow_origins.clone();
    }
    if overlay.compression.enabled != default_mw.compression.enabled {
        base.compression.enabled = overlay.compression.enabled;
    }

    // rate_limit：如果 profile 中显式配置了任何字段（与默认值不同），整体替换
    // 修复：此前 enabled=false 与默认值 false 相同，导致无法覆盖 config.toml 中的 enabled=true
    let default_rl = RateLimitConfig::default();
    if overlay.rate_limit.enabled != default_rl.enabled
        || overlay.rate_limit.requests_per_window != default_rl.requests_per_window
        || overlay.rate_limit.window_secs != default_rl.window_secs
    {
        base.rate_limit = overlay.rate_limit.clone();
    }

    // security_headers 字段级合并（与默认值比较，仅覆盖显式配置的字段）
    if overlay.security_headers.enabled != default_mw.security_headers.enabled {
        base.security_headers.enabled = overlay.security_headers.enabled;
    }
    if !overlay.security_headers.hsts {
        base.security_headers.hsts = false;
    }
    if !overlay.security_headers.csp {
        base.security_headers.csp = false;
    }
    if overlay.security_headers.csp_value != default_mw.security_headers.csp_value {
        base.security_headers.csp_value = overlay.security_headers.csp_value.clone();
    }
    if !overlay.security_headers.permissions_policy {
        base.security_headers.permissions_policy = false;
    }

    // permission 字段级合并
    if overlay.permission.enabled != default_mw.permission.enabled {
        base.permission.enabled = overlay.permission.enabled;
    }
    if !overlay.permission.rules.is_empty() {
        base.permission.rules = overlay.permission.rules.clone();
    }

    // idempotency / nonce 字段级合并（与默认值比较，仅覆盖 profile 中显式配置的字段）
    if overlay.idempotency.enabled != default_mw.idempotency.enabled {
        base.idempotency.enabled = overlay.idempotency.enabled;
    }
    if overlay.idempotency.ttl_secs != default_mw.idempotency.ttl_secs {
        base.idempotency.ttl_secs = overlay.idempotency.ttl_secs;
    }
    if overlay.nonce.enabled != default_mw.nonce.enabled {
        base.nonce.enabled = overlay.nonce.enabled;
    }
    if overlay.nonce.ttl_secs != default_mw.nonce.ttl_secs {
        base.nonce.ttl_secs = overlay.nonce.ttl_secs;
    }
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

    #[test]
    fn middleware_idempotency_and_nonce_toml_sections_are_accepted() {
        // 声明式启用的公开契约：键名写错会被 serde 静默忽略 → 中间件不会装配
        let cfg: AppConfig = toml::from_str(
            r#"
            [middleware.idempotency]
            enabled = true
            ttl_secs = 120

            [middleware.nonce]
            enabled = true
            ttl_secs = 30
            "#,
        )
        .unwrap();

        assert!(cfg.middleware.idempotency.enabled);
        assert_eq!(cfg.middleware.idempotency.ttl_secs, 120);
        assert!(cfg.middleware.nonce.enabled);
        assert_eq!(cfg.middleware.nonce.ttl_secs, 30);
    }

    #[test]
    fn middleware_idempotency_and_nonce_default_to_disabled() {
        // 向后兼容：未配置时不改变既有中间件链
        let cfg = AppConfig::default();
        assert!(!cfg.middleware.idempotency.enabled);
        assert!(!cfg.middleware.nonce.enabled);
        assert_eq!(cfg.middleware.idempotency.ttl_secs, 86400);
        assert_eq!(cfg.middleware.nonce.ttl_secs, 300);

        // 缺省 section 时沿用默认值（老配置文件不写这两个 section 也能解析）
        let parsed: AppConfig = toml::from_str("[middleware]\nrequest_id = true\n").unwrap();
        assert!(parsed.middleware.request_id);
        assert!(!parsed.middleware.idempotency.enabled);
        assert!(!parsed.middleware.nonce.enabled);
    }
}

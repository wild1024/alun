//! 全局资源注册表
//!
//! 所有资源统一通过全局单例访问，代码更简洁。
//!
//! # 示例
//!
//! ```ignore
//! use alun::{db, cfg, render_template};
//!
//! async fn query_user(id: &str) -> Result<Option<Row>, String> {
//!     let users = db().find_by_id("user", id).await?;
//!     Ok(users)
//! }
//!
//! async fn page_home() -> Result<Html<String>, String> {
//!     let content = render_template("home.html", &json!({"title": cfg().app_name}))?;
//!     Ok(Html(content))
//! }
//! ```

use std::sync::Arc;
use std::sync::OnceLock;

#[cfg(feature = "cache")]
use alun_cache::SharedCache;
use alun_config::ConfigManager;
#[cfg(feature = "db")]
use alun_db::Db;
#[cfg(feature = "template")]
use alun_template::TemplateEngine;

// 全局资源实例（单例，标准库 OnceLock）
#[cfg(feature = "db")]
static DB: OnceLock<Db> = OnceLock::new();
/// 命名数据源注册表（F14）。
///
/// 存的是 `&'static Db`（注册时 `Box::leak`），因此可以安全地把引用**拷贝**出
/// 读锁返回，而不必让返回值绑定到锁守卫的生命周期。
#[cfg(feature = "db")]
static DB_NAMED: OnceLock<std::sync::RwLock<std::collections::HashMap<String, &'static Db>>> =
    OnceLock::new();
#[cfg(feature = "cache")]
static CACHE: OnceLock<SharedCache> = OnceLock::new();
static CONFIG: OnceLock<Arc<ConfigManager>> = OnceLock::new();
#[cfg(feature = "template")]
static TEMPLATE: OnceLock<TemplateEngine> = OnceLock::new();
static UPLOAD_PATH: OnceLock<String> = OnceLock::new();
static DOWNLOAD_PATH: OnceLock<String> = OnceLock::new();

// ── 设置资源（框架启动时调用）───────────────────────────────────────

/// 初始化数据库
#[cfg(feature = "db")]
pub fn set_db(db: Db) -> Result<(), &'static str> {
    DB.set(db).map_err(|_| "数据库资源已初始化")
}

/// 注册一个**命名数据源**（F14）。
///
/// 用于报表库分离 / 只读副本 / 按业务分库等场景：默认数据源仍走 [`db`]，
/// 额外数据源按名字取用 [`db_named`]。
///
/// # 事务与副本的约束（**必须遵守**）
///
/// 框架**不做**自动读写分离（`SELECT` 自动走副本）——主从延迟 + 事务内读会让
/// 语义不可推理。因此：
/// - 副本 / 报表库应通过本方法以只读账号注册；
/// - **事务必须由主库句柄创建**（`db().begin()`），不要让副本参与写事务。
///
/// # 错误
///
/// 同名数据源已注册时返回 `Err`。注意：发生该错误时被拒绝的连接已被
/// `Box::leak` 泄漏（进程级、有界，可接受）。
#[cfg(feature = "db")]
pub fn set_db_named(name: impl Into<String>, db: Db) -> Result<(), String> {
    let name = name.into();
    if name.is_empty() {
        return Err("命名数据源的名字不能为空".into());
    }
    let leaked: &'static Db = Box::leak(Box::new(db));
    let registry =
        DB_NAMED.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()));
    let mut guard = registry
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.contains_key(&name) {
        return Err(format!("命名数据源 `{name}` 已注册"));
    }
    guard.insert(name, leaked);
    Ok(())
}

/// 获取命名数据源；不存在时返回 `None`。
#[cfg(feature = "db")]
pub fn try_db_named(name: &str) -> Option<&'static Db> {
    let registry = DB_NAMED.get()?;
    let guard = registry
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.get(name).copied()
}

/// 获取命名数据源；不存在时 panic（与 [`db`] 一致的 fail-fast 取向）。
///
/// # Panics
///
/// 指定名字的数据源未注册。
#[cfg(feature = "db")]
pub fn db_named(name: &str) -> &'static Db {
    try_db_named(name).unwrap_or_else(|| {
        panic!("命名数据源 `{name}` 未注册，请先调用 set_db_named(\"{name}\", db)")
    })
}

/// 初始化缓存
#[cfg(feature = "cache")]
pub fn set_cache(cache: SharedCache) -> Result<(), &'static str> {
    CACHE.set(cache).map_err(|_| "缓存资源已初始化")
}

/// 初始化配置
pub fn set_config(config: Arc<ConfigManager>) -> Result<(), &'static str> {
    CONFIG.set(config).map_err(|_| "配置资源已初始化")
}

/// 初始化模板引擎
#[cfg(feature = "template")]
pub fn set_template(template: TemplateEngine) -> Result<(), &'static str> {
    TEMPLATE.set(template).map_err(|_| "模板引擎已初始化")
}

// ── 获取资源（业务代码调用）────────────────────────────────────────

/// 获取全局数据库实例
#[cfg(feature = "db")]
pub fn db() -> &'static Db {
    DB.get().expect("数据库未初始化，请先调用 set_db()")
}

/// 安全获取数据库（返回 Option）
#[cfg(feature = "db")]
pub fn try_db() -> Option<&'static Db> {
    DB.get()
}

/// 获取全局缓存实例
#[cfg(feature = "cache")]
pub fn cache() -> &'static SharedCache {
    CACHE.get().expect("缓存未初始化，请先调用 set_cache()")
}

/// 安全获取缓存（返回 Option）
#[cfg(feature = "cache")]
pub fn try_cache() -> Option<&'static SharedCache> {
    CACHE.get()
}

/// 获取**按当前租户隔离**的缓存视图（fail-closed）。
///
/// 这是多租户下读写缓存的推荐入口：所有 key 自动加上 `tenant:{租户键}:`
/// 前缀，租户之间互不可见。
///
/// # 错误
///
/// - 无租户上下文 → 报错（**不退回全局命名空间**，否则会串租户）
/// - 显式跨租户（`TenantContext::Cross`）→ 报错（跨租户没有单一命名空间语义；
///   如需全局缓存请显式调用 [`cache`]）
/// - 租户值不能作为命名空间片段（含非法字符 / 类型不支持）→ 报错
///
/// # 示例
///
/// ```ignore
/// use alun::cache_tenant;
///
/// let ns = cache_tenant()?;
/// ns.set("dict:order_status", &statuses).await?;
/// ```
#[cfg(all(feature = "cache", feature = "db"))]
pub fn cache_tenant() -> alun_core::Result<alun_cache::CacheNamespace<'static, SharedCache>> {
    use alun_core::Error;

    let key = alun_db::prelude::current_tenant_key()
        .map_err(|e| Error::Msg(format!("{e}")))?
        .ok_or_else(|| {
            Error::Msg(
                "cache_tenant() 需要租户上下文：请在 tenant_scope(..) 内调用，\
                 或改用全局 cache() 明确使用全局命名空间"
                    .into(),
            )
        })?;

    Ok(cache().scoped(format!("tenant:{key}")))
}

/// 获取全局配置管理器
pub fn config() -> &'static Arc<ConfigManager> {
    CONFIG.get().expect("配置未初始化，请先调用 set_config()")
}

/// 安全获取配置（返回 Option）
pub fn try_config() -> Option<&'static Arc<ConfigManager>> {
    CONFIG.get()
}

/// 获取全局配置（快捷方式）
pub fn cfg() -> &'static alun_config::AppConfig {
    config().get()
}

/// 渲染模板（便捷函数）
#[cfg(feature = "template")]
pub fn render_template<T: serde::Serialize>(name: &str, context: &T) -> alun_core::Result<String> {
    TEMPLATE
        .get()
        .ok_or_else(|| alun_core::Error::Template("模板引擎未初始化".into()))
        .and_then(|t| t.render(name, context))
}

/// 安全获取模板引擎（返回 Option）
#[cfg(feature = "template")]
pub fn try_template() -> Option<&'static TemplateEngine> {
    TEMPLATE.get()
}

/// 初始化上传文件存储路径
pub fn set_upload_path(path: String) -> Result<(), &'static str> {
    UPLOAD_PATH.set(path).map_err(|_| "上传路径已初始化")
}

/// 获取上传文件存储路径
pub fn upload_path() -> &'static str {
    UPLOAD_PATH.get().map(|s| s.as_str()).unwrap_or("uploads")
}

/// 安全获取上传路径（返回 Option）
pub fn try_upload_path() -> Option<&'static str> {
    UPLOAD_PATH.get().map(|s| s.as_str())
}

/// 初始化下载文件存储路径
pub fn set_download_path(path: String) -> Result<(), &'static str> {
    DOWNLOAD_PATH.set(path).map_err(|_| "下载路径已初始化")
}

/// 获取下载文件存储路径
pub fn download_path() -> &'static str {
    DOWNLOAD_PATH
        .get()
        .map(|s| s.as_str())
        .unwrap_or("downloads")
}

/// 安全获取下载路径（返回 Option）
pub fn try_download_path() -> Option<&'static str> {
    DOWNLOAD_PATH.get().map(|s| s.as_str())
}

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

use std::sync::OnceLock;
use std::sync::Arc;

#[cfg(feature = "db")]
use alun_db::Db;
#[cfg(feature = "cache")]
use alun_cache::SharedCache;
use alun_config::ConfigManager;
#[cfg(feature = "template")]
use alun_template::TemplateEngine;

// 全局资源实例（单例，标准库 OnceLock）
#[cfg(feature = "db")]
static DB: OnceLock<Db> = OnceLock::new();
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
pub fn render_template<T: serde::Serialize>(
    name: &str,
    context: &T,
) -> alun_core::Result<String> {
    TEMPLATE.get()
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
    DOWNLOAD_PATH.get().map(|s| s.as_str()).unwrap_or("downloads")
}

/// 安全获取下载路径（返回 Option）
pub fn try_download_path() -> Option<&'static str> {
    DOWNLOAD_PATH.get().map(|s| s.as_str())
}

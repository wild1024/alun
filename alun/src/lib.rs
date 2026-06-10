//! # Alun —— 快、简、美的 Rust Web 框架
//!
//! 配置驱动，一行启动，零成本抽象。
//!
//! # 快速开始
//!
//! ```ignore
//! use alun::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> alun::Result<()> {
//!     App::from_config()?   // 从 config/config.toml 自动加载
//!         .get("/", || async { Res::ok("Hello, Alun!") })
//!         .parse_cli()      // 支持 gen-config 生成配置文件
//!         .serve("8080")
//!         .await
//! }
//! ```

pub mod prelude {
    pub use alun_core::{Result, Error, Plugin, PluginManager, Res, ResResult, ApiError, PageData, PageQuery, codes};
    pub use alun_web::resources::{cfg, config, try_config, set_config};

    #[cfg(feature = "db")]
    pub use alun_web::resources::{db, try_db, set_db};

    #[cfg(feature = "cache")]
    pub use alun_web::resources::{cache, try_cache, set_cache};

    #[cfg(feature = "template")]
    pub use alun_web::resources::{render_template, try_template, set_template};

    pub use alun_web::{App, AlunRouter, TokenClaims, TokenType, UserId, AuthClaims, ValidatedJson, JWT};
    pub use alun_web::middleware::{NonceLayer, IdempotencyLayer};
    pub use alun_config::AppConfig;

    #[cfg(feature = "db")]
    pub use alun_db::{Db, Row, ActiveTx, Isolation, Hook, NullHook, HookChain, factory};

    pub use serde_json::{json, Value as JsonValue};

    #[cfg(feature = "template")]
    pub use alun_template::TemplateEngine;

    #[cfg(feature = "cache")]
    pub use alun_cache::{Cache, CacheStats, LocalCache};

    #[cfg(feature = "plugin")]
    pub use alun_plugin;

    pub use axum::response::Json as AxumJson;
    pub use axum::extract::{Path, Query};
    pub use axum::Extension;
}

pub use alun_macros::{get, post, put, delete, controller, plugin, permission, task_handler};
pub use alun_core::{Result, Error, Res, ApiError};
pub use alun_web::resources::{cfg, config, try_config, set_config};

#[cfg(feature = "db")]
pub use alun_web::resources::{db, try_db, set_db};

#[cfg(feature = "cache")]
pub use alun_web::resources::{cache, try_cache, set_cache};

#[cfg(feature = "template")]
pub use alun_web::resources::{render_template, try_template, set_template};

pub use alun_db::{Db, Row, IdKind};
pub use alun_web::{App, AlunRouter, TokenClaims, TokenType, UserId, AuthClaims, ValidatedJson, JWT, ROUTES, PermissionDef, PERMISSION_ROUTES, NoAuthDef, NO_AUTH_ROUTES};
pub use alun_web::{validate_uuid, validate_mobile, validate_password_strength, validate_id_card, validate_date, validate_email, validate_url, validate_datetime, validate_date_or_datetime};
pub use alun_web::ValidateExt;
pub use alun_config::AppConfig;

pub use linkme::distributed_slice;
pub use uuid;

/// Web 子模块（完整导出）
pub use alun_web as web;

#[cfg(feature = "kafka")]
pub use alun_kafka;

#[cfg(feature = "task")]
pub use alun_task;

#[cfg(feature = "fs")]
pub use alun_fs;

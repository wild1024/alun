//! Alun web layer: 路由、中间件、提取器、响应、全局资源

pub mod app;
pub mod router;
pub mod middleware;
pub mod jwt;
pub mod response;
pub mod extract;
pub mod resources;

pub use app::App;
pub use router::AlunRouter;
pub use response::{Res, ResResult, ApiError, PageData};
pub use middleware::{UserId, AuthClaims, AuthLayer, RequireRoleLayer, RequirePermissionLayer, TokenClaims, TokenType};
pub use jwt::JWT;
pub use extract::ValidatedJson;

pub use resources::{cfg, config, try_config, set_config};
pub use resources::{upload_path, try_upload_path, set_upload_path};
pub use resources::{download_path, try_download_path, set_download_path};

#[cfg(feature = "db")]
pub use resources::{db, try_db, set_db};

#[cfg(feature = "cache")]
pub use resources::{cache, try_cache, set_cache};

#[cfg(feature = "template")]
pub use resources::{render_template, try_template, set_template};

/// 路由注册分布式切片 —— `#[get]`、`#[post]` 等宏注解的处理器在此汇集
#[linkme::distributed_slice]
pub static ROUTES: [fn(&mut AlunRouter)] = [..];

/// 路径权限定义 —— 由 `#[permission("xxx")]` 宏注解生成
///
/// 每条记录描述一个接口路径所需的权限标识。
/// 运行时通过 `PermissionCheckLayer` 中间件校验。
#[derive(Debug, Clone)]
pub struct PermissionDef {
    /// URL 路径
    pub path: &'static str,
    /// HTTP 方法
    pub method: &'static str,
    /// 所需权限标识
    pub permission: &'static str,
}

/// 路径权限分布式切片 —— `#[permission]` 宏注解的权限规则在此汇集
#[linkme::distributed_slice]
pub static PERMISSION_ROUTES: [PermissionDef] = [..];

/// 无需认证的路径定义 —— 由 `#[no_auth]` 宏注解生成
///
/// 在路径列表中指定的接口将绕过 AuthLayer 中间件。
#[derive(Debug, Clone)]
pub struct NoAuthDef {
    /// URL 路径
    pub path: &'static str,
}

/// 无需认证路径分布式切片 —— `#[no_auth]` 宏注解的路径在此汇集
#[linkme::distributed_slice]
pub static NO_AUTH_ROUTES: [NoAuthDef] = [..];

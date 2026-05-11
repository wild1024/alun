//! Alun 请求中间件模块
//!
//! 按职责拆分为多个子模块：
//! | 模块               | 功能                 |
//! |-------------------|---------------------|
//! | [request_id]      | 请求 ID 生成/注入     |
//! | [auth]            | JWT Bearer Token 认证 |
//! | [role]            | 角色校验              |
//! | [permission]      | 权限标识校验          |
//! | [permission_check]| 路径权限规则校验       |
//! | [request_log]     | 请求访问日志（tracing 输出） |
//! | [rate_limit]      | IP 滑动窗口限流       |
//! | [security_headers]| 安全响应头注入         |
//! | [nonce]           | Nonce 防重放（按需）    |
//! | [idempotency]     | 幂等键（按需）         |

pub mod request_id;
pub mod auth;
pub mod role;
pub mod permission;
pub mod permission_check;
pub mod request_log;
pub mod rate_limit;
pub mod security_headers;
#[cfg(feature = "cache")]
pub mod nonce;
#[cfg(feature = "cache")]
pub mod idempotency;

pub use request_id::{RequestIdLayer, RequestIdService};
pub use auth::{AuthLayer, AuthService};
pub use role::{RequireRoleLayer, RequireRoleService};
pub use permission::{RequirePermissionLayer, RequirePermissionService};
pub use permission_check::{PermissionCheckLayer, PermissionRule, PermissionCheckService};
pub use request_log::{RequestLogLayer, RequestLogService};
pub use rate_limit::{RateLimitLayer, IpWindow, RateLimitService};
pub use security_headers::{SecurityHeadersLayer, SecurityHeadersService};
#[cfg(feature = "cache")]
pub use nonce::{NonceLayer, NonceService};
#[cfg(feature = "cache")]
pub use idempotency::{IdempotencyLayer, IdempotencyService};

/// Token 类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TokenType {
    /// 访问 Token
    #[serde(rename = "access")]
    Access,
    /// 刷新 Token
    #[serde(rename = "refresh")]
    Refresh,
}

/// JWT Token 声明（与 jsonwebtoken 配合使用）
///
/// 由 AuthLayer 中间件验证后注入到 `request.extensions`，
/// 业务代码可通过 `Extension<AuthClaims>` 提取。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenClaims {
    /// JWT 唯一标识（jti），用于 Token 黑名单/撤销机制
    #[serde(default)]
    pub jti: Option<String>,
    /// 用户 ID（subject）
    pub sub: String,
    /// 用户名
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// 角色列表（如 ["admin", "user"]）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// 权限列表（如 ["user:read", "user:write"]）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
    /// Token 类型
    #[serde(default)]
    pub token_type: Option<TokenType>,
    /// 过期时间（Unix 时间戳，秒）
    pub exp: usize,
    /// 签发时间（Unix 时间戳，秒）
    #[serde(default)]
    pub iat: usize,
}

impl TokenClaims {
    /// 获取用户 ID
    pub fn user_id(&self) -> &str {
        &self.sub
    }

    /// 是否拥有指定角色
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// 是否拥有任一角色
    pub fn has_any_role(&self, roles: &[&str]) -> bool {
        roles.iter().any(|r| self.has_role(r))
    }

    /// 是否拥有所有角色
    pub fn has_all_roles(&self, roles: &[&str]) -> bool {
        roles.iter().all(|r| self.has_role(r))
    }

    /// 是否拥有指定权限
    ///
    /// 若 claims 中包含 `*` 或 `*:*:*` 通配权限，则匹配任意权限。
    pub fn has_permission(&self, permission: &str) -> bool {
        self.is_super_admin() || self.permissions.iter().any(|p| p == permission)
    }

    /// 是否拥有任一权限
    pub fn has_any_permission(&self, permissions: &[&str]) -> bool {
        self.is_super_admin() || permissions.iter().any(|p| self.has_permission(p))
    }

    /// 是否为超级管理员（拥有 `*` 或 `*:*:*` 通配权限）
    pub fn is_super_admin(&self) -> bool {
        self.permissions.iter().any(|p| p == "*" || p == "*:*:*")
    }
}

/// 已认证用户的 ID（从 JWT Token 中提取）
#[derive(Clone, Debug)]
pub struct UserId(pub String);

/// JWT 认证声明（挂载在 `request.extensions` 中）
#[derive(Clone, Debug)]
pub struct AuthClaims(pub TokenClaims);

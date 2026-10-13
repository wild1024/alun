//! REST 端点（`#[alun::get/post/patch]`，由 `App::scan()` 折叠注册）。
//!
//! 路径参数使用 axum 0.8 的 `{id}` 语法。
//! 认证后 `AuthClaims` 注入到请求扩展；租户作用域为**项目侧约定**：
//! `sub` 形如 `u{user_id}:t{tenant_id}`（框架无 `#[alun::tenant_scope]`，缺口 G1/G2）。

pub mod invoices;
pub mod subscriptions;
pub mod tenants;
pub mod transfers;

use alun::{ApiError, TokenClaims};

/// 端点统一返回类型（避免与 `alun_db` 的 `Result<T>` 别名冲突）。
pub type ApiResult<T> = std::result::Result<T, ApiError>;

/// 从 `AuthClaims.sub` 解析 `(user_id, tenant_id)`。
///
/// 约定：`u{user}:t{tenant}`；解析失败返回 401。
pub fn identity_of(claims: &TokenClaims) -> Result<(i64, i64), ApiError> {
    let parts: Vec<&str> = claims.sub.split(':').collect();
    if parts.len() != 2 {
        return Err(ApiError::unauthorized("无效的身份声明（sub）"));
    }
    let user_id = parts[0]
        .trim_start_matches('u')
        .parse::<i64>()
        .map_err(|_| ApiError::unauthorized("无效的用户 id"))?;
    let tenant_id = parts[1]
        .trim_start_matches('t')
        .parse::<i64>()
        .map_err(|_| ApiError::unauthorized("无效的租户 id"))?;
    Ok((user_id, tenant_id))
}

/// 金额解析（请求体用字符串承载，避免浮点）。
pub fn parse_money(s: &str) -> Result<rust_decimal::Decimal, ApiError> {
    use std::str::FromStr;
    rust_decimal::Decimal::from_str(s.trim())
        .map_err(|e| ApiError::bad_request(format!("金额格式错误 `{s}`: {e}")))
}

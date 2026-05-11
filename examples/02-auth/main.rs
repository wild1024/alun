//! 示例 02：认证（登录 → 刷新 → 登出）
//!
//! 演示 Token 生命周期管理，使用框架内置的 JWT 管理模块

use alun::{App, Res, ApiError, ValidatedJson, JWT};
use alun::web::AuthClaims;
use axum::Extension;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[allow(dead_code)]
struct LoginReq { username: String, password: String }

#[derive(Serialize)]
struct LoginRes { access_token: String, refresh_token: String }

#[derive(Deserialize)]
struct RefreshReq { refresh_token: String }

#[derive(Serialize)]
struct RefreshRes { access_token: String, refresh_token: String }

#[alun::post("/api/auth/login")]
async fn login(ValidatedJson(_req): ValidatedJson<LoginReq>) -> std::result::Result<Res<LoginRes>, ApiError> {
    let jwt = JWT::from_config();

    // 实际项目中应验证用户名密码，此处为演示
    let access_token = jwt.create_access_token("user_1", Some("alice"), &["user".into()], &["user:read".into()])
        .map_err(|e| ApiError::bad_request(e))?;
    let refresh_token = jwt.create_refresh_token("user_1")
        .map_err(|e| ApiError::internal(e))?;

    Ok(Res::ok(LoginRes { access_token, refresh_token }))
}

#[alun::post("/api/auth/refresh")]
async fn refresh(ValidatedJson(req): ValidatedJson<RefreshReq>) -> std::result::Result<Res<RefreshRes>, ApiError> {
    let jwt = JWT::from_config();

    let (access_token, refresh_token) = jwt.refresh(&req.refresh_token).await
        .map_err(|e| ApiError::unauthorized(e))?;

    Ok(Res::ok(RefreshRes { access_token, refresh_token }))
}

#[alun::post("/api/auth/logout")]
async fn logout(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> std::result::Result<Res<()>, ApiError> {
    let jwt = JWT::from_config();
    jwt.logout(&claims).await;
    Ok(Res::ok(()))
}

#[alun::get("/api/auth/me")]
async fn me(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<serde_json::Value> {
    Res::ok(serde_json::json!({
        "user_id": claims.sub,
        "username": claims.username,
        "roles": claims.roles,
        "permissions": claims.permissions,
        "is_super_admin": claims.is_super_admin(),
        "iat": claims.iat,
    }))
}

#[tokio::main]
async fn main() {
    App::new().expect("初始化失败").scan().start().await.unwrap();
}

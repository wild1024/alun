//! `AuthClaims` / `Principal` 作为**提取器**的双通道行为验证。
//!
//! 两条通道：
//!
//! | 通道 | 触发条件 | 用途 |
//! |---|---|---|
//! | 1 | 请求扩展中已有 `AuthClaims`（`AuthLayer` 已执行） | 链内路由，零验签开销 |
//! | 2 | 扩展中没有（`#[alun::no_auth]` 路由、认证层关闭、层位置在最外层） | 自验签 + 黑名单校验 |
//!
//! 通道 2 依赖全局配置（`[middleware.auth].jwt_secret`），因此本文件会安装一次全局配置。
//! 该安装是进程级的（`OnceLock`），故本文件所有用例共用同一份配置。

use alun::prelude::*;
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use std::sync::Arc;
use tower::ServiceExt;

const SECRET: &str = "extractor-channel-secret";
/// UUID 主键形态的 `sub`。
const USER: &str = "22222222-2222-2222-2222-222222222222";
/// 非 UUID 主键形态的 `sub`（`alun-ledger` 式复合作用域键）。
const SCOPED_USER: &str = "u7:t1";

fn cfg_with_auth() -> alun::AppConfig {
    let mut cfg = alun::AppConfig::default();
    cfg.middleware.auth.enabled = true;
    cfg.middleware.auth.jwt_secret = SECRET.into();
    cfg.static_files.enabled = false;
    cfg.router.not_found.enabled = true;
    cfg
}

/// 安装全局配置并返回同一份 `AppConfig`。
///
/// `set_config` 是进程级 `OnceLock`：同一二进制内只有第一次成功，其余返回 `Err`
/// ——本文件所有用例的配置完全相同，故忽略返回值是安全的（并行执行也不会分歧）。
fn install_global_config() -> alun::AppConfig {
    let cfg = cfg_with_auth();
    let cm = Arc::new(alun_config::ConfigManager {
        static_config: cfg.clone(),
        dynamic: parking_lot::RwLock::new(Default::default()),
    });
    let _ = alun::set_config(cm);
    cfg
}

// ── 路由 ──

/// 受保护路由：走**通道 1**（`AuthLayer` 已注入）。
#[alun::get("/ex/me")]
async fn me(claims: AuthClaims) -> Res<String> {
    Res::ok(claims.0.sub)
}

/// 结构化身份视图：`user_id` 是原样 `sub`，需要强类型时在调用点解析。
#[alun::get("/ex/principal")]
async fn who(p: Principal) -> Res<String> {
    let uuid = p
        .user_id
        .parse::<uuid::Uuid>()
        .map(|u| u.to_string())
        .unwrap_or_else(|| "-".to_string());
    Res::ok(format!(
        "{}|{}|{}",
        p.user_id.as_str(),
        uuid,
        p.username.unwrap_or_default()
    ))
}

/// 免认证路由 + 必填身份：只能走**通道 2**。
///
/// 改造前这里的行为是：`Extension<AuthClaims>` 提取失败 → **500**。
#[alun::no_auth("/ex/open")]
#[alun::get("/ex/open")]
async fn open(claims: AuthClaims) -> Res<String> {
    Res::ok(claims.0.sub)
}

/// 免认证路由 + 可选身份。
#[alun::no_auth("/ex/open-optional")]
#[alun::get("/ex/open-optional")]
async fn open_optional(auth: Option<AuthClaims>) -> Res<String> {
    Res::ok(auth.map(|c| c.0.sub).unwrap_or_else(|| "guest".to_string()))
}

/// 免认证路由 + 可选结构化身份。
#[alun::no_auth("/ex/open-principal")]
#[alun::get("/ex/open-principal")]
async fn open_principal(who: Option<Principal>) -> Res<String> {
    Res::ok(match who {
        Some(p) => p.user_id.as_str().to_string(),
        None => "guest".to_string(),
    })
}

// ── 工具 ──

fn token() -> String {
    token_for(USER)
}

fn token_for(sub: &str) -> String {
    use jsonwebtoken::{EncodingKey, Header, encode};

    let claims = TokenClaims {
        jti: Some("jti-extractor".into()),
        sub: sub.into(),
        username: Some("alice".into()),
        roles: vec!["auditor".into()],
        permissions: vec![],
        token_type: Some(TokenType::Access),
        exp: 9_999_999_999,
        iat: 0,
        extra: Default::default(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .expect("签发 Token")
}

async fn get(app: Router, uri: &str, token: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = builder.body(Body::empty()).expect("构造请求");
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("读取响应体");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn app_with(cfg: alun::AppConfig) -> Router {
    alun::App::with_config(cfg)
        .expect("App::with_config")
        .scan()
        .into_router()
}

/// 从统一响应体 `{code,msg,data}` 中取出 `data`。
fn data_of(body: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(body).expect("响应体应为 JSON");
    match v.get("data") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

// ── 通道 1 ──

/// 链内路由：`AuthLayer` 已注入，提取器走通道 1。
#[tokio::test]
async fn channel1_works_on_protected_route() {
    let cfg = install_global_config();
    let (status, body) = get(app_with(cfg), "/ex/me", Some(&token())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(data_of(&body), USER, "{body}");
}

/// 无凭证时仍然 401（提取器不改变认证层的放行决策）。
#[tokio::test]
async fn protected_route_without_token_is_401() {
    let cfg = install_global_config();
    let (status, body) = get(app_with(cfg), "/ex/me", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

// ── 通道 2 ──

/// 免认证路由 + 有效凭证：走通道 2 自行验签，拿得到身份。
#[tokio::test]
async fn channel2_resolves_identity_on_no_auth_route() {
    let cfg = install_global_config();
    let (status, body) = get(app_with(cfg), "/ex/open", Some(&token())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(data_of(&body), USER, "通道 2 应能自行验签得到身份: {body}");
}

/// **回归测试**：免认证路由要求身份但未提供凭证 → **401**。
///
/// 改造前 `Extension<AuthClaims>` 在此处提取失败会直接 **500**。
#[tokio::test]
async fn channel2_without_token_is_401_not_500() {
    let cfg = install_global_config();
    let (status, body) = get(app_with(cfg), "/ex/open", None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "缺少凭证必须 401（不得因提取失败而 500）: {body}"
    );
}

/// 免认证路由 + 无效凭证 → 401（而非 500，也不是静默放行）。
#[tokio::test]
async fn channel2_with_bad_token_is_401() {
    let cfg = install_global_config();
    let (status, body) = get(app_with(cfg), "/ex/open", Some("not-a-jwt")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
}

// ── 可选身份 ──

/// `Option<AuthClaims>`：无凭证 → `None`（不报错）。
#[tokio::test]
async fn optional_auth_is_none_without_credentials() {
    let cfg = install_global_config();
    let (status, body) = get(app_with(cfg), "/ex/open-optional", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(data_of(&body), "guest", "{body}");
}

/// `Option<AuthClaims>`：有凭证 → 拿到身份。
#[tokio::test]
async fn optional_auth_resolves_with_credentials() {
    let cfg = install_global_config();
    let (status, body) = get(app_with(cfg), "/ex/open-optional", Some(&token())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(data_of(&body), USER, "{body}");
}

// ── Principal ──

/// `Principal` 带出原样 `sub` + 解析后的 `Uuid` + 用户名。
#[tokio::test]
async fn principal_maps_claims_to_structured_view() {
    let cfg = install_global_config();
    let (status, body) = get(app_with(cfg), "/ex/principal", Some(&token())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(data_of(&body), format!("{USER}|{USER}|alice"), "{body}");
}

/// **回归测试**：非 UUID 的 `sub` 也能拿到身份（旧实现下 `user_id` 为 `None`）。
///
/// 这是 `Principal` 不再把 `user_id` 定成 `Option<Uuid>` 的原因：复合键 / 整型 /
/// IdP 主体同样必须可用。
#[tokio::test]
async fn principal_keeps_non_uuid_subject() {
    let cfg = install_global_config();
    let (status, body) = get(
        app_with(cfg),
        "/ex/principal",
        Some(&token_for(SCOPED_USER)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // 原样 `sub` 可用；UUID 解析失败只是 `-` —— 身份不丢、也不报错
    assert_eq!(data_of(&body), format!("{SCOPED_USER}|-|alice"), "{body}");
}

/// `Option<Principal>`：无凭证 → `None`。
#[tokio::test]
async fn optional_principal_is_none_without_credentials() {
    let cfg = install_global_config();
    let (status, body) = get(app_with(cfg), "/ex/open-principal", None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(data_of(&body), "guest", "{body}");
}

/// `Option<Principal>`：有凭证 → 结构化身份。
#[tokio::test]
async fn optional_principal_resolves_with_credentials() {
    let cfg = install_global_config();
    let (status, body) = get(app_with(cfg), "/ex/open-principal", Some(&token())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(data_of(&body), USER, "{body}");
}

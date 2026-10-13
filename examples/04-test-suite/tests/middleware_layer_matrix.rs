//! 中间件**粒度矩阵**与**层位置**的行为验证。
//!
//! 对应三个入口：
//!
//! | 入口 | 粒度 | 位置 | 能力 |
//! |---|---|---|---|
//! | [`App::layer`] | 整站（含 404 兜底） | 内置链**之外** | 看得到全部拒绝与耗时，**读不到身份** |
//! | [`App::route_layer`] | 仅匹配到的路由 | 内置链**之内** | **读得到身份**，看不到内置层的拒绝 |
//! | [`AlunRouter::merge_with_layer`] | 某个前缀下的子树 | 内置链之内 | 同 `route_layer`，但范围受限 |
//! | `#[alun::layer]` | 单条路由的方法处理器 | 内置链之内（方法级） | 同 `route_layer` |
//!
//! 判断「层看到了什么」的方式：探针层把结果写进响应头 `x-probe`。
//! - `id:<sub>` —— 本层执行时 `AuthClaims` 已在请求扩展中（层在链内）
//! - `id:none` —— 本层执行时读不到身份（层在链外，`AuthLayer` 尚未运行）
//! - 无该响应头 —— 本层根本没有执行

use alun::prelude::*;
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{Next, from_fn};
use axum::response::Response;
use tower::ServiceExt;

const SECRET: &str = "middleware-matrix-secret";
const USER: &str = "11111111-1111-1111-1111-111111111111";

// ── 探针层 ──

/// 把「本层看到什么身份」写进响应头 `x-probe`。
///
/// 注意：读身份必须在 `next.run(req)` **之前**（`req` 会被消费）。
pub async fn probe_layer(req: Request, next: Next) -> Response {
    let seen = match req.extensions().get::<AuthClaims>() {
        Some(claims) => format!("id:{}", claims.0.sub),
        None => "id:none".to_string(),
    };
    let mut resp = next.run(req).await;
    resp.headers_mut()
        .insert("x-probe", seen.parse().expect("响应头取值"));
    resp
}

// ── 路由（scan 通道 + 宏属性） ──

/// 普通路由：不做任何事，只用于确认请求真的到达处理器。
#[alun::get("/mware/plain")]
async fn plain() -> Res<String> {
    Res::ok("plain".into())
}

/// `#[alun::layer]` 走 `#[alun::get]` + `scan()` 通道。
///
/// 处理器参数直接取 `AuthClaims` —— 若层不在链内，这一步就取不到身份。
#[alun::layer(axum::middleware::from_fn(probe_layer))]
#[alun::get("/mware/macro-scan")]
async fn macro_scan(claims: AuthClaims) -> Res<String> {
    Res::ok(claims.0.sub)
}

/// `#[alun::layer]` + `#[alun::permission]` 叠加：验证执行顺序为
/// 「权限检查 → 自定义层 → 处理器」。
///
/// 无权限时应由 `PermissionCheckLayer` 直接 403，自定义层**根本不执行**
/// （响应里没有 `x-probe`）。
#[alun::layer(axum::middleware::from_fn(probe_layer))]
#[alun::permission(path = "/mware/guarded", method = "GET", permission = "mware:read")]
#[alun::get("/mware/guarded")]
async fn guarded() -> Res<String> {
    Res::ok("guarded".into())
}

// ── 路由（宏属性 + 模块通道） ──

#[alun::routes]
mod layered_mod {
    use alun::prelude::*;

    /// 模块内（`#[alun::routes]` 通道）使用 `#[alun::layer]`。
    #[alun::layer(axum::middleware::from_fn(super::probe_layer))]
    #[alun::get("/mware/macro-mod")]
    pub async fn mod_route(claims: AuthClaims) -> Res<String> {
        Res::ok(claims.0.sub)
    }

    /// 同一模块内的**无层**路由：用于证明层只作用于标注的那一条路由。
    #[alun::get("/mware/macro-plain")]
    pub async fn mod_plain() -> Res<String> {
        Res::ok("plain".into())
    }
}

// ── 工具 ──

fn cfg_with_auth() -> alun::AppConfig {
    let mut cfg = alun::AppConfig::default();
    cfg.middleware.auth.enabled = true;
    cfg.middleware.auth.jwt_secret = SECRET.into();
    cfg.static_files.enabled = false;
    cfg.router.not_found.enabled = true;
    cfg
}

/// 签一个合法 Access Token（`permissions` 决定权限校验是否放行）。
fn token(permissions: &[&str]) -> String {
    use jsonwebtoken::{EncodingKey, Header, encode};

    let claims = TokenClaims {
        jti: Some("jti-matrix".into()),
        sub: USER.into(),
        username: Some("alice".into()),
        roles: vec!["auditor".into()],
        permissions: permissions.iter().map(|p| p.to_string()).collect(),
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

/// 发一次请求，返回 (状态码, 响应体, `x-probe` 响应头)
async fn get(app: Router, uri: &str, token: Option<&str>) -> (StatusCode, String, Option<String>) {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = builder.body(Body::empty()).expect("构造请求");
    let resp = app.oneshot(req).await.expect("oneshot");

    let status = resp.status();
    let probe = resp
        .headers()
        .get("x-probe")
        .map(|v| v.to_str().unwrap_or_default().to_string());
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("读取响应体");
    (status, String::from_utf8_lossy(&bytes).to_string(), probe)
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

// ── 1. 层位置：链内 vs 链外 ──

/// **核心断言**：同一个探针层，挂在 `route_layer`（链内）能看到身份，
/// 挂在 `layer`（链外）看不到 —— 这就是「中间件读不到身份」的根因。
#[tokio::test]
async fn route_layer_sees_identity_while_outer_layer_does_not() {
    let tok = token(&[]);

    let inner = alun::App::with_config(cfg_with_auth())
        .expect("App::with_config")
        .scan()
        .route_layer(from_fn(probe_layer))
        .into_router();
    let (status, body, probe) = get(inner, "/mware/plain", Some(&tok)).await;
    assert_eq!(status, StatusCode::OK, "链内层不应影响放行: {body}");
    assert_eq!(
        probe.as_deref(),
        Some(format!("id:{USER}").as_str()),
        "route_layer 位于链内，必须能读到身份"
    );

    let outer = alun::App::with_config(cfg_with_auth())
        .expect("App::with_config")
        .scan()
        .layer(from_fn(probe_layer))
        .into_router();
    let (status, body, probe) = get(outer, "/mware/plain", Some(&tok)).await;
    assert_eq!(status, StatusCode::OK, "链外层不应影响放行: {body}");
    assert_eq!(
        probe.as_deref(),
        Some("id:none"),
        "layer 位于链外，此时 AuthLayer 尚未运行，读不到身份"
    );
}

/// 覆盖范围：`layer` 覆盖 404 兜底，`route_layer` 不覆盖（与 axum
/// `Router::layer` / `Router::route_layer` 语义一致）。
#[tokio::test]
async fn outer_layer_covers_404_but_route_layer_does_not() {
    let outer = alun::App::with_config(cfg_with_auth())
        .expect("App::with_config")
        .scan()
        .layer(from_fn(probe_layer))
        .into_router();
    let (status, _, probe) = get(outer, "/mware/definitely-missing", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        probe.is_some(),
        "App::layer 覆盖 fallback，404 也应经过该层"
    );

    let inner = alun::App::with_config(cfg_with_auth())
        .expect("App::with_config")
        .scan()
        .route_layer(from_fn(probe_layer))
        .into_router();
    let (status, _, probe) = get(inner, "/mware/definitely-missing", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        probe.is_none(),
        "App::route_layer 只作用于匹配到的路由，不应经过 404"
    );
}

// ── 2. 子树级 ──

/// `merge_with_layer` 的层只作用于该前缀下的路由。
#[tokio::test]
async fn merge_with_layer_scopes_layer_to_prefix() {
    fn routes(r: &mut AlunRouter) {
        let mut admin = AlunRouter::new();
        admin.add_get("/ping", || async { Res::ok("admin".to_string()) });
        r.merge_with_layer("/admin", admin, |sub| sub.layer(from_fn(probe_layer)));

        let mut public = AlunRouter::new();
        public.add_get("/ping", || async { Res::ok("public".to_string()) });
        r.merge("/public", public);
    }

    let app = alun::App::with_config(cfg_with_auth())
        .expect("App::with_config")
        .routes(routes)
        .into_router();
    let tok = token(&[]);

    let (status, body, probe) = get(app.clone(), "/admin/ping", Some(&tok)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        probe.as_deref(),
        Some(format!("id:{USER}").as_str()),
        "子树层应作用于 /admin/*，且能读到身份"
    );

    let (status, body, probe) = get(app, "/public/ping", Some(&tok)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(probe.is_none(), "其它前缀不应受该层影响");
}

// ── 3. 方法级宏属性 ──

/// `#[alun::layer]` 在 `#[alun::get]` + `scan()` 通道下生效，且位于链内。
#[tokio::test]
async fn macro_layer_works_on_scan_channel() {
    let app = alun::App::with_config(cfg_with_auth())
        .expect("App::with_config")
        .scan()
        .into_router();

    let (status, body, probe) = get(app.clone(), "/mware/macro-scan", Some(&token(&[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(data_of(&body), USER, "处理器应能直接提取到身份: {body}");
    assert_eq!(probe.as_deref(), Some(format!("id:{USER}").as_str()));

    // 层只作用于标注的那条路由
    let (status, _, probe) = get(app, "/mware/plain", Some(&token(&[]))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(probe.is_none(), "未标注 `#[alun::layer]` 的路由不应穿该层");
}

/// `#[alun::layer]` 在 `#[alun::routes]` 模块通道下同样生效。
#[tokio::test]
async fn macro_layer_works_on_module_channel() {
    let app = alun::App::with_config(cfg_with_auth())
        .expect("App::with_config")
        .module(layered_mod::register)
        .into_router();

    let (status, body, probe) = get(app.clone(), "/mware/macro-mod", Some(&token(&[]))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(data_of(&body), USER, "{body}");
    assert_eq!(probe.as_deref(), Some(format!("id:{USER}").as_str()));

    let (status, _, probe) = get(app, "/mware/macro-plain", Some(&token(&[]))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(probe.is_none(), "同模块内未标注的路由不应穿该层");
}

/// `#[alun::layer]` 与 `#[alun::permission]` 叠加：顺序为
/// 「权限检查 → 自定义层 → 处理器」。
#[tokio::test]
async fn permission_check_runs_before_route_level_layer() {
    let mut cfg = cfg_with_auth();
    cfg.middleware.permission.enabled = true;

    let app = alun::App::with_config(cfg)
        .expect("App::with_config")
        .scan()
        .into_router();

    // 有权限 → 放行，且自定义层执行
    let (status, body, probe) =
        get(app.clone(), "/mware/guarded", Some(&token(&["mware:read"]))).await;
    assert_eq!(status, StatusCode::OK, "持权限应放行: {body}");
    assert_eq!(probe.as_deref(), Some(format!("id:{USER}").as_str()));

    // 无权限 → 403，且自定义层**没有执行**（证明权限检查在更外层）
    let (status, body, probe) = get(app, "/mware/guarded", Some(&token(&[]))).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "缺权限必须 403: {body}");
    assert!(
        probe.is_none(),
        "权限检查必须先于方法级层执行，403 时该层不应运行"
    );
}

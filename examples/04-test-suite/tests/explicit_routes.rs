//! 证明 `#[alun::routes]` 显式注册**不依赖编译期收集机制**。
//!
//! # 背景
//!
//! 早前的 `#[alun::get]` + `App::scan()` 依赖 `linkme::distributed_slice` 在链接期
//! 收集路由。该机制在部分工具链上会**静默失效**——切片长度为 0，路由全部丢失，
//! 编译与链接均不报错，表现为所有请求 404（实测：windows-gnu + rustc 1.101-nightly，
//! `ROUTES=0` 而 crate 内有 24 个路由宏）。现已迁移到 `inventory`（见 `scan_routes.rs`）。
//!
//! 本文件全部走显式注册路径（`#[alun::routes]` / `App::routes`），**不触碰任何
//! 编译期收集机制**，因此在任何工具链上都应当通过。若本组测试通过而依赖 `scan()`
//! 的用例失败，即可判定为收集机制问题（含成因 B：rlib 目标文件未被抽取），
//! 而非路由逻辑问题。

use alun::prelude::*;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

/// 模块级显式注册：模块内的路由注解在宏展开期被扫描并剥离。
#[alun::routes]
mod probe {
    use alun::prelude::*;

    /// GET 探针
    #[alun::get("/api/explicit/hello")]
    pub async fn hello() -> Res<String> {
        Res::ok("explicit-ok".into())
    }

    /// POST 探针
    #[alun::post("/api/explicit/echo")]
    pub async fn echo() -> Res<String> {
        Res::ok("posted".into())
    }

    /// 带权限注解的路由 —— 验证 `#[alun::permission]` 被正确剥离并收集为规则
    #[alun::permission(
        path = "/api/explicit/guarded",
        method = "GET",
        permission = "probe:read"
    )]
    #[alun::get("/api/explicit/guarded")]
    pub async fn guarded() -> Res<String> {
        Res::ok("guarded".into())
    }

    /// 免认证路径 —— 验证 `#[alun::no_auth]` 被正确剥离并收集
    #[alun::no_auth("/api/explicit/open")]
    #[alun::get("/api/explicit/open")]
    pub async fn open() -> Res<String> {
        Res::ok("open".into())
    }
}

/// 最小配置：认证与权限开关保持默认（关闭），便于观察路由本身是否可达。
fn base_cfg() -> alun::AppConfig {
    alun::AppConfig::default()
}

/// 对 Router 发一次请求，返回 (状态码, 响应体)
async fn call(app: Router, method: &str, uri: &str) -> (StatusCode, String) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("构造请求");
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("读取响应体");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

/// 显式注册的 GET 路由可达。
///
/// 若这条路由依赖编译期收集，本断言会因路由表为空而得到 404。
#[tokio::test]
async fn explicit_get_route_is_reachable() {
    let app = alun::App::with_config(base_cfg())
        .expect("App::with_config")
        .module(probe::register);

    let (status, body) = call(app.into_router(), "GET", "/api/explicit/hello").await;
    assert_eq!(status, StatusCode::OK, "显式注册的 GET 路由应可达: {body}");
    assert!(body.contains("explicit-ok"), "响应体不符: {body}");
}

/// 显式注册的 POST 路由可达（验证动词被正确写入路由表）
#[tokio::test]
async fn explicit_post_route_is_reachable() {
    let app = alun::App::with_config(base_cfg())
        .expect("App::with_config")
        .module(probe::register);

    let (status, body) = call(app.into_router(), "POST", "/api/explicit/echo").await;
    assert_eq!(status, StatusCode::OK, "显式注册的 POST 路由应可达: {body}");
    assert!(body.contains("posted"), "响应体不符: {body}");
}

/// 动词区分正确：GET 路径不接受 POST
#[tokio::test]
async fn explicit_route_rejects_wrong_method() {
    let app = alun::App::with_config(base_cfg())
        .expect("App::with_config")
        .module(probe::register);

    let (status, _) = call(app.into_router(), "POST", "/api/explicit/hello").await;
    assert_eq!(
        status,
        StatusCode::METHOD_NOT_ALLOWED,
        "GET 路由不应接受 POST"
    );
}

/// `App::routes`（fn 指针通道）同样不依赖编译期收集 —— 最底层的逃生通道
#[tokio::test]
async fn app_routes_fn_pointer_channel_works() {
    fn manual_routes(r: &mut AlunRouter) {
        r.add_get("/api/manual-ping", || async {
            Res::ok("pong".to_string())
        });
    }

    let app = alun::App::with_config(base_cfg())
        .expect("App::with_config")
        .routes(manual_routes);

    let (status, body) = call(app.into_router(), "GET", "/api/manual-ping").await;
    assert_eq!(status, StatusCode::OK, "App::routes 注册的路由应可达: {body}");
    assert!(body.contains("pong"), "响应体不符: {body}");
}

/// 两条通道可混用：显式注册 + `App::get` 链式，互不干扰
#[tokio::test]
async fn explicit_registration_mixes_with_builder() {
    let app = alun::App::with_config(base_cfg())
        .expect("App::with_config")
        .get("/api/builder-added", || async {
            Res::ok("builder".to_string())
        })
        .module(probe::register);

    let app = app.into_router();
    let (s1, b1) = call(app.clone(), "GET", "/api/builder-added").await;
    let (s2, b2) = call(app, "GET", "/api/explicit/hello").await;
    assert_eq!(s1, StatusCode::OK, "构建器注册的路由应可达: {b1}");
    assert_eq!(s2, StatusCode::OK, "模块注册的路由应可达: {b2}");
}

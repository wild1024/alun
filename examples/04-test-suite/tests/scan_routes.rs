//! 回归守卫：证明 `#[alun::get]` + `App::scan()` 的编译期收集**确实生效**
//!
//! # 为什么需要这个文件
//!
//! `examples/04-test-suite/tests/explicit_routes.rs` 全部走显式注册
//! （`#[alun::routes]` / `App::routes`），**不触碰编译期收集**；而
//! `examples/01-basic` 虽然用了 `scan()`，却没有断言。于是存在一处盲区：
//! 一旦收集机制失效（切片/注册表为空），**没有任何测试会失败**，
//! 只会在运行时表现为全部 404。
//!
//! 本文件正是补上这个盲区：用 `scan()` 装配，并**断言路由真的可达**。
//!
//! # 历史故障（F19）
//!
//! 早前用 `linkme::distributed_slice`，依赖链接器保留自定义段；在
//! `x86_64-pc-windows-gnu` 的 `--gc-sections` 下会被**整段回收**，
//! `ROUTES` 长度为 0 且**编译链接无报错**。现已迁移到 `inventory`
//! （不依赖链接器段），并删除了 `.cargo/config.toml` 里的
//! `-Wl,--no-gc-sections` 兜底——本文件即该结论的回归守卫：
//! **一旦有人改回依赖链接器段，这里会红。**

use alun::prelude::*;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

/// 宏注解的探针路由 —— 由 `inventory` 在编译期收集，`App::scan()` 装配。
#[alun::get("/api/scan/ping")]
pub async fn ping() -> Res<String> {
    Res::ok("pong".into())
}

/// 宏注解的 POST 探针。
#[alun::post("/api/scan/echo")]
pub async fn echo() -> Res<String> {
    Res::ok("posted".into())
}

/// `scan()` 必须把宏收集的 GET 路由装配进最终 Router。
#[tokio::test]
async fn scan_collects_macro_get_route() {
    let app = App::new().expect("App::new").scan();
    let router: Router = app.into_router();

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/api/scan/ping")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "`scan()` 未收集到 `#[alun::get]` 路由 —— 编译期收集机制失效"
    );
}

/// 收集到的路由数量必须与实际声明的一致（防止"部分丢失"这类飘忽故障）。
#[tokio::test]
async fn discovered_routes_is_not_empty() {
    let count = alun::discovered_routes().count();
    assert!(
        count >= 2,
        "编译期收集到的路由数应 >= 2（ping + echo），实际 {count} —— 收集机制失效"
    );
}

/// POST 路由同样走收集通道（方法过滤不能把非 GET 动词漏掉）。
#[tokio::test]
async fn scan_collects_macro_post_route() {
    let app = App::new().expect("App::new").scan();
    let router: Router = app.into_router();

    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/scan/echo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("oneshot");

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "`scan()` 未收集到 `#[alun::post]` 路由"
    );
}

/// 收集通道与显式通道**互不干扰**：`scan()` 之后仍可用 `App::get` 追加路由。
#[tokio::test]
async fn scan_and_manual_routes_coexist() {
    let app = App::new()
        .expect("App::new")
        .scan()
        .get("/api/scan/manual", || async { Res::ok("manual".to_string()) });
    let router: Router = app.into_router();

    for uri in ["/api/scan/ping", "/api/scan/manual"] {
        let r = router.clone();
        let resp = r
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK, "{uri} 应可达");
    }
}

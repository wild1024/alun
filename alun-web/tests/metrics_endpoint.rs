//! F10 · 端到端：`[metrics]` 配置 → `App` 装配 → `/metrics` 可抓取。
//!
//! 单测（`src/metrics.rs`）验证的是**组件**（层、端点函数、路径折叠）；
//! 这里验证的是**装配链路**：配置开了端点是否真的挂上、挂上后是否真的
//! 不穿鉴权链、埋点是否真的进入渲染文本。三者任一环断掉，运维侧的表现
//! 都是"指标没有数据"——而原因各不相同。
//!
//! 仅在 `metrics-prometheus` 下编译（端点与 recorder 都来自该 feature）。
#![cfg(feature = "metrics-prometheus")]

use alun_config::{AppConfig, MetricsConfig};
use alun_web::App;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::Request;
use tower::ServiceExt;

fn config_with_metrics(mc: MetricsConfig) -> AppConfig {
    let mut cfg = AppConfig::default();
    cfg.metrics = mc;
    cfg
}

/// 带回环 `ConnectInfo` 的请求（端点按来源 IP 判定，见 `[metrics].allow_ips`）。
fn req(uri: &str) -> Request<Body> {
    let mut r = Request::builder().uri(uri).body(Body::empty()).expect("request");
    let addr: std::net::SocketAddr = "127.0.0.1:54321".parse().expect("addr");
    r.extensions_mut().insert(ConnectInfo(addr));
    r
}

#[tokio::test]
async fn endpoint_is_mounted_and_scrapeable_when_enabled() {
    let cfg = config_with_metrics(MetricsConfig {
        enabled: true,
        ..Default::default()
    });

    // `into_router` 不安装 recorder（那是 `start()` 的职责），测试里手动装
    let _ = alun_web::metrics::install_prometheus(&cfg.metrics);

    let app = App::with_config(cfg)
        .expect("build app")
        .get("/hello", || async { "hi" })
        .into_router();

    // 业务请求 → 埋点
    let resp = app
        .clone()
        .oneshot(req("/hello"))
        .await
        .expect("oneshot /hello");
    assert_eq!(resp.status(), 200);

    // 抓取端点（无 Token）——端点不穿鉴权链，抓取器无需凭据
    let resp = app.oneshot(req("/metrics")).await.expect("oneshot /metrics");
    assert_eq!(resp.status(), 200, "已启用 + 回环来源 → 应可抓取");

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    let text = String::from_utf8(body.to_vec()).expect("utf8");
    assert!(
        text.contains("alun_http_requests_total"),
        "埋点应进入导出文本：{text}"
    );
    assert!(
        text.contains(r#"route="/hello""#),
        "业务路由应作为 route 标签出现：{text}"
    );
    assert!(
        !text.contains(r#"route="/metrics""#),
        "抓取端点自身不得计入请求量：{text}"
    );
    // 元数据：指标必须自带说明，否则运维拿到一堆裸序列无从下手
    assert!(
        text.contains("# HELP alun_http_requests_total"),
        "导出文本应带 # HELP 说明行：{text}"
    );
}

#[tokio::test]
async fn endpoint_is_absent_by_default() {
    // 默认 `enabled = false` —— 不对外暴露任何运行数据
    let cfg = config_with_metrics(MetricsConfig::default());
    let app = App::with_config(cfg)
        .expect("build app")
        .get("/hello", || async { "hi" })
        .into_router();

    let resp = app.oneshot(req("/metrics")).await.expect("oneshot");
    assert_eq!(
        resp.status(),
        404,
        "未启用时端点不应存在（默认私有，避免运行数据外泄）"
    );
}

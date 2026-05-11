//! IP 限流中间件（滑动窗口）

use axum::{
    extract::Request,
    response::Response,
    body::Body,
};
use axum::http::StatusCode;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tower::{Layer, Service};
use crate::response::{Res, codes};

/// 令牌桶限流中间件（基于 IP）
///
/// # 配置
///
/// 在 `config.toml` 的 `[middleware.rate_limit]` 中设置：
///
/// ```toml
/// [middleware.rate_limit]
/// enabled = true
/// requests_per_window = 100
/// window_secs = 60
/// ```
///
/// 超限返回 429 Too Many Requests。
#[derive(Clone)]
pub struct RateLimitLayer {
    /// 时间窗口内允许的最大请求数
    pub requests_per_window: u64,
    /// 时间窗口宽度（秒）
    pub window_secs: u64,
    /// IP → 计数器映射（共享状态）
    pub store: Arc<RwLock<HashMap<String, IpWindow>>>,
}

/// IP 滑动窗口计数器——记录时间窗口内的请求次数
pub struct IpWindow {
    /// 当前窗口内的请求计数
    pub count: u64,
    /// 窗口起始时刻
    pub window_start: Instant,
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitService {
            inner,
            requests_per_window: self.requests_per_window,
            window_secs: self.window_secs,
            store: self.store.clone(),
        }
    }
}

#[derive(Clone)]
/// 限流服务——滑动窗口算法实现 IP 级别请求限流
pub struct RateLimitService<S> {
    /// 下游服务
    inner: S,
    /// 时间窗口内允许的最大请求数
    requests_per_window: u64,
    /// 时间窗口宽度（秒）
    window_secs: u64,
    /// IP → 窗口计数器映射
    store: Arc<RwLock<HashMap<String, IpWindow>>>,
}

impl<S> Service<Request<Body>> for RateLimitService<S>
where
    S: Service<Request<Body>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let ip = req.headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .split(',')
            .next()
            .unwrap_or("unknown")
            .trim()
            .to_string();

        let max_req = self.requests_per_window;
        let win_secs = self.window_secs;
        let store = self.store.clone();
        let window_dur = std::time::Duration::from_secs(win_secs);

        let allowed = {
            let mut guard = store.write();
            let now = Instant::now();
            let window = guard.entry(ip).or_insert(IpWindow { count: 0, window_start: now });

            if now - window.window_start > window_dur {
                window.count = 0;
                window.window_start = now;
            }

            if window.count < max_req {
                window.count += 1;
                true
            } else {
                false
            }
        };

        if allowed {
            let mut inner = self.inner.clone();
            Box::pin(async move { inner.call(req).await })
        } else {
            let body = serde_json::to_string(&Res::<()>::fail(
                codes::TOO_MANY_REQUESTS, "请求过于频繁，请稍后再试"
            )).unwrap_or_else(|_| r#"{"code":429,"msg":"请求过于频繁，请稍后再试"}"#.to_string());
            let resp = Response::builder()
                .status(StatusCode::TOO_MANY_REQUESTS)
                .header("Content-Type", "application/json; charset=utf-8")
                .body(Body::from(body))
                .expect("response body build failed");
            Box::pin(async move { Ok(resp) })
        }
    }
}

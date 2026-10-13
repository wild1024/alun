//! IP 限流中间件（滑动窗口）

use crate::response::{Res, codes};
use axum::http::StatusCode;
use axum::{body::Body, extract::Request, response::Response};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tower::{Layer, Service};

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
        let ip = extract_trusted_client_ip(&req);

        let max_req = self.requests_per_window;
        let win_secs = self.window_secs;
        let store = self.store.clone();
        let window_dur = std::time::Duration::from_secs(win_secs);

        let allowed = {
            let mut guard = store.write();
            let now = Instant::now();
            let window = guard.entry(ip).or_insert(IpWindow {
                count: 0,
                window_start: now,
            });

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
                codes::TOO_MANY_REQUESTS,
                "请求过于频繁，请稍后再试",
            ))
            .unwrap_or_else(|_| r#"{"code":429,"msg":"请求过于频繁，请稍后再试"}"#.to_string());
            let resp = Response::builder()
                .status(StatusCode::TOO_MANY_REQUESTS)
                .header("Content-Type", "application/json; charset=utf-8")
                .body(Body::from(body))
                .expect("response body build failed");
            Box::pin(async move { Ok(resp) })
        }
    }
}

/// 提取可信的客户端 IP，作为限流键
///
/// **安全说明（S-01）**：`X-Forwarded-For` 的**首段**由客户端可控，若直接取首段作为限流键，
/// 攻击者轮换伪造值即可无限绕过限流。本实现遵循两条可信来源（与部署拓扑强相关，后端位于
/// nginx 之后）：
///
/// 1. `X-Real-IP`：由 nginx `proxy_set_header X-Real-IP $remote_addr` 覆写为真实客户端 IP（不可伪造）；
/// 2. `X-Forwarded-For` **末段**：nginx `$proxy_add_x_forwarded_for` 将真实客户端 IP 追加在末尾。
///
/// 无法从以上来源取得合法 IP 时回退为 `"unknown"`（所有此类请求共享同一限流桶，避免绕过）。
fn extract_trusted_client_ip(req: &Request<Body>) -> String {
    fn valid_ip(s: &str) -> bool {
        s.parse::<std::net::IpAddr>().is_ok()
    }

    // 1. 优先 X-Real-IP（nginx 覆写，最可信）
    if let Some(ip) = req
        .headers()
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim())
        .filter(|s| valid_ip(s))
    {
        return ip.to_string();
    }

    // 2. 回退 X-Forwarded-For 末段（nginx 追加的真实客户端 IP）
    if let Some(ip) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next_back())
        .map(|v| v.trim())
        .filter(|s| valid_ip(s))
    {
        return ip.to_string();
    }

    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_with_headers(entries: &[(&str, &str)]) -> Request<Body> {
        let mut builder = Request::builder();
        for (k, v) in entries {
            builder = builder.header(*k, *v);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn x_real_ip_takes_priority_over_forged_xff() {
        // X-Real-IP 为可信真实 IP，即使 XFF 首段被伪造也应作为限流键
        let req = request_with_headers(&[
            ("x-forwarded-for", "1.2.3.4, 203.0.113.9"),
            ("x-real-ip", "203.0.113.9"),
        ]);
        assert_eq!(extract_trusted_client_ip(&req), "203.0.113.9");
    }

    #[test]
    fn xff_uses_last_segment_not_attacker_first_segment() {
        // S-01：修复前取 XFF 首段（攻击者可控），修复后取末段（nginx 追加）作为限流键
        let req = request_with_headers(&[("x-forwarded-for", "1.2.3.4, 198.51.100.7")]);
        assert_eq!(extract_trusted_client_ip(&req), "198.51.100.7");
    }

    #[test]
    fn rotation_of_forged_first_segment_keeps_same_key() {
        // 攻击者轮换伪造首段值，但真实末段不变 → 限流键恒定，无法绕过
        let forged_rotations = ["8.8.8.8", "9.9.9.9", "6.6.6.6"];
        let base = "198.51.100.7";
        for forged in forged_rotations {
            let req = request_with_headers(&[("x-forwarded-for", &format!("{forged}, {base}"))]);
            assert_eq!(extract_trusted_client_ip(&req), base);
        }
    }

    #[test]
    fn single_hop_xff_returns_itself() {
        let req = request_with_headers(&[("x-forwarded-for", "203.0.113.42")]);
        assert_eq!(extract_trusted_client_ip(&req), "203.0.113.42");
    }

    #[test]
    fn invalid_ip_falls_back_to_unknown() {
        // 头非法或缺失时回退 unknown，所有此类请求共享同一限流桶，不退让绕过面
        let req = request_with_headers(&[]);
        assert_eq!(extract_trusted_client_ip(&req), "unknown");
        let evil = request_with_headers(&[("x-forwarded-for", "evil, 12.34.56.78")]);
        assert_eq!(extract_trusted_client_ip(&evil), "12.34.56.78");
    }
}

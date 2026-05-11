//! 幂等键中间件（按需，建议在订单创建/扣款等关键写操作上使用）
//!
//! 客户端在请求头中携带 `x-idempotency-key`，
//! 服务端保证同一 key 的请求只执行一次，
//! 重复请求直接返回缓存的首次响应结果。
//!
//! # 使用方式
//!
//! ```ignore
//! use alun_web::middleware::IdempotencyLayer;
//!
//! // 在特定写操作路由上单独包裹
//! router.route("/api/order/create", post(create_order).layer(
//!     IdempotencyLayer::new(cache, Duration::from_secs(86400))
//! ));
//! ```

use axum::{
    extract::Request,
    response::Response,
    body::Body,
};
use axum::http::StatusCode;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tower::{Layer, Service};
use alun_cache::Cache;

/// 幂等键中间件
///
/// 首次请求执行实际业务并缓存完整响应，
/// 后续相同 key 的请求直接返回缓存的响应。
/// `ttl` 决定幂等键的有效期（建议 86400 秒 = 24 小时）。
#[derive(Clone)]
pub struct IdempotencyLayer {
    cache: Arc<alun_cache::SharedCache>,
    ttl: Duration,
}

impl IdempotencyLayer {
    /// 创建幂等键中间件
    ///
    /// - `cache`: 共享缓存（建议 Redis，确保多实例间共享）
    /// - `ttl`: 幂等键过期时间（建议 86400 秒）
    pub fn new(cache: Arc<alun_cache::SharedCache>, ttl: Duration) -> Self {
        Self { cache, ttl }
    }
}

impl<S> Layer<S> for IdempotencyLayer {
    type Service = IdempotencyService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        IdempotencyService {
            inner,
            cache: self.cache.clone(),
            ttl: self.ttl,
        }
    }
}

#[derive(Clone)]
/// 幂等键服务——缓存首次响应，重复请求直接返回缓存结果
pub struct IdempotencyService<S> {
    /// 下游服务
    inner: S,
    /// 缓存层引用
    cache: Arc<alun_cache::SharedCache>,
    /// 缓存过期时间
    ttl: Duration,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedResponse {
    status: u16,
    content_type: String,
    body: String,
}

impl<S> Service<Request<Body>> for IdempotencyService<S>
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
        let idem_key = req.headers()
            .get("x-idempotency-key")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let has_idem_key = idem_key.is_some();
        let cache = self.cache.clone();
        let ttl_secs = self.ttl.as_secs();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            if let Some(ref k) = idem_key {
                let cache_key = format!("idem:{}", k);
                if let Ok(Some(cached)) = cache.get::<CachedResponse>(&cache_key).await {
                    return Ok(Response::builder()
                        .status(StatusCode::from_u16(cached.status).unwrap_or(StatusCode::OK))
                        .header("Content-Type", &cached.content_type)
                        .body(Body::from(cached.body))
                        .expect("response body build failed"));
                }
            }

            let resp = inner.call(req).await?;

            if has_idem_key {
                let k = idem_key.expect("has_idem_key checked above");
                let status = resp.status().as_u16();
                let content_type = resp.headers()
                    .get("Content-Type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("application/json; charset=utf-8")
                    .to_string();

                let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await;
                let body_str = match body_bytes {
                    Ok(bytes) => String::from_utf8_lossy(&bytes).to_string(),
                    Err(_) => String::new(),
                };
                let cache_key = format!("idem:{}", k);
                let _ = cache.set_ex(
                    &cache_key,
                    &CachedResponse {
                        status,
                        content_type: content_type.clone(),
                        body: body_str.clone(),
                    },
                    ttl_secs,
                ).await;

                return Ok(Response::builder()
                    .status(StatusCode::from_u16(status).unwrap_or(StatusCode::OK))
                    .header("Content-Type", &content_type)
                    .body(Body::from(body_str))
                    .expect("response body build failed"));
            }

            Ok(resp)
        })
    }
}
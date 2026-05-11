//! Nonce 防重放中间件（按需，建议仅在写操作路由上使用）
//!
//! 客户端在请求头中携带 `x-nonce`（唯一随机值），
//! 服务端通过缓存（LocalCache/Redis）检查该 nonce 是否已使用，
//! 若已存在则返回 409 Conflict，拒绝重复请求。
//!
//! # 使用方式
//!
//! ```ignore
//! use alun_web::middleware::NonceLayer;
//!
//! // 在特定写操作路由上单独包裹
//! router.route("/api/transfer", post(transfer_handler).layer(
//!     NonceLayer::new(cache, Duration::from_secs(300))
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
use crate::response::{Res, codes};

/// Nonce 防重放中间件
///
/// 缓存层使用 `SharedCache`（LocalCache/Redis 的 enum 包装）。
/// `ttl` 决定 nonce 的有效去重窗口（过期后相同 nonce 可再次使用）。
#[derive(Clone)]
pub struct NonceLayer {
    cache: Arc<alun_cache::SharedCache>,
    ttl: Duration,
}

impl NonceLayer {
    /// 创建 Nonce 中间件
    ///
    /// - `cache`: 共享缓存（LocalCache 或 RedisCache），通过 `Arc<SharedCache>` 传递
    /// - `ttl`: nonce 过期时间（建议 300 秒）
    pub fn new(cache: Arc<alun_cache::SharedCache>, ttl: Duration) -> Self {
        Self { cache, ttl }
    }
}

impl<S> Layer<S> for NonceLayer {
    type Service = NonceService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        NonceService {
            inner,
            cache: self.cache.clone(),
            ttl: self.ttl,
        }
    }
}

#[derive(Clone)]
/// Nonce 防重放服务——检查请求头中的 x-nonce 是否已使用
pub struct NonceService<S> {
    /// 下游服务
    inner: S,
    /// 缓存层引用
    cache: Arc<alun_cache::SharedCache>,
    /// nonce 过期时间
    ttl: Duration,
}

impl<S> Service<Request<Body>> for NonceService<S>
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
        let nonce = req.headers()
            .get("x-nonce")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let cache = self.cache.clone();
        let ttl = self.ttl;
        let mut inner = self.inner.clone();

        Box::pin(async move {
            if let Some(n) = nonce {
                let key = format!("nonce:{}", n);
                let ttl_secs = ttl.as_secs();
                let placeholder = "1".to_string();

                let exists = cache
                    .get::<String>(&key)
                    .await
                    .unwrap_or(None);

                if exists.is_some() {
                    let body = serde_json::to_string(&Res::<()>::fail(
                        codes::CONFLICT, "请求已处理，请勿重复提交"
                    )).unwrap_or_else(|_| r#"{"code":409,"msg":"请求已处理，请勿重复提交"}"#.to_string());
                    return Ok(Response::builder()
                        .status(StatusCode::CONFLICT)
                        .header("Content-Type", "application/json; charset=utf-8")
                        .body(Body::from(body))
                        .expect("response body build failed"));
                }

                let _ = cache.set_ex(&key, &placeholder, ttl_secs).await;
            }

            inner.call(req).await
        })
    }
}
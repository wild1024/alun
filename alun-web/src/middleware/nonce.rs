//! 动态 Nonce 中间件 —— 通过动态路由提交/消费 Nonce 实现 CSRF 防护、重放防护。
//!
//! 安全防护原理：
//! 服务端生成随机 nonce 并限制有效期（默认 5 分钟），
//! 客户端必须在请求头中携带 `X-Nonce`，服务端校验后立即消费（一次有效）。
//!
//! # 配置
//!
//! ```toml
//! [middleware.nonce]
//! enabled = true
//! ttl_secs = 300
//! ```
//!
//! # 使用方式
//!
//! 1. 前端先调用 `GET /api/nonce` 获取 nonce
//! 2. 后续写操作请求中携带 `X-Nonce: <获取到的nonce>`
//! 3. 中间件校验 nonce 有效且未过期，校验通过后立即消费

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
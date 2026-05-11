//! 请求 ID 中间件
//!
//! 为每个请求生成或复用 request-id，注入到请求头 x-request-id 中。

use axum::{
    extract::Request,
    response::Response,
};
use axum::http::HeaderName;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use uuid::Uuid;

/// 请求 ID 中间件
///
/// 为每个请求分配一个 UUID v7（时间有序），写入 `x-request-id` 响应头和 tracing span，
/// 便于日志串联和全链路追踪。
#[derive(Clone)]
pub struct RequestIdLayer;

impl<S> Layer<S> for RequestIdLayer {
    type Service = RequestIdService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestIdService { inner }
    }
}

#[derive(Clone)]
/// 请求 ID 服务——为每个请求分配 UUID 并注入到响应头
pub struct RequestIdService<S> {
    /// 下游服务
    inner: S,
}

impl<S, ReqBody> Service<Request<ReqBody>> for RequestIdService<S>
where
    S: Service<Request<ReqBody>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<ReqBody>) -> Self::Future {
        let request_id = req.headers()
            .iter()
            .find(|(name, _)| name.as_str().to_ascii_lowercase() == "x-request-id")
            .and_then(|(_, v)| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string()[..16].to_string());

        if let Ok(header_val) = request_id.parse() {
            req.headers_mut().insert(
                HeaderName::from_static("x-request-id"),
                header_val,
            );
        }

        let mut inner = self.inner.clone();
        Box::pin(async move { inner.call(req).await })
    }
}

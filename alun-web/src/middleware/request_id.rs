//! 请求 ID 中间件
//!
//! 为每个请求生成或复用 request-id，同时注入到请求头与响应头的 `x-request-id`，
//! 便于客户端、网关与日志串联同一个请求。

use axum::http::{HeaderName, HeaderValue};
use axum::{extract::Request, response::Response};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use uuid::Uuid;

/// 请求 ID 头名（请求头与响应头同名）
const X_REQUEST_ID: &str = "x-request-id";

/// 请求 ID 中间件
///
/// 为每个请求分配一个 UUID v4（16 位十六进制前缀），
/// 复用客户端传入的 `x-request-id`（若有），
/// 并把同一个值写入请求头（下游可读）与响应头（调用方可读）。
#[derive(Clone)]
pub struct RequestIdLayer;

impl<S> Layer<S> for RequestIdLayer {
    type Service = RequestIdService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestIdService { inner }
    }
}

#[derive(Clone)]
/// 请求 ID 服务——为每个请求分配/复用 ID 并注入到请求头与响应头
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
        let request_id = req
            .headers()
            .iter()
            .find(|(name, _)| name.as_str().eq_ignore_ascii_case(X_REQUEST_ID))
            .and_then(|(_, v)| v.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string()[..16].to_string());

        let header_value = HeaderValue::from_str(&request_id).ok();
        if let Some(ref v) = header_value {
            req.headers_mut()
                .insert(HeaderName::from_static(X_REQUEST_ID), v.clone());
        }

        let mut inner = self.inner.clone();
        Box::pin(async move {
            let mut resp = inner.call(req).await?;
            // 回写响应头：与请求头为同一个 ID（客户端据此关联本次请求）
            if let Some(v) = header_value {
                resp.headers_mut()
                    .insert(HeaderName::from_static(X_REQUEST_ID), v);
            }
            Ok(resp)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt;

    /// 下游 handler 回显它读到的请求头 `x-request-id`
    async fn echo_request_id(req: Request<Body>) -> String {
        req.headers()
            .get(X_REQUEST_ID)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("<missing>")
            .to_string()
    }

    fn app() -> Router {
        Router::new()
            .route("/", get(echo_request_id))
            .layer(RequestIdLayer)
    }

    #[tokio::test]
    async fn response_header_echoes_client_request_id() {
        let req = Request::builder()
            .uri("/")
            .header(X_REQUEST_ID, "abc123")
            .body(Body::empty())
            .unwrap();
        let resp = app().oneshot(req).await.expect("oneshot");

        assert_eq!(
            resp.headers()
                .get(X_REQUEST_ID)
                .and_then(|v| v.to_str().ok()),
            Some("abc123"),
            "响应头必须回写请求 ID（D004）"
        );
    }

    #[tokio::test]
    async fn generated_id_is_written_to_request_and_response_header() {
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = app().oneshot(req).await.expect("oneshot");

        let response_id = resp
            .headers()
            .get(X_REQUEST_ID)
            .and_then(|v| v.to_str().ok())
            .expect("缺少响应头 x-request-id")
            .to_string();
        assert_eq!(response_id.len(), 16, "生成的请求 ID 为 16 位十六进制");

        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let request_id = String::from_utf8(body.to_vec()).unwrap();
        assert_eq!(
            request_id, response_id,
            "请求头与响应头必须是同一个请求 ID（保持请求头行为不变）"
        );
    }
}

//! 安全响应头中间件
//!
//! 默认注入以下安全头：
//! - X-Content-Type-Options: nosniff
//! - X-Frame-Options: DENY
//! - Strict-Transport-Security: max-age=31536000; includeSubDomains
//! - Content-Security-Policy: default-src 'self'
//! - Referrer-Policy: strict-origin-when-cross-origin
//!
//! 通过 [SecurityHeadersConfig] 按需开关各个 header，
//! 或设置 `enabled = false` 关闭整个中间件。

use axum::{
    extract::Request,
    response::Response,
    body::Body,
};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use alun_config::SecurityHeadersConfig;

/// 安全头中间件 —— 注入 HTTP 安全响应头
#[derive(Clone)]
pub struct SecurityHeadersLayer {
    config: SecurityHeadersConfig,
}

impl SecurityHeadersLayer {
    /// 从配置创建安全头中间件
    pub fn new(config: SecurityHeadersConfig) -> Self {
        Self { config }
    }
}

impl<S> Layer<S> for SecurityHeadersLayer {
    type Service = SecurityHeadersService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SecurityHeadersService {
            inner,
            config: self.config.clone(),
        }
    }
}

#[derive(Clone)]
/// 安全头服务——注入 HTTP 安全响应头
pub struct SecurityHeadersService<S> {
    /// 下游服务
    inner: S,
    /// 安全头配置
    config: SecurityHeadersConfig,
}

impl<S> Service<Request<Body>> for SecurityHeadersService<S>
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
        let mut inner = self.inner.clone();
        let config = self.config.clone();
        Box::pin(async move {
            let mut resp = inner.call(req).await?;
            let headers = resp.headers_mut();

            if config.nosniff {
                headers.insert(
                    axum::http::HeaderName::from_static("x-content-type-options"),
                    axum::http::HeaderValue::from_static("nosniff"),
                );
            }

            if config.frame_options {
                headers.insert(
                    axum::http::HeaderName::from_static("x-frame-options"),
                    axum::http::HeaderValue::from_static("DENY"),
                );
            }

            if config.hsts {
                let mut hsts_val = format!("max-age={}", config.hsts_max_age_secs);
                if config.hsts_include_subdomains {
                    hsts_val.push_str("; includeSubDomains");
                }
                if let Ok(val) = axum::http::HeaderValue::from_str(&hsts_val) {
                    headers.insert(
                        axum::http::HeaderName::from_static("strict-transport-security"),
                        val,
                    );
                }
            }

            if config.csp {
                if let Ok(val) = axum::http::HeaderValue::from_str(&config.csp_value) {
                    headers.insert(
                        axum::http::HeaderName::from_static("content-security-policy"),
                        val,
                    );
                }
            }

            if config.referrer_policy {
                if let Ok(val) = axum::http::HeaderValue::from_str(&config.referrer_policy_value) {
                    headers.insert(
                        axum::http::HeaderName::from_static("referrer-policy"),
                        val,
                    );
                }
            }

            if config.permissions_policy {
                if let Ok(val) = axum::http::HeaderValue::from_str(&config.permissions_policy_value) {
                    headers.insert(
                        axum::http::HeaderName::from_static("permissions-policy"),
                        val,
                    );
                }
            }

            Ok(resp)
        })
    }
}
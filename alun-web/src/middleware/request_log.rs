//! 请求访问日志中间件
//!
//! 记录请求路径、状态码、耗时，输出到 tracing（终端/文件/ELK）。
//! 如需操作审计日志（含请求参数、响应体、脱敏、写库），
//! 请通过 `App::with_middleware_hook` 注入项目级中间件。

use axum::{extract::Request, response::Response};
use axum::extract::ConnectInfo;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;
use tower::{Layer, Service};
use tracing::{info, warn};

/// 请求访问日志中间件：记录请求路径 + 状态码 + 耗时
///
/// 通过 config.toml 中 `[middleware]` 下的 `request_log` 和 `request_log_config` 控制开关和参数。
#[derive(Clone)]
pub struct RequestLogLayer {
    /// 不记录日志的路径列表
    pub exclude_paths: Vec<String>,
    /// 是否记录请求耗时
    pub log_duration: bool,
}

impl Default for RequestLogLayer {
    fn default() -> Self {
        Self {
            exclude_paths: vec!["/health".into(), "/favicon.ico".into()],
            log_duration: true,
        }
    }
}

impl RequestLogLayer {
    pub fn new() -> Self { Self::default() }

    pub fn exclude_paths(mut self, paths: Vec<String>) -> Self {
        self.exclude_paths = paths;
        self
    }
}

impl<S> Layer<S> for RequestLogLayer {
    type Service = RequestLogService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestLogService {
            inner,
            exclude_paths: self.exclude_paths.clone(),
            log_duration: self.log_duration,
        }
    }
}

#[derive(Clone)]
/// 请求日志服务——记录请求路径、状态码、耗时
pub struct RequestLogService<S> {
    /// 下游服务
    inner: S,
    /// 不记录日志的路径列表
    exclude_paths: Vec<String>,
    /// 是否记录请求耗时
    log_duration: bool,
}

impl<S, ReqBody> Service<Request<ReqBody>> for RequestLogService<S>
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

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let method = req.method().clone();
        let path = req.uri().path().to_string();
        let connect_info = req.extensions().get::<ConnectInfo<SocketAddr>>().cloned();
        let req_id = req.headers()
            .iter()
            .find(|(name, _)| name.as_str().to_ascii_lowercase() == "x-request-id")
            .and_then(|(_, v)| v.to_str().ok())
            .unwrap_or("-")
            .to_string();
        let start = Instant::now();
        let should_log = !self.exclude_paths.contains(&path);

        let mut inner = self.inner.clone();
        let log_duration = self.log_duration;
        let ip = req.headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .or_else(|| req.headers().get("x-real-ip").and_then(|v| v.to_str().ok()))
            .map(|s| s.to_string())
            .or_else(|| connect_info.map(|ci| ci.0.ip().to_string()));
        Box::pin(async move {
            let result = inner.call(req).await;
            if !should_log { return result; }

            let elapsed = start.elapsed();
            let duration_str = if log_duration {
                format!("{}ms", elapsed.as_millis())
            } else {
                String::new()
            };
            match &result {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if status >= 500 {
                        warn!(
                            method = %method, path = %path, status = status,
                            duration = %duration_str, request_id = %req_id,
                            ip = ?ip,
                            "请求异常"
                        );
                    } else {
                        info!(
                            method = %method, path = %path, status = status,
                            duration = %duration_str, request_id = %req_id,
                            ip = ?ip,
                            "请求完成"
                        );
                    }
                }
                Err(_e) => {
                    warn!(
                        method = %method, path = %path,
                        duration = %duration_str, request_id = %req_id,
                        "请求失败"
                    );
                }
            }
            result
        })
    }
}
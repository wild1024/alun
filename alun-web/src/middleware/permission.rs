//! 权限校验中间件

use axum::{
    extract::Request,
    response::Response,
    body::Body,
};
use axum::http::StatusCode;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};
use crate::response::{Res, codes};
use super::AuthClaims;

/// 方法级权限拦截层（any 匹配：拥有任意一个权限即放行）
///
/// 通过 `RequirePermissionLayer::any()` 创建，配合 `App::with_permission()` 使用。
#[derive(Clone)]
pub struct RequirePermissionLayer {
    /// 所需的任一权限列表
    pub permissions: Vec<String>,
}

impl RequirePermissionLayer {
    /// 创建 any 匹配的权限拦截层
    pub fn any(permissions: Vec<String>) -> Self { Self { permissions } }
}

impl<S> Layer<S> for RequirePermissionLayer {
    type Service = RequirePermissionService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequirePermissionService { inner, permissions: self.permissions.clone() }
    }
}

#[derive(Clone)]
/// 方法级权限拦截服务——校验用户是否拥有所需权限
pub struct RequirePermissionService<S> {
    /// 下游服务
    inner: S,
    /// 所需的权限列表
    permissions: Vec<String>,
}

impl<S> Service<Request<Body>> for RequirePermissionService<S>
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
        match req.extensions().get::<AuthClaims>() {
            Some(AuthClaims(claims)) => {
                let has_perm = self.permissions.iter().any(|p| claims.has_permission(p));
                if has_perm {
                    let mut inner = self.inner.clone();
                    Box::pin(async move { inner.call(req).await })
                } else {
                    let body = serde_json::to_string(&Res::<()>::fail(
                        codes::FORBIDDEN, "无权限访问，权限不足"
                    )).unwrap_or_else(|_| r#"{"code":403,"msg":"无权限访问，权限不足"}"#.to_string());
                    let resp = Response::builder()
                        .status(StatusCode::FORBIDDEN)
                        .header("Content-Type", "application/json; charset=utf-8")
                        .body(Body::from(body)).expect("response body build failed");
                    Box::pin(async move { Ok(resp) })
                }
            }
            None => {
                let body = serde_json::to_string(&Res::<()>::fail(
                    codes::UNAUTHORIZED, "未认证，请先登录"
                )).unwrap_or_else(|_| r#"{"code":401,"msg":"未认证，请先登录"}"#.to_string());
                let resp = Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header("Content-Type", "application/json; charset=utf-8")
                    .body(Body::from(body)).expect("response body build failed");
                Box::pin(async move { Ok(resp) })
            }
        }
    }
}

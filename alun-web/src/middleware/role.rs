//! 角色校验中间件

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

#[derive(Clone)]
/// 角色拦截层——拥有任意一个角色即放行
pub struct RequireRoleLayer {
    pub roles: Vec<String>,
}

impl RequireRoleLayer {
    pub fn any(roles: Vec<String>) -> Self { Self { roles } }
}

impl<S> Layer<S> for RequireRoleLayer {
    type Service = RequireRoleService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequireRoleService { inner, roles: self.roles.clone() }
    }
}

#[derive(Clone)]
/// 角色拦截服务——校验用户是否拥有所需角色
pub struct RequireRoleService<S> {
    /// 下游服务
    inner: S,
    /// 所需角色列表
    roles: Vec<String>,
}

impl<S> Service<Request<Body>> for RequireRoleService<S>
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
                let has_role = self.roles.iter().any(|r| claims.has_role(r));
                if has_role {
                    let mut inner = self.inner.clone();
                    Box::pin(async move { inner.call(req).await })
                } else {
                    let body = serde_json::to_string(&Res::<()>::fail(
                        codes::FORBIDDEN, "无权限访问，角色不足"
                    )).unwrap_or_else(|_| r#"{"code":403,"msg":"无权限访问，角色不足"}"#.to_string());
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

//! 认证中间件：JWT Bearer Token 验证 + 黑名单检查

use axum::{
    extract::Request,
    response::Response,
    body::Body,
};
use axum::http::StatusCode;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::collections::HashSet;
use tower::{Layer, Service};
use crate::response::{Res, codes};
use super::{UserId, AuthClaims, TokenClaims};

/// JWT 认证中间件
///
/// 从 `Authorization: Bearer <token>` 中提取并验证 JWT，
/// 将解析出的 `UserContext` 挂载到 `request.extensions` 中。
/// 失败返回 401 Unauthorized。
#[derive(Clone)]
pub struct AuthLayer {
    /// JWT 密钥（HS256）
    pub jwt_secret: String,
    /// 跳过认证的路径列表（如 `/public/*`、`/api/login`）
    pub ignore_paths: Vec<String>,
    /// 缓存层引用（用于缓存用户信息）
    #[cfg(feature = "cache")]
    pub cache: Option<alun_cache::SharedCache>,
}

impl<S> Layer<S> for AuthLayer {
    type Service = AuthService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthService {
            inner,
            jwt_secret: self.jwt_secret.clone(),
            ignore_paths: self.ignore_paths.iter().cloned().collect(),
            #[cfg(feature = "cache")]
            cache: self.cache.clone(),
        }
    }
}

#[derive(Clone)]
/// JWT 认证服务——验证 Bearer Token 并注入用户上下文到请求 extensions
pub struct AuthService<S> {
    /// 下游服务
    inner: S,
    /// JWT 密钥（HS256）
    jwt_secret: String,
    /// 跳过认证的路径集合
    ignore_paths: HashSet<String>,
    /// 缓存层引用（用于缓存用户信息）
    #[cfg(feature = "cache")]
    cache: Option<alun_cache::SharedCache>,
}

impl<S> Service<Request<Body>> for AuthService<S>
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

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let path = req.uri().path().to_string();
        // 检查路径是否在忽略列表中：
        // 1. 精确匹配
        // 2. 前缀匹配（如 /api 匹配 /api/xxx，/api/files/dl 匹配 /api/files/dl/xxx）
        let is_ignore_path = self.ignore_paths.contains(&path)
            || self.ignore_paths.iter().any(|p| path.starts_with(&format!("{}/", p)));

        let token_opt: Option<&str> = req.headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        match token_opt {
            Some(token) => match validate_and_extract_claims(&self.jwt_secret, token) {
                Ok(claims) => {
                    #[cfg(feature = "cache")]
                    let cache = self.cache.clone();
                    let mut inner = self.inner.clone();
                    #[allow(unused_variables)]
                    let is_ignore = is_ignore_path;
                    Box::pin(async move {
                        #[cfg(feature = "cache")]
                        {
                            // 黑名单检查：仅在非 ignore_path 时拒绝，ignore_path 应放行
                            if !is_ignore {
                                if let (Some(ref c), Some(ref jti)) = (&cache, &claims.jti) {
                                    let key = format!("token:blacklist:{}", jti);
                                    if let Ok(Some(_)) = alun_cache::Cache::get::<serde_json::Value>(c, &key).await {
                                        let body = serde_json::to_string(&Res::<()>::fail(
                                            codes::UNAUTHORIZED, "Token 已登出，请重新登录"
                                        )).unwrap_or_else(|_| r#"{"code":401,"msg":"Token 已登出，请重新登录"}"#.to_string());
                                        return Ok(Response::builder()
                                            .status(StatusCode::UNAUTHORIZED)
                                            .header("Content-Type", "application/json; charset=utf-8")
                                            .body(Body::from(body))
                                            .expect("response body build failed"));
                                    }
                                }
                            }
                        }
                        // 有有效 Token，注入用户信息（ignore_path 也注入，以便业务可以获取用户信息）
                        req.extensions_mut().insert(UserId(claims.sub.clone()));
                        req.extensions_mut().insert(AuthClaims(claims.clone()));
                        let mut response = inner.call(req).await?;
                        response.extensions_mut().insert(AuthClaims(claims));
                        Ok(response)
                    })
                }
                Err(_) => {
                    if is_ignore_path {
                        // ignore_path 上的无效 Token，忽略错误继续处理，不注入用户信息
                        let mut inner = self.inner.clone();
                        Box::pin(async move { inner.call(req).await })
                    } else {
                        let body = serde_json::to_string(&Res::<()>::fail(
                            codes::UNAUTHORIZED, "Token 无效或已过期"
                        )).unwrap_or_else(|_| r#"{"code":401,"msg":"Token 无效或已过期"}"#.to_string());
                        Box::pin(async move {
                            Ok(Response::builder()
                                .status(StatusCode::UNAUTHORIZED)
                                .header("Content-Type", "application/json; charset=utf-8")
                                .body(Body::from(body))
                                .expect("response body build failed"))
                        })
                    }
                }
            },
            None => {
                if is_ignore_path {
                    // ignore_path 上没有 Token，直接放行
                    let mut inner = self.inner.clone();
                    Box::pin(async move { inner.call(req).await })
                } else {
                    let body = serde_json::to_string(&Res::<()>::fail(
                        codes::UNAUTHORIZED, "未授权访问，请先登录"
                    )).unwrap_or_else(|_| r#"{"code":401,"msg":"未授权访问，请先登录"}"#.to_string());
                    Box::pin(async move {
                        Ok(Response::builder()
                            .status(StatusCode::UNAUTHORIZED)
                            .header("Content-Type", "application/json; charset=utf-8")
                            .body(Body::from(body))
                            .expect("response body build failed"))
                    })
                }
            }
        }
    }
}

fn validate_and_extract_claims(secret: &str, token: &str) -> Result<TokenClaims, String> {
    use jsonwebtoken::{decode, DecodingKey, Validation};

    let token_data = decode::<TokenClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| format!("Token 验证失败: {}", e))?;

    Ok(token_data.claims)
}

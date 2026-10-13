//! 认证中间件：JWT Bearer Token 验证 + 黑名单检查

use super::{AuthClaims, TokenClaims, TokenType, UserId};
use crate::response::{Res, codes};
use axum::http::StatusCode;
use axum::{body::Body, extract::Request, response::Response};
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};

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
        // CORS 预检请求（OPTIONS）不携带凭据，跳过认证直接放行。
        // 浏览器发送跨域请求（带 Authorization 头）前会先发 OPTIONS 预检，
        // 若被认证中间件拦截返回 401，会导致浏览器阻止实际请求（ERR_FAILED）。
        // /auth/login 等公开路由的 OPTIONS 碰巧通过（在 ignore_paths 中），
        // 但 /auth/me、/auth/menus 等受保护路由的预检会被拦截。
        if req.method() == axum::http::Method::OPTIONS {
            let mut inner = self.inner.clone();
            return Box::pin(async move { inner.call(req).await });
        }

        let path = req.uri().path().to_string();
        // 检查路径是否在忽略列表中：
        // 1. 精确匹配
        // 2. 前缀匹配（如 /api 匹配 /api/xxx，/api/files/dl 匹配 /api/files/dl/xxx）
        let is_ignore_path = self.ignore_paths.contains(&path)
            || self
                .ignore_paths
                .iter()
                .any(|p| path.starts_with(&format!("{}/", p)));

        let token_opt: Option<&str> = req
            .headers()
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
                            if !is_ignore && let (Some(c), Some(jti)) = (&cache, &claims.jti) {
                                let key = format!("token:blacklist:{}", jti);
                                if let Ok(Some(_)) =
                                    alun_cache::Cache::get::<serde_json::Value>(c, &key).await
                                {
                                    let body = serde_json::to_string(&Res::<()>::fail(
                                        codes::UNAUTHORIZED,
                                        "Token 已登出，请重新登录",
                                    ))
                                    .unwrap_or_else(|_| {
                                        r#"{"code":401,"msg":"Token 已登出，请重新登录"}"#
                                            .to_string()
                                    });
                                    return Ok(Response::builder()
                                        .status(StatusCode::UNAUTHORIZED)
                                        .header("Content-Type", "application/json; charset=utf-8")
                                        .body(Body::from(body))
                                        .expect("response body build failed"));
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
                            codes::UNAUTHORIZED,
                            "Token 无效或已过期",
                        ))
                        .unwrap_or_else(|_| {
                            r#"{"code":401,"msg":"Token 无效或已过期"}"#.to_string()
                        });
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
                        codes::UNAUTHORIZED,
                        "未授权访问，请先登录",
                    ))
                    .unwrap_or_else(|_| r#"{"code":401,"msg":"未授权访问，请先登录"}"#.to_string());
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

/// 校验 JWT 并提取声明 —— **认证的唯一真值实现**。
///
/// `AuthLayer` 与 `AuthClaims` 的提取器（`FromRequestParts` 通道 2）都调用它，
/// 因此两者对「Token 是否有效」的判定永远一致。新增校验规则（如受众 `aud`、
/// 签发方 `iss`、时钟偏移）只需改这里，两处自动同步。
///
/// # 错误
///
/// 返回 `Err` 的原因字符串仅用于日志/调试；对外一律映射为 401，不泄漏细节。
pub(crate) fn validate_and_extract_claims(
    secret: &str,
    token: &str,
) -> Result<TokenClaims, String> {
    use jsonwebtoken::{DecodingKey, Validation, decode};

    let token_data = decode::<TokenClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| format!("Token 验证失败: {}", e))?;

    // 安全修复 P1-3：拒绝 Refresh Token 被当作 Access Token 使用。
    // Refresh Token 有效期更长且携带空权限数组，若被接受为 Access Token，
    // 会延长攻击者对仅按 claims.sub 过滤的用户端点的访问窗口。
    if token_data.claims.token_type != Some(TokenType::Access) {
        return Err("Token 类型不正确，需要 Access Token".into());
    }

    Ok(token_data.claims)
}

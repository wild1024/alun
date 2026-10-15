//! 路径权限校验中间件（配置 + 宏注解联合校验）

use super::AuthClaims;
use crate::response::Res;
use axum::http::StatusCode;
use axum::{body::Body, extract::Request, response::Response};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tower::{Layer, Service};

/// 将路径按 `/` 分段，逐段比对。`{param}` 占位符匹配任意非空段。
fn match_path(request_path: &str, rule_path: &str) -> bool {
    if request_path == rule_path {
        return true;
    }
    let req_segments: Vec<&str> = request_path.trim_matches('/').split('/').collect();
    let rule_segments: Vec<&str> = rule_path.trim_matches('/').split('/').collect();
    if req_segments.len() != rule_segments.len() {
        return false;
    }
    for (req, rule) in req_segments.iter().zip(rule_segments.iter()) {
        if rule.starts_with('{') && rule.ends_with('}') {
            continue;
        }
        if req != rule {
            return false;
        }
    }
    true
}

/// 路径级权限校验中间件（配置 + 宏注解联合校验）
///
/// 合并 `config.toml` 中 `[middleware.permission.rules]` 的静态规则
/// 和 `#[permission]` 宏注解的编译期规则。
#[derive(Clone)]
pub struct PermissionCheckLayer {
    /// 权限规则列表
    ///
    /// 以 `Arc` 共享：`Layer::layer()` 会为**每条路由**生成一个服务实例
    /// （且在生成前 axum 还会按路由克隆一次 layer），若每次直接克隆 `Vec`，
    /// 装配成本为 `O(路由数 × 规则数)` 的深克隆（每条规则含多个 `String`）。
    /// 实测在 ~200 路由 × ~200 规则时占启动期中间件装配的 90% 以上
    /// （155–213ms / 171–232ms，debug）；共享后每次生成服务只做引用计数递增。
    pub rules: Arc<Vec<PermissionRule>>,
}

/// 权限匹配规则
#[derive(Debug, Clone)]
pub struct PermissionRule {
    /// URL 路径（精确或前缀匹配）
    pub path: String,
    /// 适用的 HTTP 方法列表（空表示所有方法）
    pub methods: Vec<String>,
    /// 所需权限标识
    pub permission: String,
}

impl PermissionCheckLayer {
    /// 从配置文件规则创建
    pub fn from_config(config_rules: &[alun_config::PermissionRule]) -> Self {
        Self {
            rules: Arc::new(
                config_rules
                    .iter()
                    .map(|r| PermissionRule {
                        path: r.path.clone(),
                        methods: r.methods.clone(),
                        permission: r.permission.clone(),
                    })
                    .collect(),
            ),
        }
    }

    /// 合并 `#[permission]` 宏注解的编译期规则
    pub fn with_macro_rules(mut self, rules: &[crate::PermissionDef]) -> Self {
        let list = Arc::make_mut(&mut self.rules);
        for def in rules {
            list.push(PermissionRule {
                path: def.path.to_string(),
                methods: vec![def.method.to_string()],
                permission: def.permission.to_string(),
            });
        }
        self
    }

    /// 追加一条规则（装配阶段使用；`Arc` 共享下按写时复制处理）
    pub fn push_rule(&mut self, rule: PermissionRule) {
        Arc::make_mut(&mut self.rules).push(rule);
    }

    /// 是否有权限规则（无规则时中间件不生效）
    pub fn has_rules(&self) -> bool {
        !self.rules.is_empty()
    }
}

impl<S> Layer<S> for PermissionCheckLayer {
    type Service = PermissionCheckService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        PermissionCheckService {
            inner,
            rules: Arc::clone(&self.rules),
        }
    }
}

#[derive(Clone)]
/// 路径权限校验服务——基于请求路径和方法匹配权限规则
pub struct PermissionCheckService<S> {
    /// 下游服务
    inner: S,
    /// 权限规则列表（与层共享，装配时零深克隆）
    rules: Arc<Vec<PermissionRule>>,
}

impl<S> Service<Request<Body>> for PermissionCheckService<S>
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
        let path = req.uri().path().to_string();
        let method = req.method().to_string();

        match self.rules.iter().find(|rule| {
            let path_matches = match_path(&path, &rule.path);
            let method_matches = rule.methods.is_empty()
                || rule.methods.iter().any(|m| m.eq_ignore_ascii_case(&method));
            path_matches && method_matches
        }) {
            Some(rule) => {
                let required_perm = rule.permission.clone();
                let claims_present = req.extensions().get::<AuthClaims>().cloned();

                let has_perm = match &claims_present {
                    Some(AuthClaims(claims)) => {
                        tracing::debug!(
                            "权限检查: path={}, required={}, user_perms={:?}",
                            path,
                            required_perm,
                            claims.permissions
                        );
                        claims.has_permission(&required_perm)
                    }
                    None => {
                        tracing::warn!(
                            "权限检查: path={}, required={}, AuthClaims 未注入 (auth 中间件未执行或配置有误)",
                            path,
                            required_perm
                        );
                        false
                    }
                };

                if has_perm {
                    let mut inner = self.inner.clone();
                    Box::pin(async move { inner.call(req).await })
                } else {
                    let body = serde_json::to_string(&Res::<()>::fail(
                        StatusCode::FORBIDDEN.as_u16() as i32,
                        format!("无权限访问，需要权限: {}", required_perm),
                    ))
                    .unwrap_or_else(|_| {
                        r#"{"code":403,"msg":"无权限访问，需要权限: xxx"}"#.to_string()
                    });
                    let resp = Response::builder()
                        .status(StatusCode::FORBIDDEN)
                        .header("Content-Type", "application/json; charset=utf-8")
                        .body(Body::from(body))
                        .expect("response body build failed");
                    Box::pin(async move { Ok(resp) })
                }
            }
            None => {
                let mut inner = self.inner.clone();
                Box::pin(async move { inner.call(req).await })
            }
        }
    }
}

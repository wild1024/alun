//! 多租户上下文中间件。
//!
//! 从请求的 JWT 载荷中读取租户 claim，写入 [`alun_db::prelude::tenant_scope`]
//! 的 `task_local` 上下文，使下游所有模型查询 / 写入自动带上租户条件。
//!
//! ## 中间件顺序
//!
//! 必须挂在认证层**之内**（先 `.layer(TenantLayer)` 再 `.layer(AuthLayer)`）：
//! 认证层先验证签名并把 [`AuthClaims`] 注入 `request.extensions`，租户层据此
//! 判断「载荷可信」后才会读取 claim——**未经验证的 token 一律忽略**，
//! 避免攻击者伪造 `tenant_id` 越权。
//!
//! ## 失败模式
//!
//! claim 缺失 / 无法解析时**不设置**租户上下文（而非报错放行）。此时：
//! - 声明了 `#[tenant]` 的模型在查询前 fail-closed 报错（不会读到全量数据）；
//! - 未声明租户列 / `#[model(cross_tenant)]` 的模型不受影响。

use axum::{body::Body, extract::Request, response::Response};
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Layer, Service};

use alun_db::prelude::{TenantContext, Value};

use super::AuthClaims;

/// 租户上下文中间件。
///
/// 由 `App` 在 `[tenant].enabled = true` 时自动装配，业务代码通常无需手动挂载。
#[derive(Clone)]
pub struct TenantLayer {
    /// JWT claim 中承载租户 ID 的字段名（默认 `tenant_id`）。
    pub claim: String,
    /// 跳过租户上下文注入的路径列表（如登录、健康检查）。
    pub ignore_paths: Vec<String>,
}

impl<S> Layer<S> for TenantLayer {
    type Service = TenantService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TenantService {
            inner,
            claim: self.claim.clone(),
            ignore_paths: self.ignore_paths.iter().cloned().collect(),
        }
    }
}

/// 租户上下文中间件服务——解析租户 claim 并以 `task_local` 作用域包裹下游调用。
#[derive(Clone)]
pub struct TenantService<S> {
    /// 下游服务
    inner: S,
    /// JWT claim 中承载租户 ID 的字段名
    claim: String,
    /// 跳过租户上下文注入的路径集合
    ignore_paths: HashSet<String>,
}

impl<S> Service<Request<Body>> for TenantService<S>
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
        // CORS 预检（OPTIONS）不携带凭据，与认证层保持一致直接放行。
        if req.method() == axum::http::Method::OPTIONS {
            let mut inner = self.inner.clone();
            return Box::pin(async move { inner.call(req).await });
        }

        let path = req.uri().path().to_string();
        let is_ignore_path = self.ignore_paths.contains(&path)
            || self
                .ignore_paths
                .iter()
                .any(|p| path.starts_with(&format!("{}/", p)));

        // 只有认证层已验证通过（存在 `AuthClaims`）时才信任载荷载荷中的租户 claim。
        let is_verified = req.extensions().get::<AuthClaims>().is_some();

        let tenant = if is_ignore_path || !is_verified {
            None
        } else {
            extract_tenant(&req, &self.claim)
        };

        match tenant {
            Some(context) => {
                let mut inner = self.inner.clone();
                Box::pin(
                    async move { alun_db::prelude::tenant_scope(context, inner.call(req)).await },
                )
            }
            None => {
                let mut inner = self.inner.clone();
                Box::pin(async move { inner.call(req).await })
            }
        }
    }
}

/// 从请求的 Bearer Token 载荷中解析租户上下文。
///
/// 仅做**载荷解码**（签名与有效期已由认证层校验），因此不重复做 HMAC 校验；
/// 调用方必须确保 `AuthClaims` 已存在。
///
/// # 返回
///
/// - 载荷可解析且 claim 为标量 → `Some(TenantContext::Scoped(值))`
/// - 无凭据 / 载荷无法解析 / claim 缺失或非标量 → `None`
fn extract_tenant(req: &Request<Body>, claim: &str) -> Option<TenantContext> {
    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))?;

    let payload: serde_json::Value = jsonwebtoken::dangerous::insecure_decode_claims(token).ok()?;
    let raw = payload.get(claim)?;
    json_to_tenant_value(raw).map(TenantContext::Scoped)
}

/// 把 JWT 中的租户 claim 转换为数据库参数值。
///
/// 租户 ID 应为标量：整数（有符号 / 无符号）、字符串或布尔。
/// 其余形态（对象 / 数组 / 浮点 / null）返回 `None`，交由 fail-closed 守卫拦截。
fn json_to_tenant_value(raw: &serde_json::Value) -> Option<Value> {
    match raw {
        serde_json::Value::Number(number) => number
            .as_i64()
            .map(Value::I64)
            .or_else(|| number.as_u64().map(Value::U64)),
        serde_json::Value::String(text) => Some(Value::Text(text.clone())),
        serde_json::Value::Bool(flag) => Some(Value::Bool(*flag)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::State;
    use axum::routing::get;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::ServiceExt;

    /// 端到端测试用的租户模型：`tenant_id` 为普通业务字段。
    #[derive(alun_db::prelude::Model, Debug, Clone, PartialEq)]
    #[table("invoices")]
    struct TenantInvoice {
        /// 主键
        #[key]
        id: i64,
        /// 标题
        title: String,
        /// 租户列：行级隔离依据
        #[tenant]
        tenant_id: i64,
    }

    /// 读取当前租户可见的发票标题（模型查询自动应用上下文租户作用域）。
    async fn list_invoices(
        State(db): State<alun_db::prelude::Db>,
    ) -> Result<axum::Json<Vec<String>>, axum::http::StatusCode> {
        use alun_db::prelude::CrudExt;

        let rows = TenantInvoice::query()
            .find(&db)
            .await
            .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok(axum::Json(rows.into_iter().map(|r| r.title).collect()))
    }

    /// 测试用 JWT 密钥（HS256）。
    const SECRET: &str = "alun-web-tenant-test-secret-32bytes";

    /// 签发携带租户 claim 的 Access Token。
    fn token_with_tenant(tenant: i64) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let claims = serde_json::json!({
            "sub": "u-1",
            "tenant_id": tenant,
            "token_type": "access",
            "exp": now + 3600,
            "iat": now,
        });
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .expect("签发测试 token")
    }

    /// 签发**不含**租户 claim 的 Access Token。
    fn token_without_tenant() -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let claims = serde_json::json!({
            "sub": "u-1",
            "token_type": "access",
            "exp": now + 3600,
            "iat": now,
        });
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .expect("签发测试 token")
    }

    /// 回显当前租户上下文的小型 handler。
    async fn who() -> axum::Json<serde_json::Value> {
        let tenant = alun_db::prelude::current_tenant().map(|ctx| format!("{ctx:?}"));
        axum::Json(serde_json::json!({ "tenant": tenant }))
    }

    /// 认证层（用于注入 `AuthClaims`）。
    fn auth_layer() -> crate::middleware::AuthLayer {
        crate::middleware::AuthLayer {
            jwt_secret: SECRET.to_string(),
            ignore_paths: Vec::new(),
            #[cfg(feature = "cache")]
            cache: None,
        }
    }

    /// 组装「认证 → 租户」链；`ignore_paths` 透传给租户层。
    fn router(ignore_paths: Vec<String>) -> Router {
        Router::new()
            .route("/api/who", get(who))
            .layer(TenantLayer {
                claim: "tenant_id".into(),
                ignore_paths,
            })
            .layer(auth_layer())
    }

    /// 发送带可选 Bearer Token 的 GET，返回响应体 JSON。
    async fn call(
        router: &Router,
        path: &str,
        token: Option<&str>,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method("GET").uri(path);
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let response = router
            .clone()
            .oneshot(builder.body(Body::empty()).expect("构造请求"))
            .await
            .expect("oneshot");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("读取响应体");
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// 不同租户的 token 各自进入自己的上下文，不互相串扰。
    #[tokio::test]
    async fn tenant_context_is_per_request() {
        let app = router(Vec::new());

        let (status, body) = call(&app, "/api/who", Some(&token_with_tenant(7))).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            body["tenant"]
                .as_str()
                .unwrap_or_default()
                .contains("I64(7)"),
            "租户 7 的请求应进入 Scoped(I64(7))：{body}"
        );

        let (_, body) = call(&app, "/api/who", Some(&token_with_tenant(8))).await;
        assert!(
            body["tenant"]
                .as_str()
                .unwrap_or_default()
                .contains("I64(8)"),
            "同一链路上租户 8 不应看到租户 7 的上下文：{body}"
        );
    }

    /// claim 缺失时不设置上下文（模型级 fail-closed 守卫在 DB 层拦截）。
    #[tokio::test]
    async fn missing_claim_leaves_context_unset() {
        let app = router(Vec::new());
        let (status, body) = call(&app, "/api/who", Some(&token_without_tenant())).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            body["tenant"].is_null(),
            "无租户 claim 时不得设置上下文：{body}"
        );
    }

    /// ignore_paths 命中的路径跳过注入（与认证层白名单对齐）。
    #[tokio::test]
    async fn ignore_path_skips_injection() {
        let app = router(vec!["/api/who".into()]);
        let (status, body) = call(&app, "/api/who", Some(&token_with_tenant(9))).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            body["tenant"].is_null(),
            "白名单路径不应注入租户上下文：{body}"
        );
    }

    /// 无认证层时（无 `AuthClaims`）不信任载荷中的租户 claim。
    #[tokio::test]
    async fn unverified_payload_is_not_trusted() {
        let app = Router::new()
            .route("/api/who", get(who))
            .layer(TenantLayer {
                claim: "tenant_id".into(),
                ignore_paths: Vec::new(),
            });

        let (status, body) = call(&app, "/api/who", Some(&token_with_tenant(7))).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(
            body["tenant"].is_null(),
            "未经认证层验证的 token 不得注入租户上下文：{body}"
        );
    }

    /// 端到端：两个租户的 token 各自只能看到本租户数据，互不可见。
    #[tokio::test]
    async fn two_tenants_cannot_see_each_others_rows() {
        use alun_db::prelude::Db;

        // 内存库 + 两个租户的发票数据（种子走原生 SQL，不经模型写入逻辑）。
        let db = Db::connect("sqlite::memory:").await.expect("创建内存库");
        db.execute(
            "CREATE TABLE invoices (
                id INTEGER PRIMARY KEY,
                title TEXT NOT NULL,
                tenant_id INTEGER NOT NULL
            )",
        )
        .await
        .expect("建表");
        db.execute(
            "INSERT INTO invoices (id, title, tenant_id) VALUES
                (1, 't1-a', 1), (2, 't1-b', 1), (3, 't2-a', 2)",
        )
        .await
        .expect("种子数据");

        // 与生产一致的「认证 → 租户」链：租户层在认证层之内。
        let app = Router::new()
            .route("/api/invoices", get(list_invoices))
            .with_state(db)
            .layer(TenantLayer {
                claim: "tenant_id".into(),
                ignore_paths: Vec::new(),
            })
            .layer(auth_layer());

        let (status, body) = call(&app, "/api/invoices", Some(&token_with_tenant(1))).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(
            body,
            serde_json::json!(["t1-a", "t1-b"]),
            "租户 1 只看自己的行"
        );

        let (status, body) = call(&app, "/api/invoices", Some(&token_with_tenant(2))).await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(
            body,
            serde_json::json!(["t2-a"]),
            "租户 2 看不到租户 1 的行"
        );

        // 无 token 时认证层拦截，租户上下文不会兜底成全量。
        let (status, _) = call(&app, "/api/invoices", None).await;
        assert_eq!(status, axum::http::StatusCode::UNAUTHORIZED);
    }
}

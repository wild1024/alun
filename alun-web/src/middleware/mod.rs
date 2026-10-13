//! Alun 请求中间件模块
//!
//! 按职责拆分为多个子模块：
//! | 模块               | 功能                 |
//! |-------------------|---------------------|
//! | [request_id]      | 请求 ID 生成/注入     |
//! | [auth]            | JWT Bearer Token 认证 |
//! | [role]            | 角色校验              |
//! | [permission]      | 权限标识校验          |
//! | [permission_check]| 路径权限规则校验       |
//! | [principal]       | 当前操作者的结构化身份视图 |
//! | [request_log]     | 请求访问日志（tracing 输出） |
//! | [rate_limit]      | IP 滑动窗口限流       |
//! | [security_headers]| 安全响应头注入         |
//! | [nonce]           | Nonce 防重放（按需，`[middleware.nonce]` 启用）    |
//! | [idempotency]     | 幂等键（按需，`[middleware.idempotency]` 启用）    |
//! | [tenant]          | 多租户上下文注入（按需，`[tenant]` 启用，需 `db` feature） |

pub mod auth;
#[cfg(feature = "cache")]
pub mod idempotency;
#[cfg(feature = "cache")]
pub mod nonce;
pub mod permission;
pub mod permission_check;
pub mod principal;
pub mod rate_limit;
pub mod request_id;
pub mod request_log;
pub mod role;
pub mod security_headers;
#[cfg(feature = "db")]
pub mod tenant;

pub use auth::{AuthLayer, AuthService};
#[cfg(feature = "cache")]
pub use idempotency::{IdempotencyLayer, IdempotencyService};
#[cfg(feature = "cache")]
pub use nonce::{NonceLayer, NonceService};
pub use permission::{RequirePermissionLayer, RequirePermissionService};
pub use permission_check::{PermissionCheckLayer, PermissionCheckService, PermissionRule};
pub use principal::Principal;
pub use rate_limit::{IpWindow, RateLimitLayer, RateLimitService};
pub use request_id::{RequestIdLayer, RequestIdService};
pub use request_log::{RequestLogLayer, RequestLogService};
pub use role::{RequireRoleLayer, RequireRoleService};
pub use security_headers::{SecurityHeadersLayer, SecurityHeadersService};
#[cfg(feature = "db")]
pub use tenant::{TenantLayer, TenantService};

use crate::api::ApiError;
use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::{header, request::Parts};

/// Token 类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TokenType {
    /// 访问 Token
    #[serde(rename = "access")]
    Access,
    /// 刷新 Token
    #[serde(rename = "refresh")]
    Refresh,
}

/// JWT Token 声明（与 jsonwebtoken 配合使用）
///
/// 由 AuthLayer 中间件验证后注入到 `request.extensions`，
/// 业务代码可通过 `Extension<AuthClaims>` 提取。
///
/// # 自定义声明
///
/// [`extra`](Self::extra) 是**通用扩展位**：除下表固定字段之外的一切载荷
/// 都落在其中，并以 `#[serde(flatten)]` 与固定字段平铺在 **JSON 顶层**。
/// 因此多租户的 `tenant_id`、组织 / 门店维度、会话版本号、`locale` 等
/// 无需修改框架即可随 Token 传递。
///
/// ```ignore
/// // 签发（注意 claim 平铺在顶层，与固定字段同级）
/// let token = jwt.issue(
///     ClaimsBuilder::new("u-1")
///         .username("alice")
///         .claim("tenant_id", serde_json::json!(42))
/// )?;
///
/// // 读取
/// let claims = jwt.validate(&token)?;
/// assert_eq!(claims.extra_i64("tenant_id"), Some(42));
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TokenClaims {
    /// JWT 唯一标识（jti），用于 Token 黑名单/撤销机制
    #[serde(default)]
    pub jti: Option<String>,
    /// 用户 ID（subject）
    pub sub: String,
    /// 用户名
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// 角色列表（如 ["admin", "user"]）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// 权限列表（如 ["user:read", "user:write"]）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,
    /// Token 类型
    #[serde(default)]
    pub token_type: Option<TokenType>,
    /// 过期时间（Unix 时间戳，秒）
    pub exp: usize,
    /// 签发时间（Unix 时间戳，秒）
    #[serde(default)]
    pub iat: usize,
    /// 自定义声明（通用扩展位）。
    ///
    /// `#[serde(flatten)]` 使其与固定字段**平铺在 JSON 顶层**，因此
    /// 既有的按顶层字段名读取载荷的代码（含租户中间件）无需改动。
    /// 反序列化时未被固定字段识别的一律收入此处；旧 Token（无任何自定义
    /// 声明）解析后为空 map，天然向后兼容。
    #[serde(flatten, default)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,
}

impl TokenClaims {
    /// 获取用户 ID
    pub fn user_id(&self) -> &str {
        &self.sub
    }

    /// 是否拥有指定角色
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|r| r == role)
    }

    /// 是否拥有任一角色
    pub fn has_any_role(&self, roles: &[&str]) -> bool {
        roles.iter().any(|r| self.has_role(r))
    }

    /// 是否拥有所有角色
    pub fn has_all_roles(&self, roles: &[&str]) -> bool {
        roles.iter().all(|r| self.has_role(r))
    }

    /// 是否拥有指定权限
    ///
    /// 若 claims 中包含 `*` 或 `*:*:*` 通配权限，则匹配任意权限。
    pub fn has_permission(&self, permission: &str) -> bool {
        self.is_super_admin() || self.permissions.iter().any(|p| p == permission)
    }

    /// 是否拥有任一权限
    pub fn has_any_permission(&self, permissions: &[&str]) -> bool {
        self.is_super_admin() || permissions.iter().any(|p| self.has_permission(p))
    }

    /// 是否为超级管理员（拥有 `*` 或 `*:*:*` 通配权限）
    pub fn is_super_admin(&self) -> bool {
        self.permissions.iter().any(|p| p == "*" || p == "*:*:*")
    }

    // ── 自定义声明（extra） ──

    /// 读取自定义声明的原始 JSON 值。
    pub fn extra_claim(&self, key: &str) -> Option<&serde_json::Value> {
        self.extra.get(key)
    }

    /// 以字符串读取自定义声明。
    ///
    /// 数字与布尔也会被转换为字符串形式（便于与 `X-Tenant-Code` 等
    /// 以文本传递的标识比对）。
    pub fn extra_str(&self, key: &str) -> Option<String> {
        match self.extra.get(key)? {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    }

    /// 以整数读取自定义声明（兼容 JSON 整数与可无损转换的数字字符串）。
    pub fn extra_i64(&self, key: &str) -> Option<i64> {
        match self.extra.get(key)? {
            serde_json::Value::Number(n) => n
                .as_i64()
                .or_else(|| n.as_u64().and_then(|v| i64::try_from(v).ok())),
            serde_json::Value::String(s) => s.parse::<i64>().ok(),
            _ => None,
        }
    }

    /// 是否存在指定自定义声明。
    pub fn has_extra(&self, key: &str) -> bool {
        self.extra.contains_key(key)
    }
}

/// `TokenClaims` 的构建器——用于签发携带**自定义声明**的 Token。
///
/// 固定字段（`jti` / `exp` / `iat`）由 [`ClaimsBuilder::build`] 自动填充，
/// 业务只需声明自己的语义。自定义声明经 [`claim`](Self::claim) 加入，
/// 序列化后与固定字段**平铺在 JSON 顶层**。
///
/// # 示例
///
/// ```ignore
/// use alun::web::{ClaimsBuilder, JWT};
///
/// let jwt = JWT::from_config();
/// let token = jwt.issue(
///     ClaimsBuilder::new("u-1")
///         .username("alice")
///         .roles(&["admin".to_string()])
///         .claim("tenant_id", serde_json::json!(42))
///         .claim("session_ver", serde_json::json!(3)),
/// )?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct ClaimsBuilder {
    /// 用户 ID（`sub`）
    user_id: String,
    /// 用户名
    username: Option<String>,
    /// 角色列表
    roles: Vec<String>,
    /// 权限列表
    permissions: Vec<String>,
    /// 自定义声明
    extra: std::collections::HashMap<String, serde_json::Value>,
}

impl ClaimsBuilder {
    /// 以用户 ID 创建构建器（唯一必填项）。
    pub fn new(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            ..Default::default()
        }
    }

    /// 设置用户名。
    pub fn username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// 设置角色列表（覆盖式）。
    pub fn roles(mut self, roles: &[String]) -> Self {
        self.roles = roles.to_vec();
        self
    }

    /// 设置权限列表（覆盖式）。
    pub fn permissions(mut self, permissions: &[String]) -> Self {
        self.permissions = permissions.to_vec();
        self
    }

    /// 追加一个自定义声明（任意 JSON 值）。
    ///
    /// 同名声明后者覆盖前者。
    pub fn claim(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// 追加一个字符串自定义声明。
    pub fn claim_str(self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.claim(key, serde_json::Value::String(value.into()))
    }

    /// 追加一个整数自定义声明。
    pub fn claim_i64(self, key: impl Into<String>, value: i64) -> Self {
        self.claim(key, serde_json::json!(value))
    }

    /// 批量设置自定义声明。
    pub fn claims(mut self, extra: impl IntoIterator<Item = (String, serde_json::Value)>) -> Self {
        self.extra.extend(extra);
        self
    }

    /// 当前已声明的自定义声明（只读）。
    pub fn extra(&self) -> &std::collections::HashMap<String, serde_json::Value> {
        &self.extra
    }

    /// 补齐 `jti` / `exp` / `iat` 后产出最终载荷。
    ///
    /// - `token_type`：`Access` 或 `Refresh`
    /// - `expire_secs`：有效期秒数（自当前时刻起算）
    pub fn build(self, token_type: TokenType, expire_secs: u64) -> TokenClaims {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as usize)
            .unwrap_or(0);
        TokenClaims {
            jti: Some(uuid::Uuid::new_v4().to_string()),
            sub: self.user_id,
            username: self.username,
            roles: self.roles,
            permissions: self.permissions,
            token_type: Some(token_type),
            exp: now + expire_secs as usize,
            iat: now,
            extra: self.extra,
        }
    }
}

/// 已认证用户的 ID（从 JWT Token 中提取）
///
/// 持有的是 JWT `sub` 的**原样字符串**——ID 的形态（UUID / 自增 / 复合键 / IdP 主体）
/// 由业务决定，框架不做转换也不做校验。需要强类型时用 [`UserId::parse`]。
#[derive(Clone, Debug)]
pub struct UserId(pub String);

impl UserId {
    /// 以字符串形式读取 ID。
    ///
    /// 任何主键形态下都可用，是日志、审计、跨系统透传的稳妥选择。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 把 ID 解析成业务真正使用的类型；解析失败返回 `None`。
    ///
    /// 在**调用点**声明 ID 类型，框架便无需对 `sub` 的形态做任何假设：
    ///
    /// ```ignore
    /// let uuid = p.user_id.parse::<uuid::Uuid>();   // UUID 主键
    /// let id   = p.user_id.parse::<i64>();          // 自增 / 雪花整型主键
    /// let id   = p.user_id.parse::<String>();       // 恒成功（恒得 Some）
    /// ```
    ///
    /// 复合键（如 `u7:t1`）请自行拆解——作用域语法属业务约定，框架不定义。
    pub fn parse<T>(&self) -> Option<T>
    where
        T: std::str::FromStr,
    {
        self.0.parse().ok()
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for UserId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// JWT 认证声明（挂载在 `request.extensions` 中）
///
/// 两种取法都可用，且**不依赖所在层的位置**：
///
/// | 取法 | 说明 |
/// |---|---|
/// | handler 参数 `claims: AuthClaims` | 推荐。标准提取器，见下方「提取器」 |
/// | `Extension<AuthClaims>` | 兼容既有代码；仅在 `AuthLayer` 已执行时可用 |
///
/// 需要结构化的用户视图（`user_id`、角色列表）请用
/// [`Principal`](crate::middleware::Principal)。
#[derive(Clone, Debug)]
pub struct AuthClaims(pub TokenClaims);

/// `AuthClaims` 作为提取器——采用**双通道**读取身份，使「读身份」与中间件位置彻底解耦。
///
/// | 通道 | 触发条件 | 开销 |
/// |---|---|---|
/// | 1 | 请求扩展中已有 `AuthClaims`（`AuthLayer` 已执行，绝大多数情况） | 一次克隆，零验签 |
/// | 2 | 扩展中没有：自定义层位于最外层、路由未挂认证层、认证层整体关闭 | 自行验签 + 黑名单校验 |
///
/// 这解决了 `Extension<AuthClaims>` 的两个硬伤：
/// ① 在 `#[alun::no_auth]` 路由（或 `[middleware.auth] enabled = false`）上**提取失败直接 500**；
/// ② 只有位于 `AuthLayer` 内侧的层才拿得到身份。
///
/// # 与 `AuthLayer` 的分工
///
/// 提取器只**读取**身份，**不参与放行决策**——是否放行始终由 `AuthLayer` 决定。
/// 通道 2 也**不复现** `ignore_paths`：那是认证层的拒绝策略，与「我要身份」无关。
/// 唯一必须与 `AuthLayer` 保持一致的语义是**黑名单**（已登出的凭证不是有效身份），
/// 否则审计日志会把操作人错误归因。
///
/// 通道 2 使用 `[middleware.auth].jwt_secret` 验签——若你手工构造了使用其它密钥的
/// `AuthLayer`，请确保请求经过该层（走通道 1）。
///
/// # 示例
///
/// ```ignore
/// use alun::prelude::*;
///
/// #[alun::get("/me")]
/// async fn me(claims: AuthClaims) -> Res<String> {
///     Res::ok(claims.0.sub)
/// }
/// ```
impl<S> FromRequestParts<S> for AuthClaims
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // ── 通道 1：AuthLayer 已注入（零验签开销） ──
        if let Some(claims) = parts.extensions.get::<AuthClaims>() {
            return Ok(claims.clone());
        }

        // ── 通道 2：回退验签 ──
        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or_else(|| ApiError::unauthorized("未授权访问，请先登录"))?
            .to_string();

        let secret = crate::resources::try_config()
            .map(|cm| cm.get().middleware.auth.jwt_secret.clone())
            .ok_or_else(|| ApiError::internal("配置未初始化，无法解析身份"))?;

        let claims = auth::validate_and_extract_claims(&secret, &token)
            .map_err(|_| ApiError::unauthorized("Token 无效或已过期"))?;

        // 与 AuthLayer 保持一致：已登出（黑名单）的凭证不是有效身份
        #[cfg(feature = "cache")]
        if let (Some(cache), Some(jti)) = (crate::resources::try_cache(), claims.jti.as_deref()) {
            let key = format!("token:blacklist:{jti}");
            if let Ok(Some(_)) = alun_cache::Cache::get::<serde_json::Value>(cache, &key).await {
                return Err(ApiError::unauthorized("Token 已登出，请重新登录"));
            }
        }

        // 写回扩展：下游的提取器与 `Extension<AuthClaims>` 命中通道 1，避免重复验签
        let claims = AuthClaims(claims);
        parts.extensions.insert(claims.clone());
        Ok(claims)
    }
}

/// 可选认证：handler 参数写 `Option<AuthClaims>` 即可。
///
/// 「无凭证」与「凭证无效 / 已过期」**一律**返回 `Ok(None)`，不产生错误响应——
/// 这正是「可选登录」所需语义（同一接口区分登录态与游客态）。
///
/// # 不要用它做放行决策
///
/// `Option<T>` 会吞掉一切拒绝，把本应 401 的请求变成 `None`。需要「未登录即拒绝」时
/// 请直接用 `AuthClaims`。
///
/// # 示例
///
/// ```ignore
/// #[alun::no_auth("/articles")]
/// #[alun::get("/articles")]
/// async fn list(auth: Option<AuthClaims>) -> Res<String> {
///     let viewer = auth.map(|c| c.0.sub).unwrap_or_else(|| "guest".to_string());
///     Res::ok(viewer)
/// }
/// ```
impl<S> OptionalFromRequestParts<S> for AuthClaims
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(
            <AuthClaims as FromRequestParts<S>>::from_request_parts(parts, state)
                .await
                .ok(),
        )
    }
}

#[cfg(test)]
mod claims_tests {
    use super::*;

    /// 自定义声明必须**平铺在 JSON 顶层**（与固定字段同级）。
    ///
    /// 这是租户中间件等既有「按顶层字段名读取载荷」的代码能零改动
    /// 继续工作的前提。
    #[test]
    fn extra_claims_are_flattened_to_top_level() {
        let claims = ClaimsBuilder::new("u-1")
            .username("alice")
            .claim("tenant_id", serde_json::json!(42))
            .claim_str("locale", "zh-CN")
            .build(TokenType::Access, 3600);

        let json = serde_json::to_value(&claims).expect("序列化");

        assert_eq!(json["sub"], "u-1");
        assert_eq!(json["tenant_id"], 42, "自定义声明应与固定字段同级");
        assert_eq!(json["locale"], "zh-CN");
        // 不应出现嵌套的 "extra" 键
        assert!(json.get("extra").is_none(), "extra 不应作为嵌套对象出现");
    }

    /// 旧 Token（无任何自定义声明）必须仍可解析——向后兼容。
    #[test]
    fn legacy_payload_without_extra_still_deserializes() {
        let legacy = serde_json::json!({
            "jti": "j-1",
            "sub": "u-1",
            "username": "alice",
            "roles": ["admin"],
            "permissions": ["user:read"],
            "token_type": "access",
            "exp": 4_000_000_000usize,
            "iat": 1_700_000_000usize,
        });

        let claims: TokenClaims = serde_json::from_value(legacy).expect("旧载荷应可解析");
        assert_eq!(claims.sub, "u-1");
        assert!(claims.extra.is_empty(), "无自定义声明时 extra 应为空");
        assert!(claims.has_role("admin"));
    }

    /// 载荷里出现固定字段之外的键 → 自动落入 `extra`。
    #[test]
    fn unknown_fields_land_in_extra() {
        let payload = serde_json::json!({
            "sub": "u-1",
            "exp": 4_000_000_000usize,
            "tenant_id": 7,
            "org_code": "acme",
            "session_ver": 3,
        });

        let claims: TokenClaims = serde_json::from_value(payload).expect("解析");
        assert!(claims.extra.contains_key("tenant_id"));
        assert_eq!(claims.extra_i64("tenant_id"), Some(7));
        assert_eq!(claims.extra_str("org_code").as_deref(), Some("acme"));
        assert_eq!(claims.extra_i64("session_ver"), Some(3));
        assert_eq!(claims.extra_str("tenant_id").as_deref(), Some("7"));
        assert_eq!(claims.extra_i64("missing"), None);
        assert!(!claims.has_extra("missing"));
    }

    /// 往返：序列化 → 反序列化，自定义声明无损。
    #[test]
    fn extra_claims_survive_roundtrip() {
        let original = ClaimsBuilder::new("u-9")
            .roles(&["ops".to_string()])
            .claim_i64("tenant_id", 1234)
            .build(TokenType::Refresh, 7200);

        let text = serde_json::to_string(&original).expect("序列化");
        let parsed: TokenClaims = serde_json::from_str(&text).expect("反序列化");

        assert_eq!(parsed.sub, "u-9");
        assert_eq!(parsed.extra_i64("tenant_id"), Some(1234));
        assert_eq!(parsed.token_type, Some(TokenType::Refresh));
    }
}

//! 当前操作者的**结构化身份视图** —— [`Principal`]
//!
//! 框架已经把身份的整条链路都做完了（JWT 签发、验签、claims 注入、租户、权限校验、限流），
//! 链条上唯一的缺口是：wire format 里的 `sub` 是 `String`、角色是 `Vec<String>`，
//! 而业务侧每一次都要自己解析 `claims.sub` 并遍历 `claims.roles`。
//! 每个基于 alun 的业务仓都会把这个转换重写一遍——`Principal` 就是把这段重复收回到框架。
//!
//! # 设计边界一：`user_id` 不做形态假设
//!
//! `sub` 里装什么**由业务的主键决定**，框架无从假设，而现实中至少并存这几种：
//!
//! | 主键形态 | `sub` 实例 |
//! |---|---|
//! | UUID | `22222222-2222-2222-2222-222222222222` |
//! | 自增 / 雪花整型 | `1024` |
//! | 复合作用域键 | `u7:t1`（本仓库 `alun-ledger` 的约定） |
//! | 外部 IdP 主体 | `auth0\|5f3c…`、微信 `openid` |
//! | 手机号 / 工号 | `13800000000` |
//!
//! 若把 `user_id` 定成 `Option<Uuid>`，上表后四种会**统一退化为 `None`**：调用方拿不到身份，
//! 只能回退去自己解析 `sub`——「消除重复」的初衷被原样还了回去，而且多了一个
//! 「解析失败」的隐式分支（字段是 `Option`，但 `sub` 本身从不缺失，二者语义不匹配）。
//!
//! 因此 `user_id` 的类型是 [`UserId`]——**原样字符串标识，永远可用**；
//! 需要强类型时在**调用点**声明你要的类型：`p.user_id.parse::<Uuid>()`
//! （见 [`UserId::parse`]）。ID 类型是调用点的知识，不是身份类型自身的知识。
//!
//! 同理**不**把 `Principal` 泛型化（`Principal<Uuid>`）：泛型参数会传染到提取器，
//! 使 `Option<Principal>` 无法类型推导，且等于强迫整个应用只能有一种 ID 约定。
//!
//! # 设计边界二：只搬运事实，不裁决权限
//!
//! `Principal` **只做机械映射**：把载荷里的字段规范化成领域类型，不做任何授权判断。
//!
//! 特别地，框架**不**内置 `is_super_admin`：超管的判定依据（角色编码？通配权限码？
//! 某个配置项？）因系统而异，内置只会制造第二个真值源，让「谁是超管」出现分歧。
//! 业务侧写一行即可：
//!
//! ```ignore
//! // 以角色编码为唯一真值源
//! let is_super = principal.has_role("SUPER_ADMIN");
//! ```
//!
//! 完整载荷始终可通过 [`Principal::claims`] 取到，不丢失任何信息。

use std::sync::Arc;

use axum::extract::{FromRequestParts, OptionalFromRequestParts};
use axum::http::request::Parts;

use super::{AuthClaims, TokenClaims, UserId};
use crate::api::ApiError;

/// 当前操作者的结构化视图（handler 参数可直接提取）。
///
/// 与 [`AuthClaims`] 的分工：`AuthClaims` 是**原始载荷**（`sub: String` + 全部自定义声明），
/// `Principal` 是**业务视图**（`user_id: UserId` + 角色列表 + 强类型解析入口）。
/// 两者读的是同一份身份，取其一即可；需要自定义声明时用 [`Principal::claims`]。
///
/// # 提取方式
///
/// | 写法 | 语义 |
/// |---|---|
/// | `p: Principal` | 必须有有效身份，否则 401 |
/// | `p: Option<Principal>` | 可选登录，缺失即 `None` |
///
/// 两者都复用 [`AuthClaims`] 的双通道提取，因此**不受中间件位置影响**。
///
/// # 示例
///
/// ```ignore
/// use alun::prelude::*;
///
/// #[alun::get("/me")]
/// async fn me(p: Principal) -> Res<String> {
///     // 原样标识：任何主键形态都能取到（推荐用于日志、审计、透传）
///     let id = p.user_id.as_str();
///
///     // 需要强类型时在调用点声明（UUID 主键场景）
///     let uuid = p.user_id.parse::<uuid::Uuid>();
///
///     Res::ok(format!("{id} / {uuid:?}"))
/// }
/// ```
#[derive(Clone, Debug)]
pub struct Principal {
    /// 用户 ID —— JWT `sub` 的**原样标识**，任何主键形态下都可用、不丢失。
    ///
    /// 框架不对它做形态假设（UUID / 整型 / 复合键 / IdP 主体皆可）；
    /// 需要强类型时用 [`UserId::parse`]，需要 `&str` 时用 [`UserId::as_str`]。
    pub user_id: UserId,
    /// 用户名（JWT `username` 声明，可能缺失）
    pub username: Option<String>,
    /// 角色编码列表（**语义由业务定义**，框架只搬运）
    pub roles: Vec<String>,
    /// 原始声明 —— 逃生口：`permissions`、`jti`、`exp`、**自定义声明**（`extra`）都在这里
    pub claims: Arc<TokenClaims>,
}

impl Principal {
    /// 由原始声明构造视图（纯机械映射，不失败、不裁决）。
    pub fn from_claims(claims: TokenClaims) -> Self {
        Self {
            user_id: UserId(claims.sub.clone()),
            username: claims.username.clone(),
            roles: claims.roles.clone(),
            claims: Arc::new(claims),
        }
    }

    /// 是否拥有指定角色。
    pub fn has_role(&self, role: &str) -> bool {
        self.claims.has_role(role)
    }

    /// 是否拥有任一角色。
    pub fn has_any_role(&self, roles: &[&str]) -> bool {
        self.claims.has_any_role(roles)
    }

    /// 是否拥有指定权限。
    ///
    /// **透传** [`TokenClaims::has_permission`]：`permissions` 中含 `*` 或 `*:*:*` 时视为
    /// 拥有全部权限。若你的系统另外约定了一套判定依据（例如按角色编码），请以那套为准，
    /// **不要**与这里的通配语义混用——两套依据并存就会产生分歧。
    /// 需要严格按字面匹配时直接读 `principal.claims.permissions`。
    pub fn has_permission(&self, permission: &str) -> bool {
        self.claims.has_permission(permission)
    }

    /// 是否拥有任一权限（同上，含通配语义）。
    pub fn has_any_permission(&self, permissions: &[&str]) -> bool {
        self.claims.has_any_permission(permissions)
    }
}

/// `Principal` 提取器——复用 [`AuthClaims`] 的双通道（扩展优先、回退验签）。
impl<S> FromRequestParts<S> for Principal
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let AuthClaims(claims) =
            <AuthClaims as FromRequestParts<S>>::from_request_parts(parts, state).await?;
        Ok(Self::from_claims(claims))
    }
}

/// 可选身份：`Option<Principal>` —— 无凭证 / 凭证无效一律 `None`。
impl<S> OptionalFromRequestParts<S> for Principal
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        Ok(
            <Principal as FromRequestParts<S>>::from_request_parts(parts, state)
                .await
                .ok(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::{ClaimsBuilder, TokenType};

    fn claims(sub: &str) -> TokenClaims {
        ClaimsBuilder::new(sub)
            .username("alice")
            .roles(&["auditor".to_string()])
            .permissions(&["user:read".to_string()])
            .claim_i64("tenant_id", 42)
            .build(TokenType::Access, 3600)
    }

    /// 结构化视图只搬运事实：`sub` 原样可达、用户名、角色、原始载荷全部可达。
    #[test]
    fn maps_claims_to_view() {
        let id = uuid::Uuid::new_v4();
        let p = Principal::from_claims(claims(&id.to_string()));

        assert_eq!(p.user_id.as_str(), id.to_string());
        assert_eq!(
            p.user_id.parse::<uuid::Uuid>(),
            Some(id),
            "强类型在调用点解析"
        );
        assert_eq!(p.username.as_deref(), Some("alice"));
        assert_eq!(p.roles, vec!["auditor".to_string()]);
        assert!(p.has_role("auditor"));
        assert!(!p.has_role("SUPER_ADMIN"), "框架不假设任何超管判定依据");
        assert!(p.has_permission("user:read"));
        // 自定义声明经逃生口仍可取到
        assert_eq!(p.claims.extra_i64("tenant_id"), Some(42));
    }

    /// **回归测试**：非 UUID 的 `sub` 不再丢身份 —— 这正是放弃 `Option<Uuid>` 的原因。
    ///
    /// 旧实现下 `user_id` 为 `None`，调用方必须回退去读 `claims.sub`，
    /// 结构化视图对这类系统等于失效。
    #[test]
    fn non_uuid_sub_keeps_identity() {
        let p = Principal::from_claims(claims("legacy-user-7"));

        assert_eq!(p.user_id.as_str(), "legacy-user-7", "身份必须原样可用");
        assert_eq!(p.claims.sub, "legacy-user-7", "与原始载荷一致");
        assert_eq!(
            p.user_id.parse::<uuid::Uuid>(),
            None,
            "解析失败只是 None —— 身份不丢、也不报错"
        );
    }

    /// ID 类型由调用点决定：整型主键、复合作用域键都能各取所需。
    #[test]
    fn caller_decides_id_type() {
        let numeric = Principal::from_claims(claims("1024"));
        assert_eq!(numeric.user_id.parse::<i64>(), Some(1024));
        assert_eq!(numeric.user_id.parse::<String>().as_deref(), Some("1024"));

        // alun-ledger 式复合键：框架不定义作用域语法，交业务拆解
        let scoped = Principal::from_claims(claims("u7:t1"));
        assert_eq!(scoped.user_id.parse::<i64>(), None);
        let (user, tenant) = scoped.user_id.as_str().split_once(':').expect("u7:t1");
        assert_eq!((user, tenant), ("u7", "t1"));
    }
}

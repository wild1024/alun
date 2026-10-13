//! 租户与用户端点。

use alun::{ApiError, AuthClaims, Res, db};
use alun_web::api::{PageData, PageQuery};
use alun_db::prelude::CrudExt;
use alun_db::prelude::*;
use axum::Extension;
use axum::extract::{Json, Path};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::api::{ApiResult, identity_of};
use crate::models::{Tenant, User};
use crate::services::error::map_db_err;

/// 创建租户请求。
#[derive(Debug, Deserialize)]
pub struct CreateTenantReq {
    /// 主键（显式指定，保证可复现）。
    pub id: i64,
    /// 名称。
    pub name: String,
    /// 套餐。
    pub plan: String,
    /// 区域。
    pub region: Option<String>,
}

/// 创建租户。
#[alun::post("/api/tenants")]
pub async fn create_tenant(Json(req): Json<CreateTenantReq>) -> ApiResult<Res<Tenant>> {
    let now = Utc::now();
    let mut tenant = Tenant {
        id: req.id,
        name: req.name,
        plan: req.plan,
        status: Tenant::ACTIVE.to_string(),
        created_at: now,
        updated_at: now,
        region: req.region.unwrap_or_else(|| "cn".to_string()),
    };
    tenant
        .insert_or_update(db())
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(tenant))
}

/// 查询租户。
#[alun::get("/api/tenants/{id}")]
pub async fn get_tenant(Path(id): Path<i64>) -> ApiResult<Res<Tenant>> {
    let tenant = Tenant::find(db(), id).await.map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(tenant))
}

/// 创建用户请求。
#[derive(Debug, Deserialize)]
pub struct CreateUserReq {
    /// 主键。
    pub id: i64,
    /// 邮箱（唯一）。
    pub email: String,
    /// 姓名。
    pub name: String,
}

/// 创建用户（写入当前 JWT 所属租户）。
#[alun::post("/api/users")]
pub async fn create_user(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    Json(req): Json<CreateUserReq>,
) -> ApiResult<Res<User>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let now = Utc::now();
    let (first, last) = split_name(&req.name);
    let mut user = User {
        id: req.id,
        tenant_id,
        email: req.email,
        name: req.name,
        status: User::ACTIVE.to_string(),
        role: User::ROLE_MEMBER.to_string(),
        created_at: now,
        updated_at: now,
        first_name: first,
        last_name: last,
        profile: None,
        locale: "zh-CN".to_string(),
    };
    user.insert_or_update(db())
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(user))
}

/// 查询用户（限本租户，避免跨租户 IDOR）。
#[alun::get("/api/users/{id}")]
pub async fn get_user(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    Path(id): Path<i64>,
) -> ApiResult<Res<User>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let users = crate::services::tenant::list_users(db(), tenant_id)
        .await
        .map_err(|e| map_db_err(&e))?;
    let user = users
        .into_iter()
        .find(|u| u.id == id)
        .ok_or_else(|| ApiError::not_found("用户不存在"))?;
    Ok(Res::ok(user))
}

/// 分页响应体。
#[derive(Debug, Serialize)]
pub struct UserListPage {
    /// 列表。
    pub list: Vec<User>,
    /// 总数。
    pub total: u64,
    /// 页码。
    pub page: u64,
    /// 每页大小。
    pub page_size: u64,
}

/// 分页查询用户。
#[alun::get("/api/users")]
pub async fn list_users(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    axum::extract::Query(pq): axum::extract::Query<PageQuery>,
) -> ApiResult<Res<PageData<Vec<User>>>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let all = crate::services::tenant::list_users(db(), tenant_id)
        .await
        .map_err(|e| map_db_err(&e))?;
    let total = all.len() as u64;
    let skip = ((pq.page - 1) * pq.page_size) as usize;
    let list: Vec<User> = all
        .into_iter()
        .skip(skip)
        .take(pq.page_size as usize)
        .collect();
    Ok(Res::page(list, total, pq.page, pq.page_size))
}

fn split_name(name: &str) -> (String, String) {
    match name.split_once(' ') {
        Some((first, last)) => (first.to_string(), last.to_string()),
        None => (name.to_string(), String::new()),
    }
}

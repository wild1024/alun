//! 订阅、报表、审计与运维端点。

use alun::{ApiError, AuthClaims, Res, db};
use alun_web::api::{PageData, PageQuery};
use alun_db::prelude::*;
use axum::Extension;
use axum::extract::{Json, Path};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::api::{ApiResult, identity_of};
use crate::models::{Subscription, SubscriptionStatus};
use crate::services::{billing, error::map_db_err, report, tenant as tenant_svc};

/// 创建订阅请求。
#[derive(Debug, Deserialize)]
pub struct CreateSubscriptionReq {
    /// 主键。
    pub id: i64,
    /// 客户 id。
    pub customer_id: i64,
    /// 套餐。
    pub plan: String,
    /// 周期开始。
    pub period_start: DateTime<Utc>,
    /// 周期结束。
    pub period_end: DateTime<Utc>,
}

/// 创建订阅。
#[alun::post("/api/subscriptions")]
pub async fn create_subscription(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    Json(req): Json<CreateSubscriptionReq>,
) -> ApiResult<Res<Subscription>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let mut sub = Subscription {
        id: req.id,
        tenant_id,
        customer_id: req.customer_id,
        plan: req.plan,
        period_start: req.period_start,
        period_end: req.period_end,
        status: SubscriptionStatus::Active,
    };
    sub.insert_or_update(db())
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(sub))
}

/// MRR 报表行。
#[derive(Debug, serde::Serialize)]
pub struct MrrRow {
    /// 月份 `YYYY-MM`。
    pub month: String,
    /// 当月收入（规范化字符串）。
    pub mrr: String,
    /// 累计收入。
    pub cumulative: String,
}

/// MRR 趋势报表（CTE + Rust 侧累计，替代窗口函数）。
#[alun::get("/api/reports/mrr")]
pub async fn mrr(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
) -> ApiResult<Res<Vec<MrrRow>>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let rows = report::mrr_by_month(db(), tenant_id)
        .await
        .map_err(|e| map_db_err(&e))?;
    let cum = report::cumulative(&rows);
    let out = cum
        .into_iter()
        .map(|(month, running)| {
            let mrr = rows
                .iter()
                .find(|(m, _)| *m == month)
                .map(|(_, v)| crate::services::money::money_str(*v))
                .unwrap_or_else(|| "0".to_string());
            MrrRow {
                month,
                mrr,
                cumulative: crate::services::money::money_str(running),
            }
        })
        .collect();
    Ok(Res::ok(out))
}

/// 审计日志分页查询（本租户 + 可选实体过滤）。
#[alun::get("/api/audit-logs")]
pub async fn list_audit_logs(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    axum::extract::Query(pq): axum::extract::Query<PageQuery>,
) -> ApiResult<Res<PageData<Vec<crate::models::AuditLog>>>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let rows = db()
        .find_with_params(
            "SELECT * FROM audit_logs WHERE tenant_id = ? ORDER BY id LIMIT ? OFFSET ?",
            vec![
                alun_db::prelude::Value::I64(tenant_id),
                alun_db::prelude::Value::I64(pq.page_size as i64),
                alun_db::prelude::Value::I64(((pq.page - 1) * pq.page_size) as i64),
            ],
        )
        .await
        .map_err(|e| map_db_err(&e))?;
    let total = db()
        .scalar_with_params::<i64>(
            "SELECT COUNT(*) FROM audit_logs WHERE tenant_id = ?",
            vec![alun_db::prelude::Value::I64(tenant_id)],
        )
        .await
        .map_err(|e| map_db_err(&e))? as u64;
    let list = rows
        .into_iter()
        .map(crate::models::AuditLog::from_row)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::page(list, total, pq.page, pq.page_size))
}

/// 到期订阅扫描（报表/定时任务用）。
#[alun::get("/api/subscriptions/due")]
pub async fn due_subscriptions() -> ApiResult<Res<Vec<Subscription>>> {
    let subs = billing::due_subscriptions(db(), Utc::now())
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(subs))
}

/// 租户内订阅列表。
#[alun::get("/api/subscriptions")]
pub async fn list_subscriptions(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
) -> ApiResult<Res<Vec<Subscription>>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let subs = tenant_svc::list_subscriptions(db(), tenant_id)
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(subs))
}

/// 账户 id 列表（调试用，非需求端点）。
#[alun::get("/api/accounts")]
pub async fn list_accounts(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
) -> ApiResult<Res<Vec<i64>>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let ids = tenant_svc::account_ids(db(), tenant_id)
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(ids))
}

/// 健康检查（框架不内置，项目侧实现；含 DB 探测）。
#[alun::get("/health")]
pub async fn health() -> ApiResult<Res<serde_json::Value>> {
    match db().scalar::<i64>("SELECT 1").await {
        Ok(_) => Ok(Res::ok(serde_json::json!({"status": "ok"}))),
        Err(e) => Err(ApiError::service_unavailable(format!("db unhealthy: {e}"))),
    }
}

/// 管理端点：需要 `tenant:admin` 权限（403 用例 API-008）。
#[alun::get("/api/admin/tenants")]
#[alun::permission(
    path = "/api/admin/tenants",
    method = "GET",
    permission = "tenant:admin"
)]
pub async fn admin_tenants() -> ApiResult<Res<Vec<crate::models::Tenant>>> {
    let tenants = tenant_svc::admin_list_tenants(db())
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(tenants))
}

/// 当前身份（用于认证用例）。
#[alun::get("/api/me")]
pub async fn me(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<serde_json::Value> {
    let (user_id, tenant_id) = identity_of(&claims).unwrap_or((0, 0));
    Res::ok(serde_json::json!({
        "user_id": user_id,
        "tenant_id": tenant_id,
        "roles": claims.roles,
        "permissions": claims.permissions,
    }))
}

/// 读取实体（辅助用，非需求端点）。
#[alun::get("/api/invoices/{id}")]
pub async fn get_invoice(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    Path(id): Path<i64>,
) -> ApiResult<Res<crate::models::Invoice>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let inv = crate::services::invoice::load(db(), tenant_id, id)
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(inv))
}

//! 发票端点（状态机 + 分页）。

use alun::{ApiError, AuthClaims, Res, db};
use alun_web::api::{PageData, PageQuery};
use alun_db::prelude::*;
use axum::Extension;
use axum::extract::{Json, Path};
use chrono::Utc;
use serde::Deserialize;

use crate::api::{ApiResult, identity_of, parse_money};
use crate::models::{Invoice, InvoiceItem, InvoiceStatus};
use crate::services::{error::map_db_err, invoice as invoice_svc, tenant as tenant_svc};

/// 发票条目请求。
#[derive(Debug, Deserialize)]
pub struct ItemReq {
    /// 条目主键。
    pub id: i64,
    /// 描述。
    pub description: String,
    /// 数量。
    pub quantity: i64,
    /// 单价（字符串）。
    pub unit_price: String,
}

/// 创建发票请求。
#[derive(Debug, Deserialize)]
pub struct CreateInvoiceReq {
    /// 发票主键。
    pub id: i64,
    /// 客户 id。
    pub customer_id: i64,
    /// 元数据（JSON）。
    pub metadata: Option<serde_json::Value>,
    /// 条目。
    pub items: Vec<ItemReq>,
}

/// 创建草稿发票（服务端重算 total 并校验一致性）。
#[alun::post("/api/invoices")]
pub async fn create_invoice(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    Json(req): Json<CreateInvoiceReq>,
) -> ApiResult<Res<Invoice>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let mut items = Vec::with_capacity(req.items.len());
    for i in &req.items {
        let unit_price = parse_money(&i.unit_price)?;
        items.push(InvoiceItem {
            id: i.id,
            invoice_id: req.id,
            description: i.description.clone(),
            quantity: i.quantity,
            unit_price,
            amount: unit_price * rust_decimal::Decimal::from(i.quantity),
            tax_rate: "0".to_string(),
        });
    }
    let total = invoice_svc::items_total(&items);
    let invoice = Invoice {
        id: req.id,
        tenant_id,
        customer_id: req.customer_id,
        status: InvoiceStatus::Draft,
        total,
        metadata: req.metadata,
        issued_at: None,
        paid_at: None,
        created_at: Utc::now(),
        note: None,
    };
    let created = invoice_svc::create_draft(db(), invoice, &items, 0)
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(created))
}

/// 状态流转请求。
#[derive(Debug, Deserialize)]
pub struct StatusReq {
    /// 目标状态：`issued` / `paid` / `voided`。
    pub status: String,
}

/// 发票状态流转。
#[alun::patch("/api/invoices/{id}/status")]
pub async fn patch_status(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    Path(id): Path<i64>,
    Json(req): Json<StatusReq>,
) -> ApiResult<Res<Invoice>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let next = match req.status.as_str() {
        "issued" => InvoiceStatus::Issued,
        "paid" => InvoiceStatus::Paid,
        "voided" => InvoiceStatus::Voided,
        other => return Err(ApiError::bad_request(format!("未知状态 `{other}`"))),
    };
    let invoice = invoice_svc::transition(db(), tenant_id, id, next, Utc::now(), 0)
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(invoice))
}

/// 分页查询发票（本租户）。
#[alun::get("/api/invoices")]
pub async fn list_invoices(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    axum::extract::Query(pq): axum::extract::Query<PageQuery>,
) -> ApiResult<Res<PageData<Vec<Invoice>>>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let all = tenant_svc::list_invoices(db(), tenant_id)
        .await
        .map_err(|e| map_db_err(&e))?;
    let total = all.len() as u64;
    let skip = ((pq.page - 1) * pq.page_size) as usize;
    let list: Vec<Invoice> = all
        .into_iter()
        .skip(skip)
        .take(pq.page_size as usize)
        .collect();
    Ok(Res::page(list, total, pq.page, pq.page_size))
}

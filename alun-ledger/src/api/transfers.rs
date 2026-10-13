//! 转账与账务端点。

use alun::{AuthClaims, Res, db};
use alun_db::prelude::*;
use axum::Extension;
use axum::extract::{Json, Path};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::api::{ApiResult, identity_of, parse_money};
use crate::models::Transfer;
use crate::services::{error::map_db_err, ledger};

/// 转账请求。
#[derive(Debug, Deserialize)]
pub struct TransferReq {
    /// 转账单主键。
    pub transfer_id: i64,
    /// 借方分录主键。
    pub debit_entry_id: i64,
    /// 贷方分录主键。
    pub credit_entry_id: i64,
    /// 源账户。
    pub from_account: i64,
    /// 目标账户。
    pub to_account: i64,
    /// 金额（字符串，避免浮点）。
    pub amount: String,
}

/// 发起转账（事务内完成扣款 / 入账 / 分录 / 审计）。
#[alun::post("/api/transfers")]
pub async fn create_transfer(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    Json(req): Json<TransferReq>,
) -> ApiResult<Res<Transfer>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let cmd = ledger::TransferCommand {
        transfer_id: req.transfer_id,
        debit_entry_id: req.debit_entry_id,
        credit_entry_id: req.credit_entry_id,
        tenant_id,
        from_account: req.from_account,
        to_account: req.to_account,
        amount: parse_money(&req.amount)?,
        at: Utc::now(),
    };
    let transfer = ledger::post_transfer(db(), cmd)
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(transfer))
}

/// 查询转账（限本租户）。
#[alun::get("/api/transfers/{id}")]
pub async fn get_transfer(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    Path(id): Path<i64>,
) -> ApiResult<Res<Transfer>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let transfer = ledger::load(db(), tenant_id, id)
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(transfer))
}

/// 账户余额查询响应。
#[derive(Debug, serde::Serialize)]
pub struct BalanceRes {
    /// 账户 id。
    pub account_id: i64,
    /// 余额（规范化字符串）。
    pub balance: String,
    /// 查询时刻。
    pub at: DateTime<Utc>,
}

/// 查询账户余额（金额经方言无关解码，见缺陷 D1）。
#[alun::get("/api/accounts/{id}/balance")]
pub async fn get_balance(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    Path(id): Path<i64>,
) -> ApiResult<Res<BalanceRes>> {
    let (_, tenant_id) = identity_of(&claims)?;
    let balance = crate::services::money::account_balance(db(), tenant_id, id)
        .await
        .map_err(|e| map_db_err(&e))?;
    Ok(Res::ok(BalanceRes {
        account_id: id,
        balance: crate::services::money::money_str(balance),
        at: Utc::now(),
    }))
}

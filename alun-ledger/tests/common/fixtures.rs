//! 夹具：确定性种子数据（固定主键 + 显式时间戳）。
//!
//! 幂等：每个用例开始前按租户清理，再写入固定数据，保证重复运行结果一致。

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;

use crate::common::harness::LedgerCtx;

/// 固定时间基准（2026-01-01T00:00:00Z）。
pub fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

/// 时间基准 + `secs` 秒。
pub fn at(secs: i64) -> DateTime<Utc> {
    t0() + chrono::Duration::seconds(secs)
}

/// 金额快捷构造（分）。
pub fn money(yuan: i64, cents: i64) -> Decimal {
    Decimal::new(yuan * 100 + cents, 2)
}

/// 租户固定 id。
pub const TENANT_T1: i64 = 1;
/// 租户固定 id（空租户）。
pub const TENANT_T2: i64 = 2;
/// 租户固定 id（大量数据）。
pub const TENANT_T3: i64 = 3;

/// 清理指定租户的全部业务数据（幂等夹具的前置动作）。
pub async fn reset_tenant(db: &Db, tenant_id: i64) -> Result<()> {
    let tables = [
        (
            "invoice_items",
            "invoice_id IN (SELECT id FROM invoices WHERE tenant_id = ?)",
        ),
        ("invoices", "tenant_id = ?"),
        ("ledger_entries", "tenant_id = ?"),
        ("transfers", "tenant_id = ?"),
        ("accounts", "tenant_id = ?"),
        (
            "usage_records",
            "subscription_id IN (SELECT id FROM subscriptions WHERE tenant_id = ?)",
        ),
        ("subscriptions", "tenant_id = ?"),
        ("audit_logs", "tenant_id = ?"),
        ("users", "tenant_id = ?"),
        ("tenants", "id = ?"),
    ];
    for (table, cond) in tables {
        db.execute_with_params(
            &format!("DELETE FROM {table} WHERE {cond}"),
            vec![Value::I64(tenant_id)],
        )
        .await?;
    }
    Ok(())
}

/// 种入一个租户。
pub async fn seed_tenant(ctx: &LedgerCtx, tenant_id: i64, name: &str) -> Result<()> {
    reset_tenant(ctx.db(), tenant_id).await?;
    ctx.db()
        .execute_with_params(
            "INSERT INTO tenants (id, name, plan, status, created_at, updated_at, region) VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(tenant_id),
                Value::Text(name.to_string()),
                Value::Text("pro".into()),
                Value::Text("active".into()),
                at(0).into(),
                at(0).into(),
                Value::Text("cn".into()),
            ],
        )
        .await?;
    Ok(())
}

/// 账户：每租户 5 个（1001/2001/3001/4001/5001）。
pub const ACCOUNT_CODES: [(&str, &str); 5] = [
    ("1001", "asset"),
    ("2001", "liability"),
    ("3001", "equity"),
    ("4001", "revenue"),
    ("5001", "expense"),
];

/// 账户 id 规则：`tenant_id * 1000 + n`（n = 1..5）。
pub fn account_id(tenant_id: i64, n: i64) -> i64 {
    tenant_id * 1000 + n
}

/// 种入 5 个账户（余额 `balance` 落在第一个账户）。
pub async fn seed_accounts(ctx: &LedgerCtx, tenant_id: i64, balance: Decimal) -> Result<()> {
    for (i, (code, kind)) in ACCOUNT_CODES.iter().enumerate() {
        let id = account_id(tenant_id, (i + 1) as i64);
        let amount = if i == 0 { balance } else { Decimal::ZERO };
        ctx.db()
            .execute_with_params(
                "INSERT INTO accounts (id, tenant_id, code, name, kind, balance, currency) VALUES (?, ?, ?, ?, ?, ?, ?)",
                vec![
                    Value::I64(id),
                    Value::I64(tenant_id),
                    Value::Text((*code).to_string()),
                    Value::Text(format!("account-{code}")),
                    Value::Text((*kind).to_string()),
                    alun_db_core::traits::TypeMapper::to_value(&amount),
                    Value::Text("CNY".into()),
                ],
            )
            .await?;
    }
    Ok(())
}

/// 种入一个用户。
pub async fn seed_user(ctx: &LedgerCtx, tenant_id: i64, user_id: i64, email: &str) -> Result<()> {
    ctx.db()
        .execute_with_params(
            "INSERT INTO users (id, tenant_id, email, name, status, role, created_at, updated_at, first_name, last_name, profile, locale) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(user_id),
                Value::I64(tenant_id),
                Value::Text(email.to_string()),
                Value::Text("Alice Zhang".into()),
                Value::Text("active".into()),
                Value::Text("member".into()),
                at(0).into(),
                at(0).into(),
                Value::Text("Alice".into()),
                Value::Text("Zhang".into()),
                Value::Null,
                Value::Text("zh-CN".into()),
            ],
        )
        .await?;
    Ok(())
}

/// 种入一张草稿发票 + 若干条目（金额由 Rust 计算）。
pub async fn seed_invoice(
    ctx: &LedgerCtx,
    tenant_id: i64,
    invoice_id: i64,
    status: &str,
) -> Result<()> {
    let item_amount = money(10, 0);
    ctx.db()
        .execute_with_params(
            "INSERT INTO invoices (id, tenant_id, customer_id, status, total, metadata, issued_at, paid_at, created_at, note) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(invoice_id),
                Value::I64(tenant_id),
                Value::I64(tenant_id * 100 + 7),
                Value::Text(status.to_string()),
                alun_db_core::traits::TypeMapper::to_value(&item_amount),
                Value::Json(serde_json::json!({"tier": "gold", "tax_id": "CN-91310000"}).to_string()),
                Value::Null,
                Value::Null,
                at(0).into(),
                Value::Null,
            ],
        )
        .await?;
    ctx.db()
        .execute_with_params(
            "INSERT INTO invoice_items (id, invoice_id, description, quantity, unit_price, amount, tax_rate) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(invoice_id * 10 + 1),
                Value::I64(invoice_id),
                Value::Text("subscription fee".into()),
                Value::I64(1),
                alun_db_core::traits::TypeMapper::to_value(&item_amount),
                alun_db_core::traits::TypeMapper::to_value(&item_amount),
                Value::Text("0".into()),
            ],
        )
        .await?;
    Ok(())
}

/// 种入订阅。
pub async fn seed_subscription(
    ctx: &LedgerCtx,
    tenant_id: i64,
    sub_id: i64,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    status: &str,
) -> Result<()> {
    ctx.db()
        .execute_with_params(
            "INSERT INTO subscriptions (id, tenant_id, customer_id, plan, period_start, period_end, status) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(sub_id),
                Value::I64(tenant_id),
                Value::I64(tenant_id * 100 + 7),
                Value::Text("pro".into()),
                period_start.into(),
                period_end.into(),
                Value::Text(status.to_string()),
            ],
        )
        .await?;
    Ok(())
}

/// 种入用量记录。
pub async fn seed_usage(
    ctx: &LedgerCtx,
    sub_id: i64,
    usage_id: i64,
    at_time: DateTime<Utc>,
    quantity: i64,
) -> Result<()> {
    ctx.db()
        .execute_with_params(
            "INSERT INTO usage_records (id, subscription_id, usage_date, quantity, metadata) VALUES (?, ?, ?, ?, ?)",
            vec![
                Value::I64(usage_id),
                Value::I64(sub_id),
                at_time.into(),
                Value::I64(quantity),
                Value::Json(serde_json::json!({"unit": "call"}).to_string()),
            ],
        )
        .await?;
    Ok(())
}

/// 常见账户 id 助手（T1 的源/目标账户）。
pub fn t1_asset() -> i64 {
    account_id(TENANT_T1, 1)
}
/// T1 的第二个账户。
pub fn t1_liability() -> i64 {
    account_id(TENANT_T1, 2)
}

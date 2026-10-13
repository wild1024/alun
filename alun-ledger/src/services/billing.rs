//! 订阅计费（用例 SUB-001..008、REC-001）。

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use alun_db_core::traits::TypeMapper;
use chrono::{DateTime, Datelike, TimeZone, Utc};
use rust_decimal::Decimal;

use crate::models::{Invoice, Subscription, SubscriptionStatus, UsageRecord};
use crate::services::invoice as invoice_svc;
use crate::services::rowx;

/// 用量聚合结果。
#[derive(Debug, Clone, PartialEq)]
pub struct UsageAgg {
    /// 计费天数（`COUNT(DISTINCT usage_date)`）。
    pub days: i64,
    /// 总用量（`SUM(quantity)`）。
    pub total: Decimal,
}

/// 到期订阅扫描：`status = 'active' AND period_end <= now`。
///
/// 边界为闭区间：`period_end == now` 命中（`<=`），`now` 之前 1 微秒不命中。
pub async fn due_subscriptions(db: &Db, now: DateTime<Utc>) -> Result<Vec<Subscription>> {
    let rows = db
        .find_with_params(
            "SELECT * FROM subscriptions WHERE status = ? AND period_end <= ? ORDER BY id",
            vec![Value::Text("active".into()), now.to_value()],
        )
        .await?;
    rows.into_iter().map(Subscription::from_row).collect()
}

/// 用量聚合：`GROUP BY + HAVING + COUNT(DISTINCT)`（`sql!` 无 `#having`，走 raw SQL）。
pub async fn aggregate_usage(db: &Db, subscription_id: i64) -> Result<UsageAgg> {
    let rows = db
        .find_with_params(
            "SELECT COUNT(DISTINCT usage_date) AS days, SUM(quantity) AS total \
             FROM usage_records WHERE subscription_id = ? \
             GROUP BY subscription_id HAVING SUM(quantity) > 0",
            vec![Value::I64(subscription_id)],
        )
        .await?;
    match rows.first() {
        Some(row) => Ok(UsageAgg {
            days: rowx::i64_of(row, "days")?,
            total: rowx::decimal_of(row, "total")?,
        }),
        None => Ok(UsageAgg {
            days: 0,
            total: Decimal::ZERO,
        }),
    }
}

/// 批量开票：走 `insert_many`（多条值合并为单条 INSERT；**不触发 Hook、不回填自增主键**）。
pub async fn issue_invoices_batch(db: &Db, invoices: &[Invoice]) -> Result<()> {
    Invoice::insert_many(invoices, db).await
}

/// 幂等开票：同一（租户, 客户, 自然月）已存在则返回既有发票，否则插入。
pub async fn create_invoice_once(db: &Db, invoice: Invoice) -> Result<(Invoice, bool)> {
    let (month_start, next_month) = month_bounds(invoice.created_at);
    let existing = db
        .find_with_params(
            "SELECT id FROM invoices WHERE tenant_id = ? AND customer_id = ? AND created_at >= ? AND created_at < ?",
            vec![
                Value::I64(invoice.tenant_id),
                Value::I64(invoice.customer_id),
                month_start.to_value(),
                next_month.to_value(),
            ],
        )
        .await?;
    if let Some(row) = existing.first() {
        let id = rowx::i64_of(row, "id")?;
        return Ok((invoice_svc::load(db, invoice.tenant_id, id).await?, false));
    }
    let mut created = invoice;
    created.insert_or_update(db).await?;
    Ok((created, true))
}

/// 自然月边界：`[月初, 次月初)`。
pub fn month_bounds(at: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let start = Utc
        .with_ymd_and_hms(at.year(), at.month(), 1, 0, 0, 0)
        .single()
        .expect("valid month start");
    let (ny, nm) = if at.month() == 12 {
        (at.year() + 1, 1)
    } else {
        (at.year(), at.month() + 1)
    };
    let next = Utc
        .with_ymd_and_hms(ny, nm, 1, 0, 0, 0)
        .single()
        .expect("valid month start");
    (start, next)
}

/// 订阅 upsert（依赖方言 `on_conflict` 能力；MySQL 返回 `UnsupportedFeature`）。
pub async fn upsert_subscription(db: &Db, sub: &mut Subscription) -> Result<()> {
    sub.upsert(
        db,
        &["id"],
        &["plan", "period_start", "period_end", "status"],
    )
    .await
}

/// 用量记录（类型化读取）。
pub async fn usage_rows(db: &Db, subscription_id: i64) -> Result<Vec<UsageRecord>> {
    let rows = db
        .find_with_params(
            "SELECT * FROM usage_records WHERE subscription_id = ? ORDER BY id",
            vec![Value::I64(subscription_id)],
        )
        .await?;
    rows.into_iter().map(UsageRecord::from_row).collect()
}

/// 订阅是否 active。
pub fn is_active(s: &Subscription) -> bool {
    matches!(s.status, SubscriptionStatus::Active)
}

/// 计费周期字符串（`YYYY-MM`）。
pub fn period_key(at: DateTime<Utc>) -> String {
    format!("{:04}-{:02}", at.year(), at.month())
}

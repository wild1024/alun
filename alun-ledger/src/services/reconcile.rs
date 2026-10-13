//! 每日对账批处理（用例 REC-002..007）。

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::services::{faults::FailPoint, money, rowx};

/// 对账结果汇总。
#[derive(Debug, Clone, PartialEq)]
pub struct ReconcileSummary {
    /// 已处理行数（含重复跳过）。
    pub processed: usize,
    /// 实际校验的账户数。
    pub checked: usize,
    /// 差异数。
    pub diffs: usize,
}

/// 批量对账：`expected` 为 (账户 id, 期望余额)。
///
/// - 以 `batch` 为批次大小推进，逐批回调 `progress(已处理行数)`；
/// - 差异写入 `audit_logs`（`action = 'reconcile_diff'`），**重复执行不新增行**（幂等）；
/// - `start` 支持断点续跑（从中断处继续）。
pub async fn reconcile<F>(
    db: &Db,
    tenant_id: i64,
    expected: &[(i64, Decimal)],
    start: usize,
    batch: usize,
    at: DateTime<Utc>,
    mut progress: F,
) -> Result<ReconcileSummary>
where
    F: FnMut(usize),
{
    let mut checked = 0usize;
    let mut diffs = 0usize;
    let mut processed = start;

    while processed < expected.len() {
        let end = (processed + batch).min(expected.len());
        for (account_id, want) in &expected[processed..end] {
            let actual = money::read_one(
                db,
                "SELECT balance FROM accounts WHERE id = ? AND tenant_id = ?",
                vec![Value::I64(*account_id), Value::I64(tenant_id)],
            )
            .await?;
            checked += 1;
            if actual != *want && insert_diff(db, tenant_id, *account_id, *want, actual, at).await?
            {
                diffs += 1;
            }
        }
        processed = end;
        progress(processed);
    }

    Ok(ReconcileSummary {
        processed,
        checked,
        diffs,
    })
}

/// 写入差异（已存在同账户差异则跳过 → 幂等）。返回是否新增。
async fn insert_diff(
    db: &Db,
    tenant_id: i64,
    account_id: i64,
    want: Decimal,
    actual: Decimal,
    at: DateTime<Utc>,
) -> Result<bool> {
    let existing = db
        .find_with_params(
            "SELECT id FROM audit_logs WHERE tenant_id = ? AND entity_type = ? AND entity_id = ? AND action = ?",
            vec![
                Value::I64(tenant_id),
                Value::Text("account".into()),
                Value::I64(account_id),
                Value::Text("reconcile_diff".into()),
            ],
        )
        .await?;
    if !existing.is_empty() {
        return Ok(false);
    }

    db.transaction(|tx| async move {
        crate::services::faults::check(FailPoint::AuditWrite)?;
        let payload = serde_json::json!({
            "expected": money::money_str(want),
            "actual": money::money_str(actual),
        });
        tx.execute_with_params(
            "INSERT INTO audit_logs (id, tenant_id, entity_type, entity_id, action, before_json, after_json, actor_id, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(account_id),
                Value::I64(tenant_id),
                Value::Text("account".into()),
                Value::I64(account_id),
                Value::Text("reconcile_diff".into()),
                Value::Null,
                Value::Json(payload.to_string()),
                Value::I64(0),
                alun_db_core::traits::TypeMapper::to_value(&at),
            ],
        )
        .await?;
        Ok(())
    })
    .await?;
    Ok(true)
}

/// 差异行数（重复重跑幂等断言用）。
pub async fn diff_count(db: &Db, tenant_id: i64) -> Result<i64> {
    db.scalar_with_params::<i64>(
        "SELECT COUNT(*) FROM audit_logs WHERE tenant_id = ? AND action = ?",
        vec![Value::I64(tenant_id), Value::Text("reconcile_diff".into())],
    )
    .await
}

/// 汇总报告：Σ|expected - actual|。
pub async fn abs_diff_total(
    db: &Db,
    tenant_id: i64,
    expected: &[(i64, Decimal)],
) -> Result<Decimal> {
    let mut total = Decimal::ZERO;
    for (account_id, want) in expected {
        let actual = money::read_one(
            db,
            "SELECT balance FROM accounts WHERE id = ? AND tenant_id = ?",
            vec![Value::I64(*account_id), Value::I64(tenant_id)],
        )
        .await?;
        total += (*want - actual).abs();
    }
    Ok(total)
}

/// 读取账户代码（辅助断言）。
pub async fn account_code(db: &Db, account_id: i64) -> Result<String> {
    let rows = db
        .find_with_params(
            "SELECT code FROM accounts WHERE id = ?",
            vec![Value::I64(account_id)],
        )
        .await?;
    rows.first()
        .ok_or(DbError::NotFound)
        .and_then(|r| rowx::text_of(r, "code"))
}

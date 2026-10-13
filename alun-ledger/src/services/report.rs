//! 报表与仪表盘（用例 RPT-001..008）。

use alun_db::prelude::*;
use alun_db_core::expr::{BinaryOp, Expr};
use alun_db_core::statement::Order;
use alun_db_core::stmt::Value;
use rust_decimal::Decimal;

use crate::services::rowx;

/// MRR 明细（CTE + 排序）；按自然月分组的结果由 [`cumulative`] 在 Rust 侧累计。
///
/// 金额列在 SQLite 为 `TEXT`，`SUM()` 会被 SQLite 强制转 REAL（精度红线），
/// 因此**不**在 SQL 侧做金额聚合。
pub async fn mrr_by_month(db: &Db, tenant_id: i64) -> Result<Vec<(String, Decimal)>> {
    let rows = db
        .find_with_params(
            "WITH paid AS (SELECT month, total FROM v_mrr WHERE tenant_id = ?) \
             SELECT month, total FROM paid ORDER BY month",
            vec![Value::I64(tenant_id)],
        )
        .await?;
    rows.iter()
        .map(|r| Ok((rowx::text_of(r, "month")?, rowx::decimal_of(r, "total")?)))
        .collect()
}

/// 累计求和（Decimal 语义，替代窗口函数 `SUM() OVER`——AST 层无窗口函数，缺口 G7）。
pub fn cumulative(rows: &[(String, Decimal)]) -> Vec<(String, Decimal)> {
    let mut acc = Decimal::ZERO;
    rows.iter()
        .map(|(m, v)| {
            acc += *v;
            (m.clone(), acc)
        })
        .collect()
}

/// 用量 Top N（`GROUP BY subscription_id ORDER BY SUM(quantity) DESC LIMIT n`）。
pub async fn top_usage(db: &Db, limit: i64) -> Result<Vec<(i64, Decimal)>> {
    let rows = db
        .find_with_params(
            "SELECT subscription_id, SUM(quantity) AS total FROM usage_records \
             GROUP BY subscription_id ORDER BY SUM(quantity) DESC, subscription_id ASC LIMIT ?",
            vec![Value::I64(limit)],
        )
        .await?;
    rows.iter()
        .map(|r| {
            Ok((
                rowx::i64_of(r, "subscription_id")?,
                rowx::decimal_of(r, "total")?,
            ))
        })
        .collect()
}

/// 深分页（OFFSET）：`page` 从 1 开始。
pub async fn deep_page(db: &Db, tenant_id: i64, page: i64, size: i64) -> Result<Vec<i64>> {
    let offset = (page - 1).max(0) * size;
    let rows = db
        .find_with_params(
            "SELECT id FROM invoices WHERE tenant_id = ? ORDER BY id LIMIT ? OFFSET ?",
            vec![Value::I64(tenant_id), Value::I64(size), Value::I64(offset)],
        )
        .await?;
    rows.iter().map(|r| rowx::i64_of(r, "id")).collect()
}

/// 键集分页（`WHERE id > ? ORDER BY id LIMIT ?`）。
pub async fn keyset_page(
    db: &Db,
    tenant_id: i64,
    after: Option<i64>,
    size: i64,
) -> Result<Vec<i64>> {
    let rows = db
        .find_with_params(
            "SELECT id FROM invoices WHERE tenant_id = ? AND id > ? ORDER BY id LIMIT ?",
            vec![
                Value::I64(tenant_id),
                Value::I64(after.unwrap_or(0)),
                Value::I64(size),
            ],
        )
        .await?;
    rows.iter().map(|r| rowx::i64_of(r, "id")).collect()
}

/// JSON 路径过滤：用 AST 节点 `Expr::JsonGet` 经 `FindBuilder::filter`（RPT-008）。
///
/// 渲染：PG `metadata ->> 'tier'`、MySQL `JSON_UNQUOTE(JSON_EXTRACT(...))`、
/// SQLite `json_extract(...)`。
pub async fn invoices_by_json_path(db: &Db, tier: &str) -> Result<Vec<i64>> {
    let column = Expr::Column {
        table: "invoices",
        column: "metadata",
    };
    let json = Expr::JsonGet {
        expr: Box::new(column),
        path: "tier".to_string(),
    };
    let cond = Expr::Binary {
        left: Box::new(json),
        op: BinaryOp::Eq,
        right: Box::new(Expr::Value(Value::Text(tier.to_string()))),
    };
    let rows = FindBuilder::new()
        .table("invoices")
        .filter(cond)
        .order_by("id", Order::Asc)
        .exec(db)
        .await?;
    rows.iter().map(|r| rowx::i64_of(r, "id")).collect()
}

/// 对账差异：转账金额 vs 分录合计（Rust 侧 Decimal 求和）。
pub async fn reconcile_diff(db: &Db, tenant_id: i64) -> Result<Vec<(i64, Decimal, Decimal)>> {
    let transfers = db
        .find_with_params(
            "SELECT id, amount FROM transfers WHERE tenant_id = ? ORDER BY id",
            vec![Value::I64(tenant_id)],
        )
        .await?;
    let mut out = Vec::new();
    for t in &transfers {
        let id = rowx::i64_of(t, "id")?;
        let amount = rowx::decimal_of(t, "amount")?;
        let entries = db
            .find_with_params(
                "SELECT amount, direction FROM ledger_entries WHERE transfer_id = ?",
                vec![Value::I64(id)],
            )
            .await?;
        let mut debit = Decimal::ZERO;
        for e in &entries {
            let a = rowx::decimal_of(e, "amount")?;
            if rowx::text_of(e, "direction")? == "debit" {
                debit += a;
            }
        }
        out.push((id, amount, debit));
    }
    Ok(out)
}

/// 只读视图的类型化读取（走 [`invoice_svc`] 同款金额解码）。
pub async fn mrr_view_rows(db: &Db, tenant_id: i64) -> Result<Vec<(String, Decimal)>> {
    mrr_by_month(db, tenant_id).await
}

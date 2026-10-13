//! 发票状态机（用例 INV-001..010）。

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use alun_db_core::traits::TypeMapper;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::models::{Invoice, InvoiceItem, InvoiceStatus};
use crate::services::audit::{self, AuditRecord};
use crate::services::rowx;

/// 金额一致性校验：`Σ(quantity × unit_price) == total`。
pub fn items_total(items: &[InvoiceItem]) -> Decimal {
    items.iter().fold(Decimal::ZERO, |acc, i| {
        acc + i.unit_price * Decimal::from(i.quantity)
    })
}

/// 从 `Row` 重建 [`Invoice`]（金额走方言无关解码，见缺陷 D1）。
pub fn invoice_from_row(row: &Row) -> Result<Invoice> {
    Ok(Invoice {
        id: rowx::i64_of(row, "id")?,
        tenant_id: rowx::i64_of(row, "tenant_id")?,
        customer_id: rowx::i64_of(row, "customer_id")?,
        status: InvoiceStatus::from_value(rowx::value(row, "status")?)?,
        total: rowx::decimal_of(row, "total")?,
        metadata: rowx::opt_json_of(row, "metadata")?,
        issued_at: rowx::opt_ts_of(row, "issued_at")?,
        paid_at: rowx::opt_ts_of(row, "paid_at")?,
        created_at: rowx::ts_of(row, "created_at")?,
        note: rowx::opt_text_of(row, "note")?,
    })
}

/// 读取发票（带租户作用域）。
pub async fn load(db: &Db, tenant_id: i64, invoice_id: i64) -> Result<Invoice> {
    let rows = db
        .find_with_params(
            "SELECT * FROM invoices WHERE id = ? AND tenant_id = ?",
            vec![Value::I64(invoice_id), Value::I64(tenant_id)],
        )
        .await?;
    rows.first()
        .ok_or(DbError::NotFound)
        .and_then(invoice_from_row)
}

/// 读取发票条目（按 id 升序，金额走解码）。
pub async fn items(db: &Db, invoice_id: i64) -> Result<Vec<InvoiceItem>> {
    let rows = db
        .find_with_params(
            "SELECT * FROM invoice_items WHERE invoice_id = ? ORDER BY id",
            vec![Value::I64(invoice_id)],
        )
        .await?;
    rows.iter()
        .map(|r| {
            Ok(InvoiceItem {
                id: rowx::i64_of(r, "id")?,
                invoice_id: rowx::i64_of(r, "invoice_id")?,
                description: rowx::text_of(r, "description")?,
                quantity: rowx::i64_of(r, "quantity")?,
                unit_price: rowx::decimal_of(r, "unit_price")?,
                amount: rowx::decimal_of(r, "amount")?,
                tax_rate: rowx::text_of(r, "tax_rate")?,
            })
        })
        .collect()
}

/// 创建草稿发票：校验金额一致性 → 同事务写入发票 + 条目 + 审计。
pub async fn create_draft(
    db: &Db,
    invoice: Invoice,
    items: &[InvoiceItem],
    actor_id: i64,
) -> Result<Invoice> {
    let sum = items_total(items);
    if sum != invoice.total {
        return Err(DbError::InvalidValue(format!(
            "invoice total {} != items sum {}",
            invoice.total, sum
        )));
    }
    let invoice_clone = invoice.clone();
    let items_owned: Vec<InvoiceItem> = items.to_vec();
    db.transaction(|tx| async move {
        tx.execute_with_params(
            "INSERT INTO invoices (id, tenant_id, customer_id, status, total, metadata, issued_at, paid_at, created_at, note) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(invoice_clone.id),
                Value::I64(invoice_clone.tenant_id),
                Value::I64(invoice_clone.customer_id),
                Value::Text("draft".into()),
                invoice_clone.total.to_value(),
                match &invoice_clone.metadata {
                    Some(j) => Value::Json(j.to_string()),
                    None => Value::Null,
                },
                Value::Null,
                Value::Null,
                invoice_clone.created_at.to_value(),
                match &invoice_clone.note {
                    Some(n) => Value::Text(n.clone()),
                    None => Value::Null,
                },
            ],
        )
        .await?;

        for item in &items_owned {
            tx.execute_with_params(
                "INSERT INTO invoice_items (id, invoice_id, description, quantity, unit_price, amount, tax_rate) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                vec![
                    Value::I64(item.id),
                    Value::I64(invoice_clone.id),
                    Value::Text(item.description.clone()),
                    Value::I64(item.quantity),
                    item.unit_price.to_value(),
                    item.amount.to_value(),
                    Value::Text(item.tax_rate.clone()),
                ],
            )
            .await?;
        }

        let after = serde_json::json!({
            "status": "draft",
            "total": invoice_clone.total.to_string(),
            "items": items_owned.len(),
        });
        audit::record(
            &tx,
            &AuditRecord {
                id: invoice_clone.id,
                tenant_id: invoice_clone.tenant_id,
                entity_type: "invoice",
                entity_id: invoice_clone.id,
                action: "create",
                before: None,
                after: Some(&after),
                actor_id,
                at: invoice_clone.created_at,
            },
        )
        .await?;

        Ok(invoice_clone)
    })
    .await
}

/// 状态流转：校验合法迁移 → 同事务更新状态与时间戳 + 审计。
pub async fn transition(
    db: &Db,
    tenant_id: i64,
    invoice_id: i64,
    next: InvoiceStatus,
    at: DateTime<Utc>,
    actor_id: i64,
) -> Result<Invoice> {
    let current = load(db, tenant_id, invoice_id).await?;
    if !current.status.can_transition_to(next) {
        return Err(DbError::InvalidValue(format!(
            "illegal transition {:?} -> {:?}",
            current.status, next
        )));
    }

    let before = serde_json::json!({ "status": status_key(current.status) });
    let after = serde_json::json!({ "status": status_key(next) });

    db.transaction(|tx| async move {
        match next {
            InvoiceStatus::Issued => {
                tx.execute_with_params(
                    "UPDATE invoices SET status = ?, issued_at = ? WHERE id = ? AND tenant_id = ?",
                    vec![
                        Value::Text(status_key(next).into()),
                        at.to_value(),
                        Value::I64(invoice_id),
                        Value::I64(tenant_id),
                    ],
                )
                .await?;
            }
            InvoiceStatus::Paid => {
                tx.execute_with_params(
                    "UPDATE invoices SET status = ?, paid_at = ? WHERE id = ? AND tenant_id = ?",
                    vec![
                        Value::Text(status_key(next).into()),
                        at.to_value(),
                        Value::I64(invoice_id),
                        Value::I64(tenant_id),
                    ],
                )
                .await?;
            }
            _ => {
                tx.execute_with_params(
                    "UPDATE invoices SET status = ? WHERE id = ? AND tenant_id = ?",
                    vec![
                        Value::Text(status_key(next).into()),
                        Value::I64(invoice_id),
                        Value::I64(tenant_id),
                    ],
                )
                .await?;
            }
        }

        audit::record(
            &tx,
            &AuditRecord {
                id: invoice_id * 100 + next as i64,
                tenant_id,
                entity_type: "invoice",
                entity_id: invoice_id,
                action: status_key(next),
                before: Some(&before),
                after: Some(&after),
                actor_id,
                at,
            },
        )
        .await?;
        Ok(())
    })
    .await?;

    load(db, tenant_id, invoice_id).await
}

/// 新增条目：`Voided` / `Paid` 后禁止。
pub async fn add_item(db: &Db, tenant_id: i64, item: InvoiceItem, at: DateTime<Utc>) -> Result<()> {
    let invoice = load(db, tenant_id, item.invoice_id).await?;
    if matches!(invoice.status, InvoiceStatus::Voided | InvoiceStatus::Paid) {
        return Err(DbError::InvalidValue(format!(
            "invoice {} is {:?}, cannot add items",
            invoice.id, invoice.status
        )));
    }
    let expected = item.unit_price * Decimal::from(item.quantity);
    if expected != item.amount {
        return Err(DbError::InvalidValue(format!(
            "item amount {} != quantity * unit_price {}",
            item.amount, expected
        )));
    }
    let new_total = invoice.total + item.amount;
    let item_id = item.id;
    let invoice_id = invoice.id;

    db.transaction(|tx| async move {
        // 事务内直接用 Model API（D003-A 已修复）：插入条目 + 按主键读回 + 部分更新金额
        let mut item = item;
        item.insert_or_update(&tx).await?;

        let inv = Invoice::find_first_by(
            &tx,
            "id = ? AND tenant_id = ?",
            (invoice_id, tenant_id),
        )
        .await?
        .ok_or(DbError::NotFound)?;
        // 只更新 total：变更集把「主键 + 变更列」交给 Hook（D016）
        Invoice::of(inv.id)
            .set(Invoice::total(), new_total)
            .update(&tx)
            .await?;
        let after = serde_json::json!({ "invoice_id": invoice_id, "item_id": item_id, "total": new_total.to_string() });
        audit::record(
            &tx,
            &AuditRecord {
                id: item_id,
                tenant_id,
                entity_type: "invoice_item",
                entity_id: item_id,
                action: "add",
                before: None,
                after: Some(&after),
                actor_id: 0,
                at,
            },
        )
        .await?;
        Ok(())
    })
    .await
}

/// 删除发票（验证级联行为；SQLite 需 `PRAGMA foreign_keys=ON`）。
pub async fn delete(db: &Db, tenant_id: i64, invoice_id: i64) -> Result<u64> {
    db.execute_with_params(
        "DELETE FROM invoices WHERE id = ? AND tenant_id = ?",
        vec![Value::I64(invoice_id), Value::I64(tenant_id)],
    )
    .await
}

/// 条目条数。
pub async fn item_count(db: &Db, invoice_id: i64) -> Result<i64> {
    InvoiceItem::count_by(db, "invoice_id = ?", (invoice_id,)).await
}

fn status_key(s: InvoiceStatus) -> &'static str {
    match s {
        InvoiceStatus::Draft => "draft",
        InvoiceStatus::Issued => "issued",
        InvoiceStatus::Paid => "paid",
        InvoiceStatus::Voided => "voided",
    }
}

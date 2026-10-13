//! M5 审计日志（用例 AUD-001..008）。
//!
//! 本文件覆盖 `services::audit`（服务层显式事务写审计）与 `hooks/*`（Hook 洋葱模型）。
//!
//! ## 框架现状（用例内逐条标注，报告汇总）
//!
//! - `Hook` 是**同步且无 IO** 的 trait（`alun-db-core/src/traits/hook.rs`），
//!   Hook 拿不到 `Db` / `Transaction`，因此**无法在 Hook 内写库**：审计落库必须由
//!   服务层在同一个显式事务内完成（AUD-005/006）。
//! - `Row` **不做变更追踪**：动态行写入不经过模型 Hook，变更追踪由 `ChangeSet` 提供
//!   全字段行，「只记变更列」必须由项目层先 SELECT 再 diff（AUD-002）。
//! - 框架**未**对审计模型提供不可变保护（无只读属性/触发器约束层）：模型层的
//!   `update` / `delete` 依旧可用，AUD-007 记录真实结果。
//! - `PersistExt::*` 只接受 `&Db`，**不参与**调用方事务（缺陷 D3）：所有需要
//!   「审计失败 → 业务回滚」的场景必须走 `tx.execute_with_params`。

mod common;

use alun_db::prelude::*;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::json;
use std::cell::{Cell, RefCell};

use alun_ledger::hooks::{drain_audit_trace, drain_order};
use alun_ledger::models::probe::{AuditProbe, HookProbe};
use alun_ledger::models::{AuditLog, Invoice, InvoiceItem, InvoiceStatus};
use alun_ledger::services::audit::{self, AuditRecord};
use alun_ledger::services::{faults::FailPoint, invoice as invoice_svc, money, rowx};

use common::assertions::*;
use common::faults::Armed;
use common::fixtures::*;
use common::harness::{Dialect, LedgerCtx, ctx};

// ==================== 测试自有 Hook 探针 ====================
//
// `#[derive(Model)]` 与 `Hook` trait 都是对外 API，测试文件内可自定义模型/Hook，
// 无需改动 `src/**`。表结构由测试按方言现场创建（见 `create_probe_tables`）。

thread_local! {
    /// 测试自有 Hook 的执行痕迹（与库内 `hooks::drain_order` 相互独立）。
    static TRACE: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// 是否让 [`ProbeDenyHook`] 拒绝本次操作。
    static DENY: Cell<bool> = const { Cell::new(false) };
}

fn trace_push(label: &str) {
    TRACE.with(|t| t.borrow_mut().push(label.to_string()));
}

fn trace_take() -> Vec<String> {
    TRACE.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

fn deny_if_armed(stage: &str) -> Result<()> {
    if DENY.with(|d| d.get()) {
        Err(DbError::InvalidValue(format!(
            "probe hook denied at {stage}"
        )))
    } else {
        Ok(())
    }
}

/// 会拒绝操作的探针 Hook（AUD-003：任何 `before_*` 返回 `Err` → 操作中止）。
pub struct ProbeDenyHook;

impl Hook for ProbeDenyHook {
    fn before_insert(&self, _row: &mut Row) -> Result<()> {
        trace_push("deny:before_insert");
        deny_if_armed("before_insert")
    }
    fn after_insert(&self, _row: &Row) -> Result<()> {
        trace_push("deny:after_insert");
        Ok(())
    }
    fn before_update(&self, _row: &mut Row) -> Result<()> {
        trace_push("deny:before_update");
        deny_if_armed("before_update")
    }
    fn after_update(&self, _row: &Row) -> Result<()> {
        trace_push("deny:after_update");
        Ok(())
    }
}

/// 排在拒绝 Hook 之后的探针，用于证明「中断后后续 Hook 与 `after_*` 均不再执行」。
pub struct ProbeTailHook;

impl Hook for ProbeTailHook {
    fn before_insert(&self, _row: &mut Row) -> Result<()> {
        trace_push("tail:before_insert");
        Ok(())
    }
    fn after_insert(&self, _row: &Row) -> Result<()> {
        trace_push("tail:after_insert");
        Ok(())
    }
    fn before_update(&self, _row: &mut Row) -> Result<()> {
        trace_push("tail:before_update");
        Ok(())
    }
    fn after_update(&self, _row: &Row) -> Result<()> {
        trace_push("tail:after_update");
        Ok(())
    }
}

/// 钩子拒绝/尾随探针模型（表由测试创建）。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("aud_gate_probe")]
#[hook(ProbeDenyHook)]
#[hook(ProbeTailHook)]
pub struct GateProbe {
    #[key]
    pub id: i64,
    pub label: String,
}

/// 审计痕迹探针模型：3 列（id / label / amount），用于 AUD-002 的部分更新断言。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("aud_trace_probe")]
#[hook(alun_ledger::hooks::AuditTraceHook)]
pub struct TraceProbe {
    #[key]
    pub id: i64,
    pub label: String,
    pub amount: Decimal,
}

/// 按方言创建探针表（幂等：`IF NOT EXISTS` + 使用前清表）。
async fn create_probe_tables(c: &LedgerCtx) {
    let ddls: [&str; 4] = if c.dialect.is_sqlite() {
        [
            "CREATE TABLE IF NOT EXISTS aud_gate_probe (id INTEGER NOT NULL PRIMARY KEY, label VARCHAR(64) NOT NULL)",
            "CREATE TABLE IF NOT EXISTS aud_trace_probe (id INTEGER NOT NULL PRIMARY KEY, label VARCHAR(64) NOT NULL, amount TEXT NOT NULL)",
            "CREATE TABLE IF NOT EXISTS hook_probe (id INTEGER NOT NULL PRIMARY KEY, label VARCHAR(64) NOT NULL)",
            "CREATE TABLE IF NOT EXISTS audit_probe (id INTEGER NOT NULL PRIMARY KEY, label VARCHAR(64) NOT NULL)",
        ]
    } else {
        [
            "CREATE TABLE IF NOT EXISTS aud_gate_probe (id BIGINT NOT NULL PRIMARY KEY, label VARCHAR(64) NOT NULL)",
            "CREATE TABLE IF NOT EXISTS aud_trace_probe (id BIGINT NOT NULL PRIMARY KEY, label VARCHAR(64) NOT NULL, amount DECIMAL(38, 10) NOT NULL)",
            "CREATE TABLE IF NOT EXISTS hook_probe (id BIGINT NOT NULL PRIMARY KEY, label VARCHAR(64) NOT NULL)",
            "CREATE TABLE IF NOT EXISTS audit_probe (id BIGINT NOT NULL PRIMARY KEY, label VARCHAR(64) NOT NULL)",
        ]
    };
    for ddl in ddls {
        c.db().execute(ddl).await.expect("create probe table");
    }
    for table in [
        "aud_gate_probe",
        "aud_trace_probe",
        "hook_probe",
        "audit_probe",
    ] {
        c.db()
            .execute(&format!("DELETE FROM {table}"))
            .await
            .expect("clean probe table");
    }
}

/// 探针行快照（金额归一化，避免 scale 差异）。
fn probe_snapshot(row: &Row) -> Result<serde_json::Value> {
    Ok(json!({
        "id": rowx::i64_of(row, "id")?,
        "label": rowx::text_of(row, "label")?,
        "amount": money::money_str(rowx::decimal_of(row, "amount")?),
    }))
}

async fn select_probe(db: &Db, id: i64) -> Result<Row> {
    db.find_with_params(
        "SELECT * FROM aud_trace_probe WHERE id = ?",
        vec![Value::I64(id)],
    )
    .await?
    .into_iter()
    .next()
    .ok_or(DbError::NotFound)
}

/// 只保留「变更列 + 主键」的快照对（框架无变更追踪，需项目层 diff）。
fn changed_only(
    before: &serde_json::Value,
    after: &serde_json::Value,
    pk: &str,
) -> (serde_json::Value, serde_json::Value) {
    let mut b = serde_json::Map::new();
    let mut a = serde_json::Map::new();
    for (key, before_value) in before.as_object().expect("before 必须是 JSON 对象") {
        let after_value = after.get(key).unwrap_or(&serde_json::Value::Null);
        if key == pk || before_value != after_value {
            b.insert(key.clone(), before_value.clone());
            a.insert(key.clone(), after_value.clone());
        }
    }
    (serde_json::Value::Object(b), serde_json::Value::Object(a))
}

// ==================== 审计写入助手（服务层显式事务） ====================

/// 发票行全字段快照（列集合 = `invoices` 表可写列）。
fn invoice_snapshot(row: &Row) -> Result<serde_json::Value> {
    Ok(json!({
        "id": rowx::i64_of(row, "id")?,
        "tenant_id": rowx::i64_of(row, "tenant_id")?,
        "customer_id": rowx::i64_of(row, "customer_id")?,
        "status": rowx::text_of(row, "status")?,
        "total": money::money_str(rowx::decimal_of(row, "total")?),
        "metadata": rowx::opt_json_of(row, "metadata")?,
        "issued_at": rowx::opt_ts_of(row, "issued_at")?.map(|t| t.to_rfc3339()),
        "paid_at": rowx::opt_ts_of(row, "paid_at")?.map(|t| t.to_rfc3339()),
        "created_at": rowx::ts_of(row, "created_at")?.to_rfc3339(),
        "note": rowx::opt_text_of(row, "note")?,
    }))
}

async fn select_invoice_row(db: &Db, tenant_id: i64, invoice_id: i64) -> Result<Row> {
    db.find_with_params(
        "SELECT * FROM invoices WHERE id = ? AND tenant_id = ?",
        invoice_params(tenant_id, invoice_id),
    )
    .await?
    .into_iter()
    .next()
    .ok_or(DbError::NotFound)
}

fn invoice_params(tenant_id: i64, invoice_id: i64) -> Vec<Value> {
    vec![Value::I64(invoice_id), Value::I64(tenant_id)]
}

/// 全字段更新 + 同事务审计（`PersistExt::update` 无法加入调用方事务，故走 raw SQL）。
///
/// 返回 `(before, after)` 两个快照，与数据库实际值一致。
async fn audited_full_update(
    db: &Db,
    invoice: Invoice,
    audit_id: i64,
    actor_id: i64,
    at: DateTime<Utc>,
) -> Result<(serde_json::Value, serde_json::Value)> {
    db.transaction(|tx| async move {
        let row = tx
            .find_with_params(
                "SELECT * FROM invoices WHERE id = ? AND tenant_id = ?",
                invoice_params(invoice.tenant_id, invoice.id),
            )
            .await?
            .into_iter()
            .next()
            .ok_or(DbError::NotFound)?;
        let before = invoice_snapshot(&row)?;

        // 全字段 UPDATE：列集合 = 模型全部非主键列
        tx.execute_with_params(
            "UPDATE invoices SET tenant_id = ?, customer_id = ?, status = ?, total = ?, metadata = ?, \
             issued_at = ?, paid_at = ?, created_at = ?, note = ? WHERE id = ?",
            vec![
                Value::I64(invoice.tenant_id),
                Value::I64(invoice.customer_id),
                invoice.status.to_value(),
                invoice.total.to_value(),
                match &invoice.metadata {
                    Some(j) => Value::Json(j.to_string()),
                    None => Value::Null,
                },
                match invoice.issued_at {
                    Some(t) => t.to_value(),
                    None => Value::Null,
                },
                match invoice.paid_at {
                    Some(t) => t.to_value(),
                    None => Value::Null,
                },
                invoice.created_at.to_value(),
                match &invoice.note {
                    Some(n) => Value::Text(n.clone()),
                    None => Value::Null,
                },
                Value::I64(invoice.id),
            ],
        )
        .await?;

        let after_row = tx
            .find_with_params(
                "SELECT * FROM invoices WHERE id = ? AND tenant_id = ?",
                invoice_params(invoice.tenant_id, invoice.id),
            )
            .await?
            .into_iter()
            .next()
            .ok_or(DbError::NotFound)?;
        let after = invoice_snapshot(&after_row)?;

        audit::record(
            &tx,
            &AuditRecord {
                id: audit_id,
                tenant_id: invoice.tenant_id,
                entity_type: "invoice",
                entity_id: invoice.id,
                action: "update",
                before: Some(&before),
                after: Some(&after),
                actor_id,
                at,
            },
        )
        .await?;
        Ok((before, after))
    })
    .await
}

/// 在显式事务内写一条审计记录（AUD-007/008 的种子数据）。
// 测试种子辅助：字段较多，展开为结构体反而降低可读性
#[allow(clippy::too_many_arguments)]
async fn seed_audit(
    db: &Db,
    id: i64,
    tenant_id: i64,
    entity_type: &str,
    entity_id: i64,
    action: &str,
    actor_id: i64,
    at: DateTime<Utc>,
) -> Result<()> {
    db.transaction(|tx| async move {
        let payload = json!({ "action": action });
        audit::record(
            &tx,
            &AuditRecord {
                id,
                tenant_id,
                entity_type,
                entity_id,
                action,
                before: None,
                after: Some(&payload),
                actor_id,
                at,
            },
        )
        .await
    })
    .await
}

// ==================== 用例 ====================

/// AUD-001：全字段更新写入的 before/after 与数据库实际值一致，且可 JSON 往返。
#[tokio::test]
async fn aud_001_full_update_snapshots_match_db() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_invoice(&c, TENANT_T1, 7001, "draft").await.unwrap();

    let pre = invoice_snapshot(&select_invoice_row(c.db(), TENANT_T1, 7001).await.unwrap())
        .expect("pre snapshot");

    // 全字段更新：状态 / 金额 / 备注 / JSON 元数据 / 时间戳
    let mut invoice = invoice_svc::load(c.db(), TENANT_T1, 7001)
        .await
        .expect("load");
    invoice.status = InvoiceStatus::Issued;
    invoice.total = money(12, 34);
    invoice.note = Some("issued by test".into());
    invoice.metadata = Some(json!({ "tier": "gold", "flag": true }));
    invoice.issued_at = Some(at(3600));
    invoice.paid_at = None;

    let (before, after) = audited_full_update(c.db(), invoice.clone(), 9401, 42, at(3600))
        .await
        .expect("audited full update");

    // 1) 快照与数据库实际值一致
    let post = invoice_snapshot(&select_invoice_row(c.db(), TENANT_T1, 7001).await.unwrap())
        .expect("post snapshot");
    assert_eq!(before, pre, "before 快照应等于更新前的数据库状态");
    assert_eq!(after, post, "after 快照应等于更新后的数据库状态");
    assert_ne!(before, after, "全字段更新必须产生差异");
    assert_eq!(after["status"], json!("issued"));
    assert_eq!(after["total"], json!("12.34"));
    assert_eq!(after["note"], json!("issued by test"));
    assert_eq!(after["metadata"]["tier"], json!("gold"));

    // 2) 审计行内容与快照一致 + JSON 往返
    let logged = audit::list_for(c.db(), "invoice", 7001)
        .await
        .expect("list_for");
    assert_eq!(logged.len(), 1, "应恰好一条审计记录");
    let rec = &logged[0];
    assert_eq!(rec.id, 9401);
    assert_eq!(rec.action, "update");
    assert_eq!(rec.actor_id, 42);
    assert_eq!(rec.tenant_id, TENANT_T1);
    assert_eq!(rec.created_at, at(3600));
    let logged_before = rec.before_json.clone().expect("before_json");
    let logged_after = rec.after_json.clone().expect("after_json");
    assert_eq!(logged_before, before);
    assert_eq!(logged_after, after);
    let roundtrip: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&logged_after).unwrap()).unwrap();
    assert_eq!(roundtrip, logged_after, "audit JSON 必须可无损往返");
}

/// AUD-002：变更集只改指定列；审计只记「变更列 + 主键」。
///
/// 框架现状：变更集把**只含变更列的行**交给 Hook，项目层无需再 diff；
/// 「只记变更列」由项目层先 SELECT 再 diff——本用例同时断言这两件事。
#[tokio::test]
async fn aud_002_partial_update_records_only_changed_columns() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    create_probe_tables(&c).await;

    let mut probe = TraceProbe {
        id: 1,
        label: "before".into(),
        amount: money(10, 0),
    };
    probe.insert_or_update(c.db()).await.expect("insert probe");
    let _ = drain_audit_trace();

    let before_snap = probe_snapshot(&select_probe(c.db(), 1).await.unwrap()).expect("before");

    // 部分更新：只更新 amount
    let mut updated = probe.clone();
    updated.amount = money(20, 0);
    let affected = TraceProbe::of(updated.id)
        .set(TraceProbe::amount(), updated.amount)
        .update(c.db())
        .await
        .expect("update");
    assert_eq!(affected, 1);

    // 1) DB 层：只有 amount 变化，未列出的列保持原值
    let after_row = select_probe(c.db(), 1).await.unwrap();
    assert_eq!(rowx::text_of(&after_row, "label").unwrap(), "before");
    assert_eq!(
        money::money_str(rowx::decimal_of(&after_row, "amount").unwrap()),
        "20"
    );
    let after_snap = probe_snapshot(&after_row).expect("after");

    // 2) Hook 层：变更集触发的 Hook 行只含**主键 + 本次变更列**（D016 已修复）
    let trace = drain_audit_trace();
    assert_eq!(
        trace,
        vec![
            "before_update[id,amount]".to_string(),
            "after_update[id,amount]".to_string(),
        ],
        "Hook 行不做变更追踪 → 全字段可见（框架缺口，项目层负责 diff）"
    );

    // 3) 项目层：diff 后只记变更列 + 主键
    let (changed_before, changed_after) = changed_only(&before_snap, &after_snap, "id");
    assert_eq!(changed_before, json!({ "id": 1, "amount": "10" }));
    assert_eq!(changed_after, json!({ "id": 1, "amount": "20" }));
    assert!(
        changed_before.as_object().unwrap().len() == 2,
        "before 快照只含变更列 + 主键"
    );

    c.db()
        .transaction(|tx| async move {
            audit::record(
                &tx,
                &AuditRecord {
                    id: 9501,
                    tenant_id: TENANT_T1,
                    entity_type: "aud_trace_probe",
                    entity_id: 1,
                    action: "partial_update",
                    before: Some(&changed_before),
                    after: Some(&changed_after),
                    actor_id: 7,
                    at: at(60),
                },
            )
            .await
        })
        .await
        .expect("audit record");

    let logged = audit::list_for(c.db(), "aud_trace_probe", 1)
        .await
        .expect("list_for");
    assert_eq!(logged.len(), 1);
    let keys: Vec<String> = logged[0]
        .before_json
        .as_ref()
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        keys,
        vec!["amount".to_string(), "id".to_string()],
        "只含变更列 + 主键"
    );
    assert_eq!(
        logged[0].after_json.as_ref().unwrap()["amount"],
        json!("20")
    );

    // 4) 空变更集与「只改主键」都被拒绝：InvalidValue
    assert!(matches!(
        TraceProbe::of(updated.id).update(c.db()).await,
        Err(DbError::InvalidValue(_))
    ));
    assert!(matches!(
        TraceProbe::of(updated.id)
            .set(TraceProbe::id(), updated.id)
            .update(c.db())
            .await,
        Err(DbError::InvalidValue(_))
    ));
}

/// AUD-003：Hook 返回 `Err` → 操作中止、不写行、错误原样上抛。
///
/// 探针 Hook 定义在本文件内（`ProbeDenyHook`），挂在测试自有模型 `GateProbe` 上。
#[tokio::test]
async fn aud_003_hook_error_aborts_without_writing_row() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    create_probe_tables(&c).await;

    // --- INSERT 路径：before_insert 返回 Err ---
    DENY.with(|d| d.set(true));
    let _ = trace_take();
    let mut fresh = GateProbe {
        id: 5,
        label: "denied".into(),
    };
    let err = fresh
        .insert_or_update(c.db())
        .await
        .expect_err("Hook 拒绝后 save 必须失败");
    assert!(
        matches!(&err, DbError::InvalidValue(m) if m.contains("denied at before_insert")),
        "错误应原样上抛，实际 {err:?}"
    );
    DENY.with(|d| d.set(false));

    assert_eq!(
        count_by(c.db(), "SELECT COUNT(*) FROM aud_gate_probe", vec![]).await,
        0,
        "被拒绝的 INSERT 不得留下任何行"
    );
    assert_eq!(
        trace_take(),
        vec!["deny:before_insert".to_string()],
        "中断后后续 Hook 与 after_* 均不得执行"
    );

    // --- 控制组：不拒绝时正常写入（before_* 正序） ---
    let mut ok = GateProbe {
        id: 6,
        label: "kept".into(),
    };
    ok.insert_or_update(c.db()).await.expect("正常写入");
    assert_eq!(
        trace_take(),
        vec![
            "deny:before_insert".to_string(),
            "tail:before_insert".to_string(),
            // after_* 逆序（洋葱模型）
            "tail:after_insert".to_string(),
            "deny:after_insert".to_string(),
        ]
    );

    // --- UPDATE 路径：before_update 返回 Err → 不执行 UPDATE，DB 不变 ---
    let changing = GateProbe {
        id: 6,
        label: "changed".into(),
    };
    DENY.with(|d| d.set(true));
    let err = changing
        .update(c.db())
        .await
        .expect_err("Hook 拒绝后 update 必须失败");
    assert!(
        matches!(&err, DbError::InvalidValue(m) if m.contains("denied at before_update")),
        "实际 {err:?}"
    );
    DENY.with(|d| d.set(false));
    assert_eq!(
        trace_take(),
        vec!["deny:before_update".to_string()],
        "before_update 中断后不得有 tail / after_update"
    );
    let row = c
        .db()
        .find_with_params(
            "SELECT label FROM aud_gate_probe WHERE id = ?",
            vec![Value::I64(6)],
        )
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(
        rowx::text_of(&row, "label").unwrap(),
        "kept",
        "被拒绝的 UPDATE 不得改动数据"
    );
}

/// AUD-004：Hook 洋葱模型——`before_*` 正序、`after_*` 逆序。
#[tokio::test]
async fn aud_004_onion_order_before_forward_after_reverse() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    create_probe_tables(&c).await;

    assert_eq!(HookProbe::hooks().len(), 3, "HookProbe 应挂 3 个 Hook");

    // INSERT：before A→B→C，after C→B→A
    let mut probe = HookProbe {
        id: 1,
        label: "v1".into(),
    };
    probe.insert_or_update(c.db()).await.expect("insert");
    assert_eq!(
        drain_order(),
        vec![
            "A:before_insert",
            "B:before_insert",
            "C:before_insert",
            "C:after_insert",
            "B:after_insert",
            "A:after_insert",
        ]
    );

    // UPDATE：同样洋葱
    probe.label = "v2".into();
    probe.update(c.db()).await.expect("update");
    assert_eq!(
        drain_order(),
        vec![
            "A:before_update",
            "B:before_update",
            "C:before_update",
            "C:after_update",
            "B:after_update",
            "A:after_update",
        ]
    );

    // DELETE：`before_delete` 正序；三个探针都未实现 `after_delete`
    probe.delete(c.db()).await.expect("delete");
    assert_eq!(
        drain_order(),
        vec!["A:before_delete", "B:before_delete", "C:before_delete"]
    );
    assert_eq!(
        count_by(c.db(), "SELECT COUNT(*) FROM hook_probe", vec![]).await,
        0
    );
}

/// AUD-005：递归禁止——审计模型自身不挂 Hook，Hook 同步无 IO 无法写库。
#[tokio::test]
async fn aud_005_audit_write_does_not_recurse() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    create_probe_tables(&c).await;

    // 结构上：`AuditLog`（审计表模型）不挂任何 Hook → 写审计不可能再触发审计
    assert!(
        AuditLog::hooks().is_empty(),
        "审计模型不得挂 Hook（递归禁止的结构保证）"
    );
    assert_eq!(AuditProbe::hooks().len(), 1, "探针模型对照组确实挂了 Hook");

    // 痕迹确实被记录（证明 Hook 真的执行了）
    let mut probe = AuditProbe {
        id: 1,
        label: "traced".into(),
    };
    probe.insert_or_update(c.db()).await.expect("insert probe");
    let trace = drain_audit_trace();
    assert_eq!(
        trace,
        vec!["before_insert[id,label]".to_string()],
        "AuditTraceHook 在 before_insert 记录行键集合"
    );
    probe.label = "traced2".into();
    probe.insert_or_update(c.db()).await.expect("update probe");
    assert_eq!(
        drain_audit_trace(),
        vec![
            "before_update[id,label]".to_string(),
            "after_update[id,label]".to_string(),
        ]
    );

    // 运行期：写一条审计行只增加 1 行（不产生「审计的审计」）
    let before = count_by(c.db(), "SELECT COUNT(*) FROM audit_logs", vec![]).await;
    seed_audit(c.db(), 9701, TENANT_T1, "audit_probe", 1, "trace", 0, at(0))
        .await
        .expect("write audit");
    let after = count_by(c.db(), "SELECT COUNT(*) FROM audit_logs", vec![]).await;
    assert_eq!(after - before, 1, "审计写入不得自我触发");

    // 类型层面的保证（编译期即成立）：`Hook::before_insert(&self, &mut Row) -> Result<()>`
    // 既不是 async，也不持有 `Db` / `Transaction`，Hook 内无法发起任何 IO。
    // 因此审计落库只能由服务层在显式事务内完成（见 AUD-006）。
}

/// AUD-006：审计写入失败 → 业务数据整体回滚。
#[tokio::test]
async fn aud_006_audit_write_failure_rolls_back_business_data() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let invoice = Invoice {
        id: 7101,
        tenant_id: TENANT_T1,
        customer_id: 107,
        status: InvoiceStatus::Draft,
        total: money(10, 0),
        metadata: Some(json!({ "tier": "gold" })),
        issued_at: None,
        paid_at: None,
        created_at: at(0),
        note: None,
    };
    let item = InvoiceItem {
        id: 71011,
        invoice_id: 7101,
        description: "subscription fee".into(),
        quantity: 1,
        unit_price: money(10, 0),
        amount: money(10, 0),
        tax_rate: "0".into(),
    };

    // 审计写入命中注入点 → 整个事务回滚
    {
        let _armed = Armed::new(FailPoint::AuditWrite);
        let err =
            invoice_svc::create_draft(c.db(), invoice.clone(), std::slice::from_ref(&item), 42)
                .await
                .expect_err("审计写入失败必须让 create_draft 失败");
        assert!(matches!(err, DbError::Transaction(_)), "实际 {err:?}");
    }

    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE tenant_id = ?",
            vec![Value::I64(TENANT_T1)]
        )
        .await,
        0,
        "业务行必须一并回滚"
    );
    assert_eq!(
        count_by(c.db(), "SELECT COUNT(*) FROM invoice_items", vec![]).await,
        0
    );
    assert_eq!(
        count_by(c.db(), "SELECT COUNT(*) FROM audit_logs", vec![]).await,
        0
    );

    // 控制组：解除注入后同一份数据可以正常落库（发票 + 条目 + 审计同一事务）
    invoice_svc::create_draft(c.db(), invoice, &[item], 42)
        .await
        .expect("控制组应成功");
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE tenant_id = ?",
            vec![Value::I64(TENANT_T1)]
        )
        .await,
        1
    );
    assert_eq!(audit::count_for(c.db(), "invoice", 7101).await.unwrap(), 1);
}

/// AUD-007：审计不可篡改——服务层无 UPDATE/DELETE 路径；模型层的真实行为按方言记录。
#[tokio::test]
async fn aud_007_no_audit_mutation_path() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    seed_audit(c.db(), 9301, TENANT_T1, "invoice", 7001, "issue", 42, at(0))
        .await
        .expect("seed audit");

    // 读路径：服务层 + 模型层都可用
    assert_eq!(audit::count_for(c.db(), "invoice", 7001).await.unwrap(), 1);
    let typed = AuditLog::find(c.db(), 9301)
        .await
        .expect("AuditLog 模型层读取");
    assert_eq!(typed.action, "issue");

    // 服务层 `services::audit` 只暴露 record / count_for / list_for（写路径仅 INSERT）。
    // 框架层 `AuditLog` 仍是普通 Model，`update` / `delete` 在编译期依旧可用——
    // 下面按方言记录真实结果，并断言「篡改可见 / 不可见」的一致性。
    let mut tampered = typed.clone();
    tampered.action = "tampered".into();
    match tampered.update(c.db()).await {
        Ok(()) => {
            let now = AuditLog::find(c.db(), 9301).await.unwrap();
            assert_eq!(now.action, "tampered", "update 成功后篡改必须可见");
            eprintln!(
                "[登记 W] {} 上 AuditLog::update 成功改写审计行：框架无不可变保护（无只读属性/触发器）",
                c.dialect.name()
            );
        }
        Err(e) => {
            assert!(
                matches!(e, DbError::Readonly(_) | DbError::Query { .. }),
                "拒绝写入的错误应是只读/数据库拒绝，实际 {e:?}"
            );
            let now = AuditLog::find(c.db(), 9301).await.unwrap();
            assert_eq!(now.action, "issue", "被拒绝的篡改不得改动数据");
            eprintln!(
                "[加固] {} 上数据库层拒绝了审计行的 UPDATE：{e}",
                c.dialect.name()
            );
        }
    }

    match typed.delete(c.db()).await {
        Ok(affected) => {
            assert_eq!(affected, 1);
            assert!(matches!(
                AuditLog::find(c.db(), 9301).await,
                Err(DbError::NotFound)
            ));
            eprintln!(
                "[登记 W] {} 上 AuditLog::delete 成功删除审计行：不可删除性仅靠项目约定（服务层无删除函数）",
                c.dialect.name()
            );
        }
        Err(e) => {
            assert!(
                matches!(e, DbError::Readonly(_) | DbError::Query { .. }),
                "实际 {e:?}"
            );
            assert!(AuditLog::find(c.db(), 9301).await.is_ok());
        }
    }
}

/// AUD-008：多维查询（实体 + 时间窗 + actor）返回预期行。
#[tokio::test]
async fn aud_008_multi_dimension_query() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_audit(
        c.db(),
        9001,
        TENANT_T2,
        "invoice",
        7001,
        "issue",
        42,
        at(50),
    )
    .await
    .expect("seed t2");

    // 种子：实体 / 时间 / actor 三个维度各自变化
    let seeds: [(i64, &str, i64, i64, i64); 6] = [
        (9601, "invoice", 7001, 42, 0), // 窗口内（下界命中的极值由 9601 承担）
        (9602, "invoice", 7001, 42, 50), // 窗口内
        (9603, "invoice", 7001, 7, 50), // actor 不同
        (9604, "invoice", 7002, 42, 50), // 实体不同
        (9605, "invoice", 7001, 42, 100), // 窗口上界（半开区间不命中）
        (9606, "invoice", 7001, 42, -1), // 早于下界
    ];
    for (id, entity_type, entity_id, actor_id, secs) in seeds {
        seed_audit(
            c.db(),
            id,
            TENANT_T1,
            entity_type,
            entity_id,
            "issue",
            actor_id,
            at(secs),
        )
        .await
        .expect("seed");
    }

    let query = "SELECT id FROM audit_logs WHERE tenant_id = ? AND entity_type = ? AND entity_id = ? \
                 AND actor_id = ? AND created_at >= ? AND created_at < ? ORDER BY id";
    let rows = c
        .db()
        .find_with_params(
            query,
            vec![
                Value::I64(TENANT_T1),
                Value::Text("invoice".into()),
                Value::I64(7001),
                Value::I64(42),
                at(0).to_value(),
                at(100).to_value(),
            ],
        )
        .await
        .expect("多维查询");
    let ids: Vec<i64> = rows
        .iter()
        .map(|r| rowx::i64_of(r, "id").unwrap())
        .collect();
    assert_eq!(ids, vec![9601, 9602], "四个维度同时生效（半开时间窗）");

    // 逐个放宽维度，观察结果如何变化
    let without_actor = c
        .db()
        .find_with_params(
            "SELECT id FROM audit_logs WHERE tenant_id = ? AND entity_type = ? AND entity_id = ? \
             AND created_at >= ? AND created_at < ? ORDER BY id",
            vec![
                Value::I64(TENANT_T1),
                Value::Text("invoice".into()),
                Value::I64(7001),
                at(0).to_value(),
                at(100).to_value(),
            ],
        )
        .await
        .expect("no actor");
    let ids: Vec<i64> = without_actor
        .iter()
        .map(|r| rowx::i64_of(r, "id").unwrap())
        .collect();
    assert_eq!(ids, vec![9601, 9602, 9603], "去掉 actor 维度后多出 9603");

    let wide_window = c
        .db()
        .find_with_params(
            "SELECT id FROM audit_logs WHERE tenant_id = ? AND entity_type = ? AND entity_id = ? \
             AND actor_id = ? ORDER BY id",
            vec![
                Value::I64(TENANT_T1),
                Value::Text("invoice".into()),
                Value::I64(7001),
                Value::I64(42),
            ],
        )
        .await
        .expect("no window");
    let ids: Vec<i64> = wide_window
        .iter()
        .map(|r| rowx::i64_of(r, "id").unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![9601, 9602, 9605, 9606],
        "去掉时间窗后包含边界外行"
    );

    let other_tenant = c
        .db()
        .find_with_params(
            "SELECT id FROM audit_logs WHERE tenant_id = ? AND actor_id = ? ORDER BY id",
            vec![Value::I64(TENANT_T2), Value::I64(42)],
        )
        .await
        .expect("cross tenant");
    assert_eq!(
        other_tenant
            .iter()
            .map(|r| rowx::i64_of(r, "id").unwrap())
            .collect::<Vec<_>>(),
        vec![9001],
        "租户维度隔离"
    );
    assert_tenant_scope(c.db(), TENANT_T1).await;
    assert_eq!(
        audit::list_for(c.db(), "invoice", 7001)
            .await
            .unwrap()
            .len(),
        6,
        "list_for 不带租户过滤：T1 的 5 条 + T2 的 1 条（服务层口径，租户隔离由调用方加 tenant_id）"
    );

    // 索引：`idx_audit_tenant_time (tenant_id, created_at)`（V0024）
    let explain = match c.dialect {
        Dialect::SqliteMem | Dialect::SqliteFile => format!(
            "EXPLAIN QUERY PLAN SELECT id FROM audit_logs WHERE tenant_id = {} AND created_at >= '2026-01-01 00:00:00' ORDER BY created_at",
            TENANT_T1
        ),
        _ => format!(
            "EXPLAIN SELECT id FROM audit_logs WHERE tenant_id = {} AND created_at >= '2026-01-01 00:00:00' ORDER BY created_at",
            TENANT_T1
        ),
    };
    match c.db().find(&explain).await {
        Ok(plan_rows) => {
            let plan = plan_rows
                .iter()
                .map(|r| {
                    r.values()
                        .map(|v| format!("{v:?}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join(" | ");
            eprintln!("[AUD-008] {} 执行计划: {plan}", c.dialect.name());
            if c.dialect.is_sqlite() {
                assert!(
                    plan.contains("idx_audit_tenant_time"),
                    "(tenant_id, created_at) 组合条件应命中 V0024 索引，实际计划: {plan}"
                );
            } else if c.dialect == Dialect::Postgres {
                // PG 计划文字里出现索引名即视为命中（小表可能退化为顺扫，故仅记录不断言）
                eprintln!(
                    "[AUD-008] PG 是否命中 idx_audit_tenant_time: {}",
                    plan.contains("idx_audit_tenant_time")
                );
            }
        }
        Err(e) => eprintln!("[AUD-008] {} 不支持 EXPLAIN: {e}", c.dialect.name()),
    }
}

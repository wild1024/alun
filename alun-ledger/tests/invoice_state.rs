//! M2 发票状态机（用例 INV-001..010，见《测试项目测试规划.md》§6.2）。
//!
//! 方言由 `ALUN_LEDGER_DB` 决定（默认 `sqlite-mem`）。金额一律经
//! [`alun_ledger::services::money`] / [`alun_ledger::services::rowx`] 读取
//! （SQLite 金额列为 `TEXT` 承载；D001 修复后类型化读取同样可用，适配器保留了
//! 「SQL 侧聚合不做浮点」的职责）。

mod common;

use alun_db::prelude::*;
use rust_decimal::Decimal;
use serde_json::json;

use alun_ledger::models::{
    BillingReport, EnumProbe, IntState, Invoice, InvoiceItem, InvoiceStatus, TextState,
};
use alun_ledger::services::{audit, invoice as invoice_svc, money, rowx};

use common::assertions::*;
use common::fixtures::*;
use common::harness::{LedgerCtx, ctx};

/// 审计/流转的操作人。
const ACTOR: i64 = 9001;

/// `#[native_enum("inv_native_state")]` 模式的**真实派生**。
///
/// 该派生曾因模板缺少匹配臂分隔符而无法编译（缺陷 D002，已修复）；
/// 现在 `Value::Enum`（PG 原生枚举列）与 `Value::Text`（SQLite 文本承载 / PG 标签按文本）
/// 都能解码，未知标签 → `InvalidValue`。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
#[native_enum("inv_native_state")]
enum NativeState {
    Draft,
    Issued,
    Paid,
}

/// 构造一张草稿发票（时间戳固定，便于断言）。
fn inv_row(
    id: i64,
    tenant_id: i64,
    total: Decimal,
    metadata: Option<serde_json::Value>,
) -> Invoice {
    Invoice {
        id,
        tenant_id,
        customer_id: tenant_id * 100 + 7,
        status: InvoiceStatus::Draft,
        total,
        metadata,
        issued_at: None,
        paid_at: None,
        created_at: at(0),
        note: Some("inv-fixture".into()),
    }
}

/// 构造一条金额自洽的条目（`amount = quantity × unit_price`）。
fn line(id: i64, invoice_id: i64, quantity: i64, unit_price: Decimal) -> InvoiceItem {
    InvoiceItem {
        id,
        invoice_id,
        description: format!("line-{id}"),
        quantity,
        unit_price,
        amount: unit_price * Decimal::from(quantity),
        tax_rate: "0.13".into(),
    }
}

/// 建草稿发票（金额与条目合计一致）。
async fn draft(c: &LedgerCtx, id: i64, items: &[InvoiceItem]) -> Invoice {
    let total = invoice_svc::items_total(items);
    invoice_svc::create_draft(c.db(), inv_row(id, TENANT_T1, total, None), items, ACTOR)
        .await
        .expect("create_draft")
}

/// 走到终态 `Paid` 的发票（100 元 / 1 条 2×50）。
async fn paid(c: &LedgerCtx, id: i64) -> Invoice {
    let items = [line(id * 10 + 1, id, 2, money(50, 0))];
    draft(c, id, &items).await;
    invoice_svc::transition(c.db(), TENANT_T1, id, InvoiceStatus::Issued, at(10), ACTOR)
        .await
        .expect("issue");
    invoice_svc::transition(c.db(), TENANT_T1, id, InvoiceStatus::Paid, at(20), ACTOR)
        .await
        .expect("pay")
}

async fn invoices_with_id(c: &LedgerCtx, id: i64) -> i64 {
    count_by(
        c.db(),
        "SELECT COUNT(*) FROM invoices WHERE id = ?",
        vec![Value::I64(id)],
    )
    .await
}

// ==================== INV-001 ====================

/// `Draft → Issued → Paid` 全链路：状态、`issued_at`/`paid_at`、每次流转 1 行审计。
#[tokio::test]
async fn inv_001_draft_issued_paid_writes_timestamps_and_audit_history() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let id = 1001;
    let items = [
        line(10_011, id, 2, money(30, 0)),
        line(10_012, id, 1, money(40, 0)),
    ];
    let created = invoice_svc::create_draft(
        c.db(),
        inv_row(id, TENANT_T1, money(100, 0), Some(json!({"tier": "gold"}))),
        &items,
        ACTOR,
    )
    .await
    .expect("create_draft");

    assert_eq!(created.status, InvoiceStatus::Draft, "创建后必须是 draft");
    assert_eq!(created.issued_at, None, "草稿无 issued_at");
    assert_eq!(created.paid_at, None, "草稿无 paid_at");
    assert_eq!(created.tenant_id, TENANT_T1);

    let issued =
        invoice_svc::transition(c.db(), TENANT_T1, id, InvoiceStatus::Issued, at(10), ACTOR)
            .await
            .expect("issue");
    assert_eq!(issued.status, InvoiceStatus::Issued);
    assert_eq!(issued.issued_at, Some(at(10)), "issued_at 必须写入流转时间");
    assert_eq!(issued.paid_at, None);

    let paid = invoice_svc::transition(c.db(), TENANT_T1, id, InvoiceStatus::Paid, at(20), ACTOR)
        .await
        .expect("pay");
    assert_eq!(paid.status, InvoiceStatus::Paid);
    assert_eq!(paid.issued_at, Some(at(10)), "paid 后 issued_at 必须保留");
    assert_eq!(paid.paid_at, Some(at(20)), "paid_at 必须写入流转时间");

    // 时间戳确实落到存储层（而非仅返回值）
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE id = ? AND status = 'paid' \
             AND issued_at IS NOT NULL AND paid_at IS NOT NULL",
            vec![Value::I64(id)],
        )
        .await,
        1
    );

    // 状态历史：create + issued + paid = 3 行，before/after 快照按序正确
    let history = audit::list_for(c.db(), "invoice", id).await.unwrap();
    let actions: Vec<&str> = history.iter().map(|h| h.action.as_str()).collect();
    assert_eq!(actions, vec!["create", "issued", "paid"], "历史行数 = 3");
    assert_eq!(history[0].actor_id, ACTOR);
    assert_eq!(history[0].before_json, None);
    assert_eq!(
        history[1].before_json.clone().unwrap(),
        json!({"status": "draft"})
    );
    assert_eq!(
        history[1].after_json.clone().unwrap(),
        json!({"status": "issued"})
    );
    assert_eq!(
        history[2].before_json.clone().unwrap(),
        json!({"status": "issued"})
    );
    assert_eq!(
        history[2].after_json.clone().unwrap(),
        json!({"status": "paid"})
    );
    assert_eq!(history[1].created_at, at(10), "审计时间 = 流转时间");

    // 条目与金额：读回值与写入一致
    let loaded_items = invoice_svc::items(c.db(), id).await.unwrap();
    assert_eq!(loaded_items, items.to_vec());
    assert_eq!(invoice_svc::items_total(&loaded_items), money(100, 0));
    let reloaded = invoice_svc::load(c.db(), TENANT_T1, id).await.unwrap();
    assert_eq!(reloaded.total, money(100, 0));
    assert_eq!(reloaded.note.as_deref(), Some("inv-fixture"));
    assert_eq!(reloaded.created_at, at(0));

    // 租户作用域：跨租户读取即 NotFound
    assert!(matches!(
        invoice_svc::load(c.db(), TENANT_T2, id).await,
        Err(DbError::NotFound)
    ));
}

// ==================== INV-002 ====================

/// 金额一致性：`total == Σ(quantity × unit_price)`，一致时 `Decimal` 无误差。
#[tokio::test]
async fn inv_002_total_must_equal_items_sum() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    // 不一致：条目合计 60 ≠ 发票 90 → 业务拒绝且不落任何行
    let bad_id = 1101;
    let bad_items = [line(11_011, bad_id, 2, money(30, 0))];
    let err = invoice_svc::create_draft(
        c.db(),
        inv_row(bad_id, TENANT_T1, money(90, 0), None),
        &bad_items,
        ACTOR,
    )
    .await
    .expect_err("金额不一致必须拒绝");
    match &err {
        DbError::InvalidValue(m) => assert!(
            m.contains("90") && m.contains("60"),
            "错误信息应带明细（total 与 items sum），实际: {m}"
        ),
        other => panic!("期望 InvalidValue，实际 {other:?}"),
    }
    assert_eq!(
        invoices_with_id(&c, bad_id).await,
        0,
        "拒绝后不得残留发票行"
    );
    assert_eq!(invoice_svc::item_count(c.db(), bad_id).await.unwrap(), 0);
    assert_eq!(
        audit::count_for(c.db(), "invoice", bad_id).await.unwrap(),
        0
    );

    // 一致：0.1 × 3 = 0.3（Decimal 精确，无二进制浮点误差）
    let id = 1102;
    let unit = Decimal::new(1, 1);
    let items = [line(11_021, id, 3, unit)];
    let created = draft(&c, id, &items).await;
    assert_eq!(created.total, Decimal::new(3, 1), "0.1 × 3 == 0.3");
    assert_eq!(invoice_svc::items_total(&items), Decimal::new(3, 1));
    let read_back = invoice_svc::load(c.db(), TENANT_T1, id)
        .await
        .unwrap()
        .total;
    assert_eq!(read_back, Decimal::new(3, 1), "Decimal 往返必须逐位相等");
    assert_ne!(
        read_back,
        Decimal::new(30_000_000_001, 11),
        "不得有浮点误差位"
    );

    // 适配器读回（方言无关）同样精确
    assert_eq!(
        money::read_one(
            c.db(),
            "SELECT total FROM invoices WHERE id = ? AND tenant_id = ?",
            vec![Value::I64(id), Value::I64(TENANT_T1)],
        )
        .await
        .unwrap(),
        Decimal::new(3, 1)
    );

    // add_item 也校验 amount == quantity × unit_price
    let bad_item = InvoiceItem {
        amount: money(99, 0),
        ..line(11_022, id, 2, money(10, 0))
    };
    let err = invoice_svc::add_item(c.db(), TENANT_T1, bad_item.clone(), at(5))
        .await
        .expect_err("条目金额不一致必须拒绝");
    assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
    assert_eq!(invoice_svc::item_count(c.db(), id).await.unwrap(), 1);
    assert_eq!(
        invoice_svc::load(c.db(), TENANT_T1, id)
            .await
            .unwrap()
            .total,
        Decimal::new(3, 1),
        "拒绝后 total 不得变化"
    );

    // 一致的新增条目：total 累加且条目落库
    let good_item = line(11_023, id, 2, money(10, 0));
    invoice_svc::add_item(c.db(), TENANT_T1, good_item.clone(), at(6))
        .await
        .expect("金额自洽的条目应通过");
    assert_eq!(invoice_svc::item_count(c.db(), id).await.unwrap(), 2);
    assert_eq!(
        invoice_svc::load(c.db(), TENANT_T1, id)
            .await
            .unwrap()
            .total,
        Decimal::new(3, 1) + money(20, 0)
    );
    let all = invoice_svc::items(c.db(), id).await.unwrap();
    assert_eq!(
        invoice_svc::items_total(&all),
        invoice_svc::load(c.db(), TENANT_T1, id)
            .await
            .unwrap()
            .total,
        "total 必须始终等于条目合计"
    );
}

// ==================== INV-003 ====================

/// 非法流转 `Paid → Draft`：业务拒绝且状态（含时间戳、审计）完全不变。
#[tokio::test]
async fn inv_003_illegal_transition_paid_to_draft_rejected_state_unchanged() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let id = 1201;
    let before = paid(&c, id).await;
    assert_eq!(before.status, InvoiceStatus::Paid);
    let audit_before = audit::count_for(c.db(), "invoice", id).await.unwrap();
    assert_eq!(audit_before, 3, "create + issued + paid");

    let err = invoice_svc::transition(c.db(), TENANT_T1, id, InvoiceStatus::Draft, at(30), ACTOR)
        .await
        .expect_err("Paid → Draft 必须拒绝");
    match &err {
        DbError::InvalidValue(m) => assert!(
            m.contains("illegal transition"),
            "错误应为非法流转，实际: {m}"
        ),
        other => panic!("期望 InvalidValue，实际 {other:?}"),
    }

    // 状态未变
    let after = invoice_svc::load(c.db(), TENANT_T1, id).await.unwrap();
    assert_eq!(after, before, "拒绝后发票行必须逐字段不变");
    assert_eq!(after.status, InvoiceStatus::Paid);
    assert_eq!(after.issued_at, Some(at(10)));
    assert_eq!(after.paid_at, Some(at(20)));

    // 审计未新增（拒绝路径不落审计）
    assert_eq!(
        audit::count_for(c.db(), "invoice", id).await.unwrap(),
        audit_before
    );

    // Paid 是终态：任何目标状态都被拒绝（含自身）
    for next in [
        InvoiceStatus::Draft,
        InvoiceStatus::Issued,
        InvoiceStatus::Paid,
        InvoiceStatus::Voided,
    ] {
        let e = invoice_svc::transition(c.db(), TENANT_T1, id, next, at(40), ACTOR)
            .await
            .expect_err("终态不得再流转");
        assert!(
            matches!(e, DbError::InvalidValue(_)),
            "{next:?} 应被拒绝，实际 {e:?}"
        );
    }
    assert_eq!(
        invoice_svc::load(c.db(), TENANT_T1, id).await.unwrap(),
        after
    );
}

// ==================== INV-004 ====================

/// `Voided`（及 `Paid`）之后禁止新增条目。
#[tokio::test]
async fn inv_004_no_items_after_voided_or_paid() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    // Voided 终态
    let voided_id = 1301;
    let items = [line(13_011, voided_id, 1, money(100, 0))];
    draft(&c, voided_id, &items).await;
    invoice_svc::transition(
        c.db(),
        TENANT_T1,
        voided_id,
        InvoiceStatus::Voided,
        at(5),
        ACTOR,
    )
    .await
    .expect("draft → voided");
    assert_eq!(
        invoice_svc::load(c.db(), TENANT_T1, voided_id)
            .await
            .unwrap()
            .status,
        InvoiceStatus::Voided
    );
    let total_before = invoice_svc::load(c.db(), TENANT_T1, voided_id)
        .await
        .unwrap()
        .total;
    let audit_before = audit::count_for(c.db(), "invoice", voided_id)
        .await
        .unwrap();
    assert_eq!(audit_before, 2, "create + voided");

    let err = invoice_svc::add_item(
        c.db(),
        TENANT_T1,
        line(13_012, voided_id, 1, money(50, 0)),
        at(6),
    )
    .await
    .expect_err("voided 后禁止新增条目");
    match &err {
        DbError::InvalidValue(m) => assert!(
            m.contains("cannot add items"),
            "错误应说明禁止新增，实际: {m}"
        ),
        other => panic!("期望 InvalidValue，实际 {other:?}"),
    }
    assert_eq!(invoice_svc::item_count(c.db(), voided_id).await.unwrap(), 1);
    assert_eq!(
        invoice_svc::load(c.db(), TENANT_T1, voided_id)
            .await
            .unwrap()
            .total,
        total_before,
        "拒绝后 total 不得变化"
    );
    assert_eq!(
        audit::count_for(c.db(), "invoice", voided_id)
            .await
            .unwrap(),
        audit_before,
        "拒绝路径不得写审计"
    );

    // Paid 同样禁止
    let paid_id = 1302;
    paid(&c, paid_id).await;
    let err = invoice_svc::add_item(
        c.db(),
        TENANT_T1,
        line(13_021, paid_id, 1, money(50, 0)),
        at(30),
    )
    .await
    .expect_err("paid 后禁止新增条目");
    assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
    assert_eq!(invoice_svc::item_count(c.db(), paid_id).await.unwrap(), 1);

    // 草稿态仍可新增（对照）
    let draft_id = 1303;
    draft(&c, draft_id, &[line(13_031, draft_id, 1, money(100, 0))]).await;
    invoice_svc::add_item(
        c.db(),
        TENANT_T1,
        line(13_032, draft_id, 1, money(50, 0)),
        at(7),
    )
    .await
    .expect("草稿态可新增条目");
    assert_eq!(invoice_svc::item_count(c.db(), draft_id).await.unwrap(), 2);
}

// ==================== INV-005 ====================

/// 枚举四模式落库语义：仅文本 / `value_map` 可往返；`#[int]` 与 `#[native_enum]`
/// 的读回类型由驱动归一决定（断言实际观测到的变体，不假设）。
#[tokio::test]
async fn inv_005_enum_encoding_modes_per_dialect() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    let dialect = c.dialect.name();

    // ---- ① value_map 模式（业务枚举 InvoiceStatus）：写 Text / 读 Text ----
    let id = 1401;
    draft(&c, id, &[line(14_011, id, 1, money(100, 0))]).await;
    let rows = c
        .db()
        .find_with_params(
            "SELECT status FROM invoices WHERE id = ?",
            vec![Value::I64(id)],
        )
        .await
        .unwrap();
    let raw_status = rowx::value(&rows[0], "status").unwrap().clone();
    assert!(
        matches!(&raw_status, Value::Text(s) if s == "draft"),
        "value_map 模式应落 Value::Text(\"draft\")，实际 {raw_status:?}"
    );
    assert_eq!(
        InvoiceStatus::from_value(&raw_status).unwrap(),
        InvoiceStatus::Draft
    );
    assert_eq!(
        InvoiceStatus::from_value(&Value::Text("paid".into())).unwrap(),
        InvoiceStatus::Paid
    );
    assert!(
        matches!(
            InvoiceStatus::from_value(&Value::I32(2)),
            Err(DbError::TypeMismatch {
                expected: "Text",
                ..
            })
        ),
        "value_map 只接受文本载荷"
    );
    assert_eq!(
        invoice_svc::load(c.db(), TENANT_T1, id)
            .await
            .unwrap()
            .status,
        InvoiceStatus::Draft
    );

    // ---- ② 默认文本模式 / ③ #[int] 模式（手写等价探针，派生本身编译失败=D2）----
    let enum_probe_ddl = match dialect {
        "sqlite-mem" | "sqlite-file" => {
            "CREATE TABLE IF NOT EXISTS enum_probe (id INTEGER PRIMARY KEY, \
             text_state TEXT NOT NULL, int_state INTEGER NOT NULL)"
        }
        "postgres" => {
            "CREATE TABLE IF NOT EXISTS enum_probe (id BIGINT PRIMARY KEY, \
             text_state TEXT NOT NULL, int_state INTEGER NOT NULL)"
        }
        _ => {
            "CREATE TABLE IF NOT EXISTS enum_probe (id BIGINT PRIMARY KEY, \
             text_state VARCHAR(32) NOT NULL, int_state INT NOT NULL)"
        }
    };
    c.db()
        .execute(enum_probe_ddl)
        .await
        .expect("create enum_probe");
    let probe_id = 9_000_141;
    c.db()
        .execute_with_params(
            "DELETE FROM enum_probe WHERE id = ?",
            vec![Value::I64(probe_id)],
        )
        .await
        .unwrap();
    c.db()
        .execute_with_params(
            "INSERT INTO enum_probe (id, text_state, int_state) VALUES (?, ?, ?)",
            vec![
                Value::I64(probe_id),
                TextState::Issued.to_value(),
                IntState::Issued.to_value(),
            ],
        )
        .await
        .expect("insert enum probe");
    let probe_rows = c
        .db()
        .find_with_params(
            "SELECT id, text_state, int_state FROM enum_probe WHERE id = ?",
            vec![Value::I64(probe_id)],
        )
        .await
        .unwrap();
    let row = &probe_rows[0];

    // 文本模式：写 Value::Text、读回 Value::Text → 往返成功
    let text_read = rowx::value(row, "text_state").unwrap().clone();
    assert_eq!(TextState::Issued.to_value().ty().name(), "Text");
    assert_eq!(
        text_read.ty().name(),
        "Text",
        "文本模式读回必须是 Text，实际 {text_read:?}"
    );
    assert_eq!(
        TextState::from_value(&text_read).unwrap(),
        TextState::Issued
    );

    // 整数模式：写侧是 Value::I32，三库整数列统一读回 Value::I64；
    // 判别值解码不再绑定列宽（D020 已修复）→ I64 同样可往返
    assert_eq!(IntState::Issued.to_value().ty().name(), "I32", "写侧为 I32");
    let int_read = rowx::value(row, "int_state").unwrap().clone();
    eprintln!(
        "[{dialect}] INV-005 #[int] 模式：写入 I32，读回 {}（驱动统一归一宽度，解码按判别值）",
        int_read.ty().name()
    );
    assert_eq!(int_read.ty().name(), "I64", "整数列读回宽度应为 I64");
    assert_eq!(
        IntState::from_value(&int_read).unwrap(),
        IntState::Issued,
        "#[int] 模式必须能读回归一后的 I64"
    );

    // Model 层同一路径：EnumProbe::from_row 必须成功
    let probe_model = EnumProbe::from_row(row.clone())
        .unwrap_or_else(|e| panic!("EnumProbe::from_row 应成功，实际 {e:?}"));
    assert_eq!(probe_model.int_state, IntState::Issued);
    assert_eq!(probe_model.text_state, TextState::Issued);

    // ---- ④ #[native_enum] 模式：仅数据库原生枚举列可用 ----
    let (native_ddl, native_col_type) = match dialect {
        "sqlite-mem" | "sqlite-file" => (
            "CREATE TABLE IF NOT EXISTS inv_native_probe (id INTEGER PRIMARY KEY, native_state TEXT NOT NULL)",
            "TEXT",
        ),
        "postgres" => {
            c.db()
                .execute(
                    "DO $$ BEGIN CREATE TYPE inv_native_state AS ENUM ('Draft', 'Issued', 'Paid'); \
                     EXCEPTION WHEN duplicate_object THEN NULL; END $$",
                )
                .await
                .expect("create pg enum type");
            (
                "CREATE TABLE IF NOT EXISTS inv_native_probe (id BIGINT PRIMARY KEY, native_state inv_native_state NOT NULL)",
                "inv_native_state",
            )
        }
        _ => (
            "CREATE TABLE IF NOT EXISTS inv_native_probe (id BIGINT PRIMARY KEY, \
             native_state ENUM('Draft', 'Issued', 'Paid') NOT NULL)",
            "ENUM",
        ),
    };
    c.db()
        .execute(native_ddl)
        .await
        .expect("create native probe");
    let native_id = 9_000_142;
    c.db()
        .execute_with_params(
            "DELETE FROM inv_native_probe WHERE id = ?",
            vec![Value::I64(native_id)],
        )
        .await
        .unwrap();
    let native_insert = if dialect == "postgres" {
        "INSERT INTO inv_native_probe (id, native_state) VALUES (?, CAST(? AS inv_native_state))"
    } else {
        "INSERT INTO inv_native_probe (id, native_state) VALUES (?, ?)"
    };
    c.db()
        .execute_with_params(
            native_insert,
            vec![Value::I64(native_id), NativeState::Issued.to_value()],
        )
        .await
        .expect("insert native probe");
    let native_rows = c
        .db()
        .find_with_params(
            "SELECT native_state FROM inv_native_probe WHERE id = ?",
            vec![Value::I64(native_id)],
        )
        .await
        .unwrap();
    let native_read = rowx::value(&native_rows[0], "native_state")
        .unwrap()
        .clone();
    eprintln!(
        "[{dialect}] INV-005 #[native_enum]（列 {native_col_type}）：写入 Enum，读回 {}",
        native_read.ty().name()
    );
    if c.dialect.is_sqlite() {
        assert_eq!(
            native_read.ty().name(),
            "Text",
            "SQLite 无原生枚举，列退化为 TEXT"
        );
    }
    // 读回形态随方言（PG 原生枚举列 → Value::Enum；SQLite 文本承载 → Value::Text），
    // 但类型化解码都必须成功且标签一致（D002 修复后 `from_value` 同时接受 Enum/Text）
    let state = NativeState::from_value(&native_read).unwrap_or_else(|e| {
        panic!("{dialect}: #[native_enum] 读回应成功（列 {native_col_type}），实际 {e:?}")
    });
    assert_eq!(state, NativeState::Issued);
    // 未知标签与非法载荷仍必须报错
    assert!(
        matches!(
            NativeState::from_value(&Value::Enum("Nope".into())),
            Err(DbError::InvalidValue(_))
        ),
        "未知标签应 InvalidValue"
    );
    assert!(
        matches!(
            NativeState::from_value(&Value::I64(2)),
            Err(DbError::TypeMismatch { .. })
        ),
        "非 Enum/Text 载荷应 TypeMismatch"
    );
    eprintln!(
        "[{dialect}] INV-005 #[native_enum]：读回类型 = {}，解码 = {state:?}（D002 已修复，真实派生）",
        native_read.ty().name()
    );

    // ---- ⑤ #[view] 语义：视图可查询，任何写操作 Readonly ----
    let paid_id = 1402;
    paid(&c, paid_id).await;
    let view_count = BillingReport::count(c.db()).await.unwrap();
    if c.dialect.is_sqlite() {
        assert_eq!(view_count, 1, "v_mrr 只含 paid 发票");
    } else {
        assert!(view_count >= 1, "v_mrr 至少含本用例的 paid 发票");
    }
    let view_rows = c
        .db()
        .find_with_params(
            "SELECT month, total, status FROM v_mrr WHERE tenant_id = ?",
            vec![Value::I64(TENANT_T1)],
        )
        .await
        .unwrap();
    assert!(!view_rows.is_empty(), "v_mrr 应能查到本用例的 paid 发票");
    for r in &view_rows {
        assert_eq!(
            rowx::text_of(r, "status").unwrap(),
            "paid",
            "v_mrr 只含 paid"
        );
        assert!(
            rowx::text_of(r, "month").unwrap().len() >= 7,
            "月份应为 YYYY-MM 文本"
        );
        assert!(rowx::decimal_of(r, "total").unwrap() >= Decimal::ZERO);
    }
    if c.dialect.is_sqlite() {
        // 内存库每个用例独立 → 精确断言
        assert_eq!(view_rows.len(), 1);
        assert_eq!(rowx::text_of(&view_rows[0], "month").unwrap(), "2026-01");
        assert_eq!(
            rowx::decimal_of(&view_rows[0], "total").unwrap(),
            money(100, 0)
        );
    }

    let mut report = BillingReport {
        tenant_id: TENANT_T1,
        month: "2026-01".into(),
        total: money(1, 0),
        status: "paid".into(),
    };
    let err = report
        .insert_or_update(c.db())
        .await
        .expect_err("视图模型不可写");
    assert!(
        matches!(err, DbError::Readonly("v_mrr")),
        "期望 Readonly(v_mrr)，实际 {err:?}"
    );
    let err = BillingReport::delete_by_pk(c.db(), Value::I64(TENANT_T1))
        .await
        .expect_err("视图模型不可删");
    assert!(matches!(err, DbError::Readonly(_)), "got {err:?}");
    assert_eq!(BillingReport::count(c.db()).await.unwrap(), view_count);
}

// ==================== INV-006 ====================

/// 未知枚举值：手工写入 `'bogus'` 后读取 → 显式错误，不 panic。
#[tokio::test]
async fn inv_006_unknown_enum_value_is_invalid_value_not_panic() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let id = 1501;
    draft(&c, id, &[line(15_011, id, 1, money(100, 0))]).await;
    c.db()
        .execute_with_params(
            "UPDATE invoices SET status = 'bogus' WHERE id = ?",
            vec![Value::I64(id)],
        )
        .await
        .expect("raw update");
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE id = ? AND status = 'bogus'",
            vec![Value::I64(id)],
        )
        .await,
        1,
        "框架不做写入侧枚举校验，脏值原样落库"
    );

    // 读取：#1 次
    let err = invoice_svc::load(c.db(), TENANT_T1, id)
        .await
        .expect_err("未知枚举值必须报错");
    match &err {
        DbError::InvalidValue(m) => assert!(m.contains("bogus"), "错误信息应含原始值，实际: {m}"),
        other => panic!("期望 InvalidValue（value_map 模式），实际 {other:?}"),
    }
    // 读取：#2 次（稳定复现，非一次性 panic）
    let again = invoice_svc::load(c.db(), TENANT_T1, id)
        .await
        .expect_err("二次读取同样报错");
    assert_eq!(format!("{again:?}"), format!("{err:?}"), "错误必须稳定");

    // 依赖 load 的写路径同样被拦（不会写入错误状态）
    let err = invoice_svc::transition(c.db(), TENANT_T1, id, InvoiceStatus::Issued, at(5), ACTOR)
        .await
        .expect_err("脏状态不得流转");
    assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE id = ? AND status = 'bogus'",
            vec![Value::I64(id)],
        )
        .await,
        1,
        "被拦截后脏值仍在（未被静默改写）"
    );

    // 三模式各自的错误变体（构造输入，跨方言恒定）
    assert!(matches!(
        InvoiceStatus::from_value(&Value::Text("bogus".into())),
        Err(DbError::InvalidValue(_))
    ));
    assert!(matches!(
        TextState::from_value(&Value::Text("bogus".into())),
        Err(DbError::InvalidValue(_))
    ));
    assert!(
        matches!(
            IntState::from_value(&Value::I32(99)),
            Err(DbError::InvalidValue(_))
        ),
        "#[int] 模式下 I32 越界应 InvalidValue"
    );
    // 驱动归一宽度不再是拒绝理由（D020 已修复）：I64 同样按判别值解码，越界才是 InvalidValue
    assert!(
        matches!(
            IntState::from_value(&Value::I64(99)),
            Err(DbError::InvalidValue(_))
        ),
        "#[int] 模式 I64 越界判别值应 InvalidValue"
    );
    assert_eq!(
        IntState::from_value(&Value::I64(2)).unwrap(),
        IntState::Issued,
        "驱动归一后的 I64 必须可解码"
    );
    // `#[native_enum]` 同时接受 Enum 与 Text 载荷（PG 枚举列按标签文本读回）
    assert!(
        matches!(
            NativeState::from_value(&Value::Text("bogus".into())),
            Err(DbError::InvalidValue(_))
        ),
        "未知标签（无论 Enum 还是 Text 载荷）应 InvalidValue"
    );
    assert!(
        matches!(
            NativeState::from_value(&Value::I64(1)),
            Err(DbError::TypeMismatch {
                expected: "Enum",
                ..
            })
        ),
        "非 Enum/Text 载荷应 TypeMismatch"
    );
}

// ==================== INV-007 ====================

/// JSON metadata 无损往返：5 层嵌套 / 100 元素数组 / emoji / 空对象 / `null`。
#[tokio::test]
async fn inv_007_json_metadata_roundtrip_distinguishes_null_states() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let deep =
        json!({"l1": {"l2": {"l3": {"l4": {"l5": {"leaf": "最深层", "n": 5, "ok": true}}}}}});
    let hundred: Vec<i64> = (0..100).collect();
    let metadata = json!({
        "nesting": deep,
        "hundred": hundred,
        "emoji": "🧾 发票 ✅ 🚀",
        "empty_object": {},
        "null_field": serde_json::Value::Null,
    });

    let id = 1601;
    let items = [line(16_011, id, 1, money(100, 0))];
    invoice_svc::create_draft(
        c.db(),
        inv_row(id, TENANT_T1, money(100, 0), Some(metadata.clone())),
        &items,
        ACTOR,
    )
    .await
    .expect("create_draft");

    let loaded = invoice_svc::load(c.db(), TENANT_T1, id).await.unwrap();
    let got = loaded.metadata.expect("metadata 应存在");
    assert_eq!(got, metadata, "JSON 必须逐字段无损往返");

    // 逐项复核
    assert_eq!(
        got["nesting"]["l1"]["l2"]["l3"]["l4"]["l5"]["leaf"],
        json!("最深层")
    );
    assert_eq!(got["nesting"]["l1"]["l2"]["l3"]["l4"]["l5"]["n"], json!(5));
    let arr = got["hundred"].as_array().expect("array");
    assert_eq!(arr.len(), 100);
    assert_eq!(got["hundred"], json!((0..100).collect::<Vec<i64>>()));
    assert_eq!(
        got["emoji"],
        json!("🧾 发票 ✅ 🚀"),
        "4 字节 emoji 不得损坏"
    );
    assert_eq!(got["empty_object"], json!({}));
    assert_eq!(got["empty_object"].as_object().unwrap().len(), 0);
    assert!(got["null_field"].is_null(), "JSON null 必须保持 null");

    // 第二张：metadata = JSON `null`（落库为文本 "null"，**不是** SQL NULL）
    let json_null_id = 1602;
    invoice_svc::create_draft(
        c.db(),
        inv_row(
            json_null_id,
            TENANT_T1,
            money(100, 0),
            Some(serde_json::Value::Null),
        ),
        &[line(16_021, json_null_id, 1, money(100, 0))],
        ACTOR,
    )
    .await
    .expect("create_draft(json null)");
    assert_eq!(
        invoice_svc::load(c.db(), TENANT_T1, json_null_id)
            .await
            .unwrap()
            .metadata,
        Some(serde_json::Value::Null),
        "JSON null 必须保持 Some(Null)，不得退化为 None"
    );

    // 第三张：metadata = SQL NULL → `None`
    let sql_null_id = 1603;
    invoice_svc::create_draft(
        c.db(),
        inv_row(sql_null_id, TENANT_T1, money(100, 0), None),
        &[line(16_031, sql_null_id, 1, money(100, 0))],
        ACTOR,
    )
    .await
    .expect("create_draft(null)");
    assert_eq!(
        invoice_svc::load(c.db(), TENANT_T1, sql_null_id)
            .await
            .unwrap()
            .metadata,
        None
    );

    // raw 层：JSON null 与 SQL NULL 的存储载荷不同
    let rows = c
        .db()
        .find_with_params(
            "SELECT id, metadata FROM invoices WHERE id IN (?, ?, ?) ORDER BY id",
            vec![
                Value::I64(id),
                Value::I64(json_null_id),
                Value::I64(sql_null_id),
            ],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    let rich_raw = rowx::value(&rows[0], "metadata").unwrap().clone();
    let json_null_raw = rowx::value(&rows[1], "metadata").unwrap().clone();
    let sql_null_raw = rowx::value(&rows[2], "metadata").unwrap().clone();
    eprintln!(
        "[{}] INV-007 raw 载荷：JSON null = {json_null_raw:?}，SQL NULL = {sql_null_raw:?}，完整文档 = {}",
        c.dialect.name(),
        rich_raw.ty().name()
    );
    assert_eq!(sql_null_raw, Value::Null, "SQL NULL 必须读回 Value::Null");
    assert_ne!(json_null_raw, Value::Null, "JSON null 不得与 SQL NULL 混淆");
    assert!(
        matches!(&json_null_raw, Value::Text(s) | Value::Json(s) if s == "null"),
        "JSON null 应以 \"null\" 文本载荷落库，实际 {json_null_raw:?}"
    );
    assert!(
        matches!(&rich_raw, Value::Text(_) | Value::Json(_)),
        "对象载荷应为文本/JSON，实际 {rich_raw:?}"
    );
    assert_ne!(rich_raw, json_null_raw, "空值载荷与文档载荷必须可区分");

    // 四态两两可区分：None / Some(null) / Some({}) / Some(完整文档)
    let states = [
        rowx::opt_json_of(&rows[2], "metadata").unwrap(),
        rowx::opt_json_of(&rows[1], "metadata").unwrap(),
        Some(json!({})),
        rowx::opt_json_of(&rows[0], "metadata").unwrap(),
    ];
    assert_eq!(states[0], None, "SQL NULL → None");
    assert_eq!(
        states[1],
        Some(serde_json::Value::Null),
        "JSON null → Some(Null)"
    );
    assert!(states[2].as_ref().unwrap().is_object());
    assert_eq!(states[3], Some(metadata.clone()));
    for i in 0..states.len() {
        for j in (i + 1)..states.len() {
            assert_ne!(states[i], states[j], "状态 {i} 与 {j} 必须可区分");
        }
    }
    assert_ne!(Some(json!(null)), None);
}

// ==================== INV-008 ====================

/// 关系加载（发票 → 条目）：断言正确性；SQL 往返次数在当前框架不可观测（缺 G10/G11）。
#[tokio::test]
async fn inv_008_relationship_loading_correctness_roundtrip_count_unobservable() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let ids = [1801_i64, 1802, 1803];
    for (n, id) in ids.iter().enumerate() {
        let items: Vec<InvoiceItem> = (0..3)
            .map(|k| {
                line(
                    id * 10 + k as i64 + 1,
                    *id,
                    k as i64 + 1,
                    money((n as i64 + 1) * 10, 0),
                )
            })
            .collect();
        draft(&c, *id, &items).await;
    }

    // 逐张加载（服务 API，按 id 升序）
    let mut per_invoice: Vec<Vec<InvoiceItem>> = Vec::new();
    for id in ids {
        per_invoice.push(invoice_svc::items(c.db(), id).await.unwrap());
    }

    // 单条 SQL 覆盖全部发票（按构造即 1 次往返）
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ");
    let sql = format!(
        "SELECT * FROM invoice_items WHERE invoice_id IN ({placeholders}) ORDER BY invoice_id, id"
    );
    let params = ids.iter().map(|id| Value::I64(*id)).collect::<Vec<Value>>();
    let rows = c.db().find_with_params(&sql, params).await.unwrap();
    assert_eq!(rows.len(), 9, "单条 SQL 覆盖 3 张发票的 9 个条目");

    let mut grouped: Vec<(i64, Vec<InvoiceItem>)> = Vec::new();
    for r in &rows {
        let invoice_id = rowx::i64_of(r, "invoice_id").unwrap();
        let item = InvoiceItem {
            id: rowx::i64_of(r, "id").unwrap(),
            invoice_id,
            description: rowx::text_of(r, "description").unwrap(),
            quantity: rowx::i64_of(r, "quantity").unwrap(),
            unit_price: rowx::decimal_of(r, "unit_price").unwrap(),
            amount: rowx::decimal_of(r, "amount").unwrap(),
            tax_rate: rowx::text_of(r, "tax_rate").unwrap(),
        };
        assert_eq!(
            item.amount,
            item.unit_price * Decimal::from(item.quantity),
            "条目金额自洽"
        );
        match grouped.last_mut() {
            Some((gid, items)) if *gid == invoice_id => items.push(item),
            _ => grouped.push((invoice_id, vec![item])),
        }
    }

    assert_eq!(grouped.len(), 3, "按 invoice_id 分组应得 3 组");
    for ((gid, batch_items), per) in grouped.iter().zip(per_invoice.iter()) {
        assert_eq!(
            batch_items, per,
            "发票 {gid} 的批量加载结果必须等于逐张加载"
        );
        assert!(
            batch_items.windows(2).all(|w| w[0].id < w[1].id),
            "条目必须按 id 升序"
        );
    }

    // 每张发票的 total 与条目合计一致（关系加载的业务不变量）
    for id in ids {
        let inv = invoice_svc::load(c.db(), TENANT_T1, id).await.unwrap();
        let its = invoice_svc::items(c.db(), id).await.unwrap();
        assert_eq!(its.len(), 3);
        assert_eq!(inv.total, invoice_svc::items_total(&its));
        assert_eq!(
            inv.total,
            money::read_one(
                c.db(),
                "SELECT COALESCE(SUM(amount), 0) FROM invoice_items WHERE invoice_id = ?",
                vec![Value::I64(id)],
            )
            .await
            .unwrap(),
            "发票 total 必须等于条目金额之和"
        );
    }

    // 空关系：无条目的发票返回空集合而非错误
    let empty_id = 1804;
    invoice_svc::create_draft(
        c.db(),
        inv_row(empty_id, TENANT_T1, Decimal::ZERO, None),
        &[],
        ACTOR,
    )
    .await
    .expect("0 条目草稿可创建");
    assert!(
        invoice_svc::items(c.db(), empty_id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(invoice_svc::item_count(c.db(), empty_id).await.unwrap(), 0);

    eprintln!(
        "[{}] INV-008 限制登记（缺 G10/G11）：框架无 SQL 语句计数器/执行钩子，`items()` 的实际往返次数不可观测 —— \
         本用例只能以「单条批量 SQL 结果 == 逐张加载结果」断言正确性，无法证明未产生 N+1",
        c.dialect.name()
    );
}

// ==================== INV-009 ====================

/// 级联删除：删发票时条目行为按方言实测（SQLite 驱动开启 `PRAGMA foreign_keys`）。
#[tokio::test]
async fn inv_009_delete_invoice_cascades_items_per_dialect() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let ids = [1901_i64, 1902];
    for id in ids {
        let items = [
            line(id * 10 + 1, id, 1, money(10, 0)),
            line(id * 10 + 2, id, 2, money(20, 0)),
        ];
        draft(&c, id, &items).await;
    }
    assert_eq!(invoice_svc::item_count(c.db(), ids[0]).await.unwrap(), 2);

    // 前置事实：外键约束确实生效（否则「级联」结论无意义）
    let orphan = line(1_999_999, 999_999, 1, money(1, 0));
    let fk_err = c
        .db()
        .execute_with_params(
            "INSERT INTO invoice_items (id, invoice_id, description, quantity, unit_price, amount, tax_rate) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(orphan.id),
                Value::I64(orphan.invoice_id),
                Value::Text(orphan.description.clone()),
                Value::I64(orphan.quantity),
                orphan.unit_price.to_value(),
                orphan.amount.to_value(),
                Value::Text(orphan.tax_rate.clone()),
            ],
        )
        .await
        .expect_err("孤儿条目必须被外键拒绝");
    eprintln!("[{}] INV-009 孤儿条目被拒绝：{fk_err:?}", c.dialect.name());
    assert!(
        matches!(
            fk_err,
            DbError::ForeignKeyViolation { .. } | DbError::Query { .. }
        ),
        "got {fk_err:?}"
    );

    // ON DELETE CASCADE（DDL 显式声明）
    let affected = invoice_svc::delete(c.db(), TENANT_T1, ids[0])
        .await
        .unwrap();
    assert_eq!(affected, 1, "删除影响 1 行发票");
    let left = invoice_svc::item_count(c.db(), ids[0]).await.unwrap();
    eprintln!(
        "[{}] INV-009 删票后剩余条目 = {left}（期望 0）",
        c.dialect.name()
    );
    assert_eq!(left, 0, "条目应随发票级联删除（ON DELETE CASCADE）");
    assert!(invoice_svc::items(c.db(), ids[0]).await.unwrap().is_empty());
    assert!(matches!(
        invoice_svc::load(c.db(), TENANT_T1, ids[0]).await,
        Err(DbError::NotFound)
    ));
    // 其他发票不受影响
    assert_eq!(invoice_svc::item_count(c.db(), ids[1]).await.unwrap(), 2);
    // 幂等：重复删除影响 0 行
    assert_eq!(
        invoice_svc::delete(c.db(), TENANT_T1, ids[0])
            .await
            .unwrap(),
        0
    );
    // 审计历史不随业务行删除（`audit_logs` 无外键，只写不改）
    assert_eq!(
        audit::count_for(c.db(), "invoice", ids[0]).await.unwrap(),
        1
    );

    eprintln!(
        "[{}] INV-009：`Invoice` 无 `#[soft_delete]`（表无 deleted_at 列），\
         软删除与 CASCADE 的冲突场景在当前实现中不存在",
        c.dialect.name()
    );
}

// ==================== INV-010 ====================

/// 并发状态流转：10 个并发请求推进同一发票到 `Paid`，断言成功次数与最终态一致。
#[tokio::test]
async fn inv_010_concurrent_transitions_serialize_to_single_winner() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let id = 2001;
    let items = [line(20_011, id, 1, money(100, 0))];
    invoice_svc::create_draft(
        c.db(),
        inv_row(id, TENANT_T1, money(100, 0), None),
        &items,
        ACTOR,
    )
    .await
    .expect("create_draft");
    invoice_svc::transition(c.db(), TENANT_T1, id, InvoiceStatus::Issued, at(10), ACTOR)
        .await
        .expect("issue");

    let mut tasks = Vec::new();
    for k in 0..10_i64 {
        let db = c.db().clone();
        tasks.push(tokio::spawn(async move {
            let secs = 100 + k;
            let res =
                invoice_svc::transition(&db, TENANT_T1, id, InvoiceStatus::Paid, at(secs), ACTOR)
                    .await;
            (secs, res)
        }));
    }

    let mut winners: Vec<(i64, Invoice)> = Vec::new();
    let mut losers: Vec<DbError> = Vec::new();
    for t in tasks {
        match t.await.expect("并发任务不得 panic") {
            (secs, Ok(inv)) => winners.push((secs, inv)),
            (_, Err(e)) => losers.push(e),
        }
    }

    assert_eq!(winners.len() + losers.len(), 10, "并发请求不得丢失");
    assert!(!winners.is_empty(), "至少一次流转必须成功");
    for (secs, inv) in &winners {
        assert_eq!(inv.status, InvoiceStatus::Paid);
        assert_eq!(inv.paid_at, Some(at(*secs)), "胜者必须写入自己的时间戳");
        assert_eq!(inv.issued_at, Some(at(10)), "issued_at 不得被并发请求破坏");
    }
    let mut error_kinds: Vec<String> = Vec::new();
    for e in &losers {
        // 失败原因必须可解释：状态已被抢先推进（InvalidValue）、审计主键冲突回滚
        // （UniqueViolation），或共享库上的锁竞争（D013 修复后有专用变体：
        // Deadlock / LockTimeout / SerializationFailure）。
        // 实测（sqlite-mem）：全部失败者均为 UniqueViolation —— 10 个请求都通过了
        // `load` 的状态检查（读后写无 CAS），最终只有 1 个审计行能插进去。
        assert!(
            matches!(
                e,
                DbError::InvalidValue(_)
                    | DbError::UniqueViolation { .. }
                    | DbError::Deadlock { .. }
                    | DbError::LockTimeout { .. }
                    | DbError::SerializationFailure { .. }
                    | DbError::Transaction(_)
            ),
            "非预期的并发失败: {e:?}"
        );
        error_kinds.push(match e {
            DbError::InvalidValue(_) => "InvalidValue(状态已推进)".to_string(),
            DbError::UniqueViolation { .. } => "UniqueViolation(审计主键)".to_string(),
            DbError::Deadlock { .. } => "Deadlock".to_string(),
            DbError::LockTimeout { .. } => "LockTimeout".to_string(),
            DbError::SerializationFailure { .. } => "SerializationFailure".to_string(),
            _ => "Transaction".to_string(),
        });
    }

    let final_inv = invoice_svc::load(c.db(), TENANT_T1, id).await.unwrap();
    assert_eq!(final_inv.status, InvoiceStatus::Paid, "最终状态必须确定");
    assert!(
        winners
            .iter()
            .any(|(secs, _)| final_inv.paid_at == Some(at(*secs))),
        "最终 paid_at 必须来自某个胜者，实际 {:?}",
        final_inv.paid_at
    );
    let paid_audit = audit::list_for(c.db(), "invoice", id)
        .await
        .unwrap()
        .into_iter()
        .filter(|h| h.action == "paid")
        .count();
    assert_eq!(
        paid_audit as i64,
        winners.len() as i64,
        "审计行数必须等于成功次数（失败者整体回滚）"
    );
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE id = ? AND status = 'paid'",
            vec![Value::I64(id)],
        )
        .await,
        1
    );

    eprintln!(
        "[{}] INV-010：10 并发流转，胜者 = {}，失败 = {}（分布: {:?}）；框架无乐观锁（缺 G5），\
         本方言下由审计主键唯一性/连接串行化决定结果",
        c.dialect.name(),
        winners.len(),
        losers.len(),
        error_kinds
    );
    assert_eq!(winners.len(), 1, "并发流转必须恰好 1 次成功");
}

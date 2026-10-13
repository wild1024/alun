//! M7 对账批处理（用例 REC-001..007）。
//!
//! 目标方言由 `ALUN_LEDGER_DB` 选择（默认 `sqlite-mem`）。

mod common;

use alun_db::prelude::*;
use alun_db_core::expr::Expr;
use alun_db_core::statement::InsertStmt;
use alun_db_core::stmt::Value;
use alun_db_core::traits::dialect::RenderCtx;
use alun_db_sql::render::insert::render_insert;
use rust_decimal::Decimal;

use alun_ledger::models::{Account, Invoice, InvoiceStatus};
use alun_ledger::services::{faults::FailPoint, reconcile, rowx};

use common::assertions::*;
use common::faults::{arm, disarm};
use common::fixtures::*;
use common::harness::{Dialect, ctx};

fn rec_invoice(id: i64, tenant_id: i64) -> Invoice {
    Invoice {
        id,
        tenant_id,
        customer_id: tenant_id * 100 + 7,
        status: InvoiceStatus::Draft,
        total: money(10, 0),
        metadata: None,
        issued_at: None,
        paid_at: None,
        created_at: at(0),
        note: None,
    }
}

// ==================== REC-001 ====================

/// 1000 条合并为单条 INSERT（语句级不可观测 → 用公共渲染器断言）。
#[tokio::test]
async fn rec_001_thousand_rows_render_as_single_insert() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let batch: Vec<Invoice> = (0..1000)
        .map(|i| rec_invoice(8_000 + i, TENANT_T1))
        .collect();
    Invoice::insert_many(&batch, c.db())
        .await
        .expect("insert_many");
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE tenant_id = ?",
            vec![Value::I64(TENANT_T1)]
        )
        .await,
        1000,
        "1000 行全部落库"
    );

    // 语句计数在框架内不可观测（Db 非 trait、无代理 executor、无 pool_stats，G10/G11）；
    // 退而断言「渲染结果确实是一条含 1000 个 VALUES 元组的 INSERT」。
    let columns = [
        "id",
        "tenant_id",
        "customer_id",
        "status",
        "total",
        "metadata",
        "issued_at",
        "paid_at",
        "created_at",
        "note",
    ];
    let values: Vec<Vec<Expr>> = (0..1000)
        .map(|_| columns.iter().map(|_| Expr::Value(Value::I64(1))).collect())
        .collect();
    let stmt = InsertStmt {
        table: "invoices",
        columns: columns.to_vec(),
        values,
        returning: None,
        on_conflict: None,
    };
    let mut rctx = RenderCtx::new(c.db().sql_dialect());
    let sql = render_insert(&stmt, &mut rctx);
    assert_eq!(sql.matches("INSERT INTO").count(), 1, "单一 INSERT 语句");
    assert_eq!(sql.matches("), (").count(), 999, "1000 个 VALUES 元组");
    assert_eq!(rctx.params.len(), 10_000, "10 列 × 1000 行参数一一对应");
    assert!(!sql.contains("ON CONFLICT"));
    assert!(!sql.contains("RETURNING"));
}

// ==================== REC-002 ====================

/// 循环内复用同一 SQL 文本（语句级不可观测，登记 G10）。
#[tokio::test]
async fn rec_002_loop_reuses_same_sql_text() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let accounts: Vec<Account> = (0..1000)
        .map(|i| Account {
            id: 2_000_000 + i,
            tenant_id: TENANT_T1,
            code: format!("RC{i:04}"),
            name: format!("acct-{i}"),
            kind: Account::KIND_ASSET.into(),
            balance: money(10, 0),
            currency: "CNY".into(),
        })
        .collect();
    Account::insert_many(&accounts, c.db())
        .await
        .expect("bulk accounts");
    let expected: Vec<(i64, Decimal)> = accounts.iter().map(|a| (a.id, a.balance)).collect();

    let summary = reconcile::reconcile(c.db(), TENANT_T1, &expected, 0, 250, at(0), |_| {})
        .await
        .expect("reconcile");
    assert_eq!(summary.processed, 1000);
    assert_eq!(summary.checked, 1000, "同一查询路径服务全部行");
    assert_eq!(summary.diffs, 0);

    // 登记：服务层循环内 SQL 文本为固定字面量，但 framework 未暴露语句计数/代理 executor。
    eprintln!(
        "[G10 登记] 无法观测底层语句次数（Db 非 trait、无代理 executor、无 pool_stats）。\
         循环内 SQL 文本固定为 `SELECT balance FROM accounts WHERE id = ? AND tenant_id = ?`，\
         由源码核对而非运行时计数。"
    );
}

// ==================== REC-003 ====================

/// 长批次对账不耗尽连接池（需要 PG：池观测能力缺口 M-3）。
#[tokio::test]
async fn rec_003_long_batch_keeps_pool_serving() {
    let Some(c) = ctx().await else { return };
    if !c.require(Dialect::Postgres) {
        eprintln!(
            "跳过 REC-003：连接池观测需要 PostgreSQL（当前 {}；框架缺 pool_stats，登记为 M-3）",
            c.dialect.name()
        );
        return;
    }
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    // 单语句参数上限（PG 65535）：分批 insert_many
    let total = 20_000i64;
    for start in (0..total).step_by(5_000) {
        let rows: Vec<Account> = (start..(start + 5_000).min(total))
            .map(|i| Account {
                id: 3_000_000 + i,
                tenant_id: TENANT_T1,
                code: format!("LG{i:05}"),
                name: "long".into(),
                kind: Account::KIND_ASSET.into(),
                balance: money(1, 0),
                currency: "CNY".into(),
            })
            .collect();
        Account::insert_many(&rows, c.db())
            .await
            .expect("chunk insert");
    }

    let expected: Vec<(i64, Decimal)> = (0..total).map(|i| (3_000_000 + i, money(1, 0))).collect();
    let other = c.db().clone();
    let (summary, served) = tokio::join!(
        reconcile::reconcile(c.db(), TENANT_T1, &expected, 0, 1_000, at(0), |_| {}),
        async move { other.scalar_with_params::<i64>("SELECT 1", vec![]).await }
    );
    let summary = summary.expect("reconcile");
    assert_eq!(summary.checked, total as usize);
    assert_eq!(served.expect("对账期间另一连接仍可服务"), 1);
}

// ==================== REC-004 ====================

/// 重复重跑幂等：差异表行数不变。
#[tokio::test]
async fn rec_004_rerun_is_idempotent() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(100, 0)).await.unwrap();

    let expected = vec![
        (account_id(TENANT_T1, 1), money(90, 0)),
        (account_id(TENANT_T1, 2), Decimal::ZERO),
        (account_id(TENANT_T1, 3), money(7, 0)),
    ];

    let first = reconcile::reconcile(c.db(), TENANT_T1, &expected, 0, 2, at(0), |_| {})
        .await
        .expect("first");
    assert_eq!(first.diffs, 2);
    assert_eq!(reconcile::diff_count(c.db(), TENANT_T1).await.unwrap(), 2);

    let second = reconcile::reconcile(c.db(), TENANT_T1, &expected, 0, 2, at(60), |_| {})
        .await
        .expect("second");
    assert_eq!(second.diffs, 0, "重跑不再新增差异");
    assert_eq!(
        reconcile::diff_count(c.db(), TENANT_T1).await.unwrap(),
        2,
        "差异表行数不变"
    );
}

// ==================== REC-005 ====================

/// 差异标记与汇总报告：汇总数字 == 明细聚合。
#[tokio::test]
async fn rec_005_diff_marking_matches_summary() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(100, 0)).await.unwrap();

    let a1 = account_id(TENANT_T1, 1);
    let a2 = account_id(TENANT_T1, 2);
    let a3 = account_id(TENANT_T1, 3);
    let expected = vec![(a1, money(90, 0)), (a2, Decimal::ZERO), (a3, money(7, 0))];

    let summary = reconcile::reconcile(c.db(), TENANT_T1, &expected, 0, 3, at(0), |_| {})
        .await
        .expect("reconcile");
    assert_eq!(
        summary,
        reconcile::ReconcileSummary {
            processed: 3,
            checked: 3,
            diffs: 2
        },
        "a1/a3 是差异，a2 无差异"
    );
    assert_eq!(
        reconcile::abs_diff_total(c.db(), TENANT_T1, &expected)
            .await
            .unwrap(),
        Decimal::from(17),
        "Σ|expected - actual| = |90-100| + |7-0|"
    );
    assert_eq!(reconcile::account_code(c.db(), a3).await.unwrap(), "3001");

    // 明细（audit_logs 载荷）聚合 == 汇总数字
    let rows = c
        .db()
        .find_with_params(
            "SELECT entity_id, after_json FROM audit_logs WHERE tenant_id = ? AND action = ? ORDER BY entity_id",
            vec![
                Value::I64(TENANT_T1),
                Value::Text("reconcile_diff".into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "每个差异恰好一行");
    let mut total = Decimal::ZERO;
    for row in &rows {
        let payload = rowx::opt_json_of(row, "after_json")
            .unwrap()
            .expect("payload");
        let want: Decimal = payload["expected"].as_str().unwrap().parse().unwrap();
        let actual: Decimal = payload["actual"].as_str().unwrap().parse().unwrap();
        total += (want - actual).abs();
    }
    assert_eq!(
        total,
        reconcile::abs_diff_total(c.db(), TENANT_T1, &expected)
            .await
            .unwrap(),
        "汇总数字 = 明细聚合"
    );
    assert_eq!(
        rowx::i64_of(&rows[0], "entity_id").unwrap(),
        a1,
        "差异按账户 id 记录"
    );
}

// ==================== REC-006 ====================

/// 进度可观测：每批一次回调 + 最终汇总。
#[tokio::test]
async fn rec_006_progress_callback_per_batch() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(100, 0)).await.unwrap();

    let expected: Vec<(i64, Decimal)> = (1..=5)
        .map(|n| {
            let want = if n == 1 { money(100, 0) } else { Decimal::ZERO };
            (account_id(TENANT_T1, n), want)
        })
        .collect();

    let mut seen: Vec<usize> = Vec::new();
    let summary = reconcile::reconcile(c.db(), TENANT_T1, &expected, 0, 2, at(0), |done| {
        seen.push(done)
    })
    .await
    .expect("reconcile");

    assert_eq!(seen, vec![2, 4, 5], "批次推进：2 → 4 → 5（尾批不满）");
    assert_eq!(
        summary,
        reconcile::ReconcileSummary {
            processed: 5,
            checked: 5,
            diffs: 0
        }
    );
}

// ==================== REC-007 ====================

/// 中断续跑：从检查点续跑不重复计入。
#[tokio::test]
async fn rec_007_interrupt_then_resume_without_double_counting() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(100, 0)).await.unwrap();

    // a1 = 100，其余为 0；期望里 a3/a5 有差异 → 差异落在索引 2 与 4
    let expected: Vec<(i64, Decimal)> = vec![
        (account_id(TENANT_T1, 1), money(100, 0)),
        (account_id(TENANT_T1, 2), Decimal::ZERO),
        (account_id(TENANT_T1, 3), money(5, 0)),
        (account_id(TENANT_T1, 4), Decimal::ZERO),
        (account_id(TENANT_T1, 5), money(9, 0)),
    ];

    // 批次 [0,2) 无差异 → 检查点 2；批次 [2,4) 命中注入点 → 中断
    arm(FailPoint::AuditWrite);
    let mut checkpoint = 0usize;
    let err = reconcile::reconcile(c.db(), TENANT_T1, &expected, 0, 2, at(0), |done| {
        checkpoint = done
    })
    .await
    .expect_err("注入点必须中断对账");
    disarm();
    assert!(matches!(err, DbError::Transaction(_)), "got {err:?}");
    assert_eq!(checkpoint, 2, "中断前完成的批次即为续跑起点");
    assert_eq!(
        reconcile::diff_count(c.db(), TENANT_T1).await.unwrap(),
        0,
        "被中断的差异写入整体回滚"
    );

    // 从检查点续跑：只处理未完成部分
    let resumed = reconcile::reconcile(c.db(), TENANT_T1, &expected, checkpoint, 2, at(0), |_| {})
        .await
        .expect("resume");
    assert_eq!(resumed.processed, 5, "处理到全集末尾");
    assert_eq!(resumed.checked, 3, "只校验剩余 3 行，未重复计入");
    assert_eq!(resumed.diffs, 2);
    assert_eq!(
        reconcile::diff_count(c.db(), TENANT_T1).await.unwrap(),
        2,
        "每个差异恰好一行"
    );

    // 全程重跑仍幂等
    let full = reconcile::reconcile(c.db(), TENANT_T1, &expected, 0, 5, at(60), |_| {})
        .await
        .expect("full rerun");
    assert_eq!(full.processed, 5);
    assert_eq!(full.diffs, 0);
    assert_eq!(
        reconcile::diff_count(c.db(), TENANT_T1).await.unwrap(),
        2,
        "续跑 + 重跑后无重复计入"
    );
}

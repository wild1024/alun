//! M6 报表（用例 RPT-001..008）。
//!
//! ## 框架/方言现状（用例内逐条标注，报告汇总）
//!
//! - **G7（无 CTE / 窗口函数）**：`Expr` 没有 CTE 或窗口函数节点，
//!   `Function` 只有 `Count/Sum/Avg/Min/Max/Coalesce/Lower/Upper/Length/Trim/Now/Custom`。
//!   因此「CTE」只能走 raw SQL，「累计求和 / LAG / RANK」只能落回 Rust 计算。
//! - **金额精度红线**：SQLite 金额列是 `TEXT`，SQL 侧 `SUM()` 会被强制转 `REAL`
//!   （浮点误差），所以报表的金额聚合一律在 Rust 侧用 `Decimal` 完成。
//! - `PageQuery` 只把 `page_size` 规整到 `[1, 1000]`，`page` **无上限**
//!   （`alun-web/src/api.rs`）。

mod common;

use alun_db::prelude::*;
use alun_db_core::expr::{BinaryOp, Expr};
use alun_db_core::statement::Order;
use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use std::time::Instant;

use alun_ledger::models::report::BillingReport;
use alun_ledger::services::{ledger, money, report, rowx};

use common::assertions::*;
use common::fixtures::*;
use common::harness::{LedgerCtx, ctx};

// ==================== 助手 ====================

/// 基准时间 + `days` 天。
fn at_days(days: i64) -> DateTime<Utc> {
    at(days * 86_400)
}

fn month_start(month: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, month, 1, 0, 0, 0).unwrap()
}

/// 直接插入一张发票（金额/时间显式指定，便于构造报表数据）。
async fn insert_invoice(
    c: &LedgerCtx,
    id: i64,
    tenant_id: i64,
    status: &str,
    total: Decimal,
    created_at: DateTime<Utc>,
    metadata: Option<&serde_json::Value>,
) {
    c.db()
        .execute_with_params(
            "INSERT INTO invoices (id, tenant_id, customer_id, status, total, metadata, issued_at, paid_at, created_at, note) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(id),
                Value::I64(tenant_id),
                Value::I64(tenant_id * 100 + 7),
                Value::Text(status.to_string()),
                total.to_value(),
                match metadata {
                    Some(j) => Value::Json(j.to_string()),
                    None => Value::Null,
                },
                Value::Null,
                Value::Null,
                created_at.to_value(),
                Value::Null,
            ],
        )
        .await
        .expect("insert invoice");
}

/// 插入订阅（自定义 `customer_id`：`uq_subscriptions_period` 约束
/// `(tenant_id, customer_id, period_start)` 要求三者组合唯一）。
async fn insert_subscription(
    c: &LedgerCtx,
    sub_id: i64,
    customer_id: i64,
    period_start: DateTime<Utc>,
    period_end: DateTime<Utc>,
    status: &str,
) {
    c.db()
        .execute_with_params(
            "INSERT INTO subscriptions (id, tenant_id, customer_id, plan, period_start, period_end, status) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(sub_id),
                Value::I64(TENANT_T1),
                Value::I64(customer_id),
                Value::Text("pro".into()),
                period_start.to_value(),
                period_end.to_value(),
                Value::Text(status.to_string()),
            ],
        )
        .await
        .expect("insert subscription");
}

/// 某月的「活跃订阅数」（期租重叠口径：`period_start <= 月初 < period_end`）。
async fn active_at(c: &LedgerCtx, tenant_id: i64, month: DateTime<Utc>) -> i64 {
    count_by(
        c.db(),
        "SELECT COUNT(*) FROM subscriptions WHERE tenant_id = ? AND period_start <= ? AND period_end > ?",
        vec![
            Value::I64(tenant_id),
            month.to_value(),
            month.to_value(),
        ],
    )
    .await
}

// ==================== 用例 ====================

/// RPT-001：MRR 趋势——CTE（`v_mrr`）取数 + Rust 侧 Decimal 累计求和。
///
/// AST 层无 CTE、无窗口函数（G7）：CTE 走 raw SQL，累计在 Rust 计算；
/// 本用例同时证明「SQL 侧 `SUM()` 在 SQLite 上会丢精度」，即不做 SQL 聚合的原因。
#[tokio::test]
async fn rpt_001_mrr_trend_cte_and_rust_cumulative() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_tenant(&c, TENANT_T3, "T3").await.unwrap();

    insert_invoice(&c, 5001, TENANT_T1, "paid", money(10, 0), at_days(4), None).await;
    insert_invoice(&c, 5002, TENANT_T1, "paid", money(30, 0), at_days(40), None).await;
    insert_invoice(&c, 5003, TENANT_T1, "paid", money(5, 50), at_days(78), None).await;
    // 非 paid：视图不含
    insert_invoice(
        &c,
        5004,
        TENANT_T1,
        "draft",
        money(99, 0),
        at_days(40),
        None,
    )
    .await;
    // 其他租户：租户维度过滤
    insert_invoice(&c, 5005, TENANT_T3, "paid", money(77, 0), at_days(4), None).await;

    let rows = report::mrr_by_month(c.db(), TENANT_T1).await.expect("mrr");
    assert_eq!(
        rows,
        vec![
            ("2026-01".to_string(), money(10, 0)),
            ("2026-02".to_string(), money(30, 0)),
            ("2026-03".to_string(), money(5, 50)),
        ],
        "CTE 取数：按自然月升序、仅 paid、仅本租户"
    );

    let cumulative = report::cumulative(&rows);
    assert_eq!(
        cumulative,
        vec![
            ("2026-01".to_string(), money(10, 0)),
            ("2026-02".to_string(), money(40, 0)),
            ("2026-03".to_string(), money(45, 50)),
        ],
        "Rust 侧 Decimal 累计求和（替代 SUM() OVER）"
    );

    // AST 路径：FindBuilder 能查视图，但没有 CTE/窗口函数节点，只能做等价过滤
    let tenant_column = Expr::Column {
        table: "v_mrr",
        column: "tenant_id",
    };
    let via_ast = FindBuilder::new()
        .table("v_mrr")
        .filter(Expr::Binary {
            left: Box::new(tenant_column),
            op: BinaryOp::Eq,
            right: Box::new(expr_value(Value::I64(TENANT_T1))),
        })
        .order_by("month", Order::Asc)
        .exec(c.db())
        .await
        .expect("AST 查询视图");
    let ast_rows: Vec<(String, Decimal)> = via_ast
        .iter()
        .map(|r| {
            (
                rowx::text_of(r, "month").unwrap(),
                rowx::decimal_of(r, "total").unwrap(),
            )
        })
        .collect();
    assert_eq!(ast_rows, rows, "AST 过滤结果应与 CTE 版本逐行一致");

    // 精度红线：SQLite 的 TEXT 金额列上 SUM() 被强制转 REAL（0.1 + 0.2 != 0.30）
    insert_invoice(&c, 5011, TENANT_T3, "paid", money(0, 10), at_days(4), None).await;
    insert_invoice(&c, 5012, TENANT_T3, "paid", money(0, 20), at_days(40), None).await;
    let sql_sum = money::read_one(
        c.db(),
        "SELECT SUM(total) FROM invoices WHERE tenant_id = ? AND status = ?",
        vec![Value::I64(TENANT_T3), Value::Text("paid".into())],
    )
    .await
    .expect("SQL SUM");
    let exact: Decimal = money(0, 10) + money(0, 20) + money(77, 0);
    if c.dialect.is_sqlite() {
        assert_ne!(
            sql_sum, exact,
            "SQLite 上 SUM(TEXT) 走浮点：{sql_sum} 已偏离精确值 {exact}（故报表在 Rust 侧聚合）"
        );
        eprintln!("[RPT-001] SQLite SQL SUM = {sql_sum}，精确值 = {exact}");
    } else {
        assert_eq!(sql_sum, exact, "PG/MySQL 原生 NUMERIC 求和应精确");
    }
}

/// RPT-002：留存曲线——`LAG` / `RANK` 在 AST 中不可用，改由 Rust 计算并逐行断言。
///
/// SQL 窗口函数仅作交叉校验（`LAG` / `DENSE_RANK` / `RANK` 直接跑 raw SQL），
/// 与 Rust 侧结果必须逐行一致。
#[tokio::test]
async fn rpt_002_retention_curve_computed_in_rust() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    // 订阅期租（跨月重叠）
    insert_subscription(&c, 1, 11, month_start(1), month_start(3), "active").await;
    insert_subscription(&c, 2, 12, month_start(1), month_start(2), "active").await;
    insert_subscription(&c, 3, 13, month_start(2), month_start(4), "active").await;
    insert_subscription(&c, 4, 14, month_start(1), month_start(5), "canceled").await;

    let buckets: Vec<DateTime<Utc>> = (1..=4).map(month_start).collect();
    let mut active = Vec::with_capacity(buckets.len());
    for bucket in &buckets {
        active.push(active_at(&c, TENANT_T1, *bucket).await);
    }
    assert_eq!(active, vec![3, 3, 2, 1], "逐月活跃订阅数（期租重叠口径）");

    // Rust 侧等价实现：LAG（上一月） / 流失（环比下降） / DENSE_RANK（活跃数降序）
    let lagged: Vec<Option<i64>> = (0..active.len())
        .map(|i| if i == 0 { None } else { Some(active[i - 1]) })
        .collect();
    let churn: Vec<i64> = (0..active.len())
        .map(|i| match lagged[i] {
            Some(prev) => (prev - active[i]).max(0),
            None => 0,
        })
        .collect();
    let dense_rank: Vec<i64> = active
        .iter()
        .map(|v| {
            1 + active
                .iter()
                .filter(|other| *other > v)
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len() as i64
        })
        .collect();

    assert_eq!(lagged, vec![None, Some(3), Some(3), Some(2)]);
    assert_eq!(churn, vec![0, 0, 1, 1]);
    assert_eq!(dense_rank, vec![1, 1, 2, 3], "稠密排名（并列共享名次）");

    // 交叉校验：raw SQL 窗口函数（AST 无法表达）与 Rust 计算逐行一致
    c.db()
        .execute("DROP TABLE IF EXISTS rpt_retention_month")
        .await
        .expect("drop temp");
    let ddl = if c.dialect.is_sqlite() {
        "CREATE TABLE rpt_retention_month (tenant_id INTEGER NOT NULL, month VARCHAR(16) NOT NULL, active INTEGER NOT NULL)"
    } else {
        "CREATE TABLE rpt_retention_month (tenant_id BIGINT NOT NULL, month VARCHAR(16) NOT NULL, active INT NOT NULL)"
    };
    c.db().execute(ddl).await.expect("create temp");
    for (bucket, count) in buckets.iter().zip(&active) {
        c.db()
            .execute_with_params(
                "INSERT INTO rpt_retention_month (tenant_id, month, active) VALUES (?, ?, ?)",
                vec![
                    Value::I64(TENANT_T1),
                    Value::Text(bucket.format("%Y-%m").to_string()),
                    Value::I64(*count),
                ],
            )
            .await
            .expect("insert series");
    }

    let window_sql = "SELECT month, active, LAG(active) OVER (ORDER BY month) AS prev, \
                      DENSE_RANK() OVER (ORDER BY active DESC) AS dense_rk, \
                      RANK() OVER (ORDER BY active DESC) AS rk \
                      FROM rpt_retention_month WHERE tenant_id = ? ORDER BY month";
    match c
        .db()
        .find_with_params(window_sql, vec![Value::I64(TENANT_T1)])
        .await
    {
        Ok(rows) => {
            assert_eq!(rows.len(), active.len(), "SQL 窗口查询行数应一致");
            let mut sql_rank = Vec::new();
            for (i, row) in rows.iter().enumerate() {
                let month = rowx::text_of(row, "month").unwrap();
                assert_eq!(
                    month,
                    buckets[i].format("%Y-%m").to_string(),
                    "逐行月份一致"
                );
                assert_eq!(
                    rowx::i64_of(row, "active").unwrap(),
                    active[i],
                    "逐行活跃数一致"
                );
                let prev = match row.get("prev") {
                    Some(Value::Null) | None => None,
                    Some(v) => Some(i64::from_value(v).unwrap()),
                };
                assert_eq!(prev, lagged[i], "LAG 与 Rust 上一月一致");
                assert_eq!(
                    rowx::i64_of(row, "dense_rk").unwrap(),
                    dense_rank[i],
                    "DENSE_RANK 与 Rust 稠密排名一致"
                );
                sql_rank.push(rowx::i64_of(row, "rk").unwrap());
            }
            // RANK 有并列跳号（1,1,3,4）——与 DENSE_RANK(1,1,2,3) 的差异一并锁定
            assert_eq!(sql_rank, vec![1, 1, 3, 4], "RANK 并列后跳号");
        }
        Err(e) => eprintln!(
            "[RPT-002][G7] {} 不支持窗口函数（AST 同样不支持）：{e}；Rust 侧计算结果已断言",
            c.dialect.name()
        ),
    }
    c.db()
        .execute("DROP TABLE rpt_retention_month")
        .await
        .expect("cleanup");
}

/// RPT-003：用量 Top N——`GROUP BY + ORDER BY SUM DESC + LIMIT`，含并列时的稳定次序。
#[tokio::test]
async fn rpt_003_top_usage_n() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    for (sub_id, customer_id) in [(1, 21), (2, 22), (3, 23), (4, 24)] {
        insert_subscription(
            &c,
            sub_id,
            customer_id,
            month_start(1),
            month_start(2),
            "active",
        )
        .await;
    }

    // 订阅 1: 10+20=30，订阅 2: 25+25=50，订阅 3: 5，订阅 4: 30（与 1 并列）
    seed_usage(&c, 1, 101, at(0), 10).await.unwrap();
    seed_usage(&c, 1, 102, at(60), 20).await.unwrap();
    seed_usage(&c, 2, 201, at(0), 25).await.unwrap();
    seed_usage(&c, 2, 202, at(60), 25).await.unwrap();
    seed_usage(&c, 3, 301, at(0), 5).await.unwrap();
    seed_usage(&c, 4, 401, at(0), 30).await.unwrap();

    let top2 = report::top_usage(c.db(), 2).await.expect("top2");
    assert_eq!(
        top2,
        vec![(2, Decimal::from(50)), (1, Decimal::from(30))],
        "Top2：50 之后并列 30 取 subscription_id 小者"
    );

    let top_all = report::top_usage(c.db(), 10).await.expect("top10");
    assert_eq!(
        top_all,
        vec![
            (2, Decimal::from(50)),
            (1, Decimal::from(30)),
            (4, Decimal::from(30)),
            (3, Decimal::from(5)),
        ],
        "全量：并列 30 按 subscription_id 升序稳定排列"
    );
}

/// RPT-004：对账差异视图——转账金额 vs 借方分录合计，逐行精确匹配。
#[tokio::test]
async fn rpt_004_reconcile_diff_view() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(2000, 0)).await.unwrap();

    ledger::post_transfer(c.db(), transfer_cmd(2001, money(100, 0), 0))
        .await
        .expect("transfer 2001");
    ledger::post_transfer(c.db(), transfer_cmd(2002, money(50, 0), 60))
        .await
        .expect("transfer 2002");

    // 平衡态：无差异
    let clean = report::reconcile_diff(c.db(), TENANT_T1)
        .await
        .expect("diff");
    assert_eq!(
        clean,
        vec![
            (2001, money(100, 0), money(100, 0)),
            (2002, money(50, 0), money(50, 0)),
        ],
        "两笔转账金额与借方分录合计一致"
    );
    assert!(clean.iter().all(|(_, amount, debit)| amount == debit));

    // 破坏第二笔的借方分录（amount 少记 10）→ 只应出现一行差异
    c.db()
        .execute_with_params(
            "UPDATE ledger_entries SET amount = ? WHERE id = ?",
            vec![money(40, 0).to_value(), Value::I64(2002 * 10 + 2)],
        )
        .await
        .expect("corrupt entry");

    let dirty = report::reconcile_diff(c.db(), TENANT_T1)
        .await
        .expect("diff");
    assert_eq!(
        dirty,
        vec![
            (2001, money(100, 0), money(100, 0)),
            (2002, money(50, 0), money(40, 0)),
        ]
    );
    let diffs: Vec<i64> = dirty
        .iter()
        .filter(|(_, amount, debit)| amount != debit)
        .map(|(id, _, _)| *id)
        .collect();
    assert_eq!(diffs, vec![2002], "差异行集合精确匹配");

    // 借贷平衡断言与差异视图互补
    assert_double_entry_balanced(c.db(), 2001).await;
    assert!(
        !ledger::transfer_is_balanced(c.db(), 2002).await.unwrap(),
        "被破坏的转账不再借贷平衡"
    );
}

/// RPT-005：深分页——`PageQuery` 只钳 `page_size`，`page` 无上限。
#[tokio::test]
async fn rpt_005_deep_pagination_is_cheap() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    for id in 6001..6004 {
        insert_invoice(&c, id, TENANT_T1, "draft", money(1, 0), at_days(0), None).await;
    }

    // 小数据量下的普通分页（对照组）
    let first = report::deep_page(c.db(), TENANT_T1, 1, 1000)
        .await
        .expect("page 1");
    assert_eq!(first, vec![6001, 6002, 6003]);

    // `?page=1001&page_size=1000`：page 不被钳制 → OFFSET 1_000_000
    let page_query = alun_web::api::PageQuery::new(1001, 1000);
    assert_eq!(page_query.page, 1001, "page 无上限（不被规整）");
    assert_eq!(page_query.page_size, 1000, "page_size 上限 1000");
    assert_eq!(page_query.offset(), 1_000_000);
    assert_eq!(page_query.limit(), 1000);
    assert_eq!(
        alun_web::api::PageQuery::new(0, 5000).page_size,
        1000,
        "page_size>1000 被规整为 1000"
    );
    let parsed: alun_web::api::PageQuery = serde_json::from_value(serde_json::json!({
        "page": 1001,
        "page_size": 1000
    }))
    .expect("反序列化即规整");
    assert_eq!(
        (parsed.page, parsed.page_size, parsed.offset()),
        (1001, 1000, 1_000_000)
    );

    let started = Instant::now();
    let deep = report::deep_page(c.db(), TENANT_T1, 1001, 1000)
        .await
        .expect("deep page");
    let elapsed = started.elapsed();
    assert!(deep.is_empty(), "超出数据范围的深分页应返回空集");
    assert!(
        elapsed.as_millis() < 500,
        "深分页耗时 {}ms 应 < 500ms",
        elapsed.as_millis()
    );
    eprintln!(
        "[RPT-005] {} 上 OFFSET 1000000 耗时 {}ms",
        c.dialect.name(),
        elapsed.as_millis()
    );
}

/// RPT-006：键集分页 vs OFFSET——结果集必须完全相同。
#[tokio::test]
async fn rpt_006_keyset_matches_offset() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    let expected: Vec<i64> = (8101..8111).collect();
    for id in &expected {
        seed_invoice(&c, TENANT_T1, *id, "draft").await.unwrap();
    }

    // OFFSET 分页
    let offset_started = Instant::now();
    let mut offset_pages: Vec<Vec<i64>> = Vec::new();
    for page in 1..=4 {
        let ids = report::deep_page(c.db(), TENANT_T1, page, 3)
            .await
            .expect("offset page");
        if ids.is_empty() {
            break;
        }
        offset_pages.push(ids);
    }
    let offset_elapsed = offset_started.elapsed();

    // 键集分页
    let keyset_started = Instant::now();
    let mut keyset_pages: Vec<Vec<i64>> = Vec::new();
    let mut cursor: Option<i64> = None;
    loop {
        let ids = report::keyset_page(c.db(), TENANT_T1, cursor, 3)
            .await
            .expect("keyset page");
        if ids.is_empty() {
            break;
        }
        cursor = ids.last().copied();
        keyset_pages.push(ids);
    }
    let keyset_elapsed = keyset_started.elapsed();

    assert_eq!(offset_pages, keyset_pages, "逐页结果集完全相同");
    let flattened: Vec<i64> = offset_pages.iter().flatten().copied().collect();
    assert_eq!(flattened, expected, "合并后覆盖全部行且顺序一致");
    assert_eq!(
        flattened
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        expected.len(),
        "无重复行"
    );
    eprintln!(
        "[RPT-006] {} OFFSET 4 页 {}µs / 键集 4 页 {}µs",
        c.dialect.name(),
        offset_elapsed.as_micros(),
        keyset_elapsed.as_micros()
    );
}

/// RPT-007：`#[view]` 只读模型——任何写路径都返回 `DbError::Readonly`。
#[tokio::test]
async fn rpt_007_view_model_is_readonly() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    insert_invoice(&c, 5901, TENANT_T1, "paid", money(20, 0), at_days(4), None).await;

    let mut view_row = BillingReport {
        tenant_id: TENANT_T1,
        month: "2026-01".into(),
        total: money(20, 0),
        status: "paid".into(),
    };

    let err = view_row
        .insert_or_update(c.db())
        .await
        .expect_err("视图不可写");
    assert!(
        matches!(err, DbError::Readonly("v_mrr")),
        "save 应返回 Readonly，实际 {err:?}"
    );
    assert!(matches!(
        view_row.update(c.db()).await,
        Err(DbError::Readonly("v_mrr"))
    ));
    assert!(matches!(
        BillingReport::of(view_row.tenant_id)
            .set(BillingReport::total(), view_row.total)
            .update(c.db())
            .await,
        Err(DbError::Readonly("v_mrr"))
    ));
    assert!(matches!(
        BillingReport::delete_by_pk(c.db(), TENANT_T1).await,
        Err(DbError::Readonly("v_mrr"))
    ));
    assert!(matches!(
        BillingReport::delete_by(c.db(), "tenant_id = ?", (Value::I64(TENANT_T1),)).await,
        Err(DbError::Readonly("v_mrr"))
    ));
    assert!(matches!(
        BillingReport::insert_many(&[view_row.clone()], c.db()).await,
        Err(DbError::Readonly("v_mrr"))
    ));

    // 读路径正常：三方言的类型化读取都应成功（SQLite 的 TEXT 承载经类型化读边界解析，D001 已修复）
    let typed = BillingReport::find(c.db(), TENANT_T1)
        .await
        .unwrap_or_else(|e| panic!("{}: 视图类型化读取应成功，实际 {e:?}", c.dialect.name()));
    assert_eq!(typed.total, money(20, 0), "视图金额列类型化往返");
    // 服务层走方言无关解码 → 同一个视图可正常读出金额
    let rows = report::mrr_by_month(c.db(), TENANT_T1)
        .await
        .expect("mrr view");
    assert_eq!(rows, vec![("2026-01".to_string(), money(20, 0))]);
    assert_eq!(
        ledger::entry_count(c.db(), 5901).await.unwrap(),
        0,
        "视图写操作不得产生任何副作用"
    );
}

/// RPT-008：`Expr::JsonGet` 方言无关 JSON 路径过滤（`native_json=false` 同样可用）。
#[tokio::test]
async fn rpt_008_json_path_across_dialects() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let gold = serde_json::json!({ "tier": "gold", "profile": { "city": "Shanghai" } });
    let silver = serde_json::json!({ "tier": "silver", "profile": { "city": "Beijing" } });
    insert_invoice(
        &c,
        5801,
        TENANT_T1,
        "paid",
        money(1, 0),
        at_days(0),
        Some(&gold),
    )
    .await;
    insert_invoice(
        &c,
        5802,
        TENANT_T1,
        "paid",
        money(1, 0),
        at_days(1),
        Some(&silver),
    )
    .await;
    insert_invoice(
        &c,
        5803,
        TENANT_T1,
        "paid",
        money(1, 0),
        at_days(2),
        Some(&gold),
    )
    .await;
    insert_invoice(&c, 5804, TENANT_T1, "paid", money(1, 0), at_days(3), None).await;

    // 服务层：单级路径 `tier`
    let gold_ids = report::invoices_by_json_path(c.db(), "gold")
        .await
        .expect("json path");
    assert_eq!(
        gold_ids,
        vec![5801, 5803],
        "只匹配 tier=gold（NULL 元数据不匹配）"
    );
    let none = report::invoices_by_json_path(c.db(), "platinum")
        .await
        .expect("json path");
    assert!(none.is_empty());

    // 多级路径：直接驱动 AST 节点（PG `#>>'{profile,city}'` / SQLite `$.profile.city` / MySQL `$.profile.city`）
    let nested = Expr::JsonGet {
        expr: Box::new(Expr::Column {
            table: "invoices",
            column: "metadata",
        }),
        path: "profile.city".to_string(),
    };
    let rows = FindBuilder::new()
        .table("invoices")
        .filter(Expr::Binary {
            left: Box::new(nested),
            op: BinaryOp::Eq,
            right: Box::new(expr_value(Value::Text("Beijing".into()))),
        })
        .order_by("id", Order::Asc)
        .exec(c.db())
        .await
        .expect("nested json path");
    let ids: Vec<i64> = rows
        .iter()
        .map(|r| rowx::i64_of(r, "id").unwrap())
        .collect();
    assert_eq!(ids, vec![5802], "多级 JSON 路径过滤跨库一致");

    // `native_json=false`（SQLite）时 JsonGet 依然可用（渲染为 json_extract）
    let capability = c.db().dialect().capability();
    assert_eq!(
        capability.native_json,
        !c.dialect.is_sqlite(),
        "SQLite 无原生 JSON 列类型，PG/MySQL 有"
    );
    eprintln!(
        "[RPT-008] {} native_json={} → JsonGet 仍可用",
        c.dialect.name(),
        capability.native_json
    );
}

/// 转账指令（RPT-004 用）。
fn transfer_cmd(transfer_id: i64, amount: Decimal, secs: i64) -> ledger::TransferCommand {
    ledger::TransferCommand {
        transfer_id,
        debit_entry_id: transfer_id * 10 + 1,
        credit_entry_id: transfer_id * 10 + 2,
        tenant_id: TENANT_T1,
        from_account: t1_asset(),
        to_account: t1_liability(),
        amount,
        at: at(secs),
    }
}

/// `Expr::Value` 构造助手（避免 `Value` 与 `serde_json::Value` 混淆）。
fn expr_value(value: Value) -> Expr {
    Expr::Value(value)
}

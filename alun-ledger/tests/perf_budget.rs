//! L5 性能预算（需求 §6.7 / 规划 §8 的测试侧承载）。
//!
//! 说明：框架没有 statement counter / pool stats（缺口 G10/G11），因此这里只做
//! **端到端墙钟**断言，阈值取需求 §6.7 的保守放宽值（CI 共享 runner 噪声大）。
//! `cargo bench` 侧见 `benches/`。

mod common;

use std::time::Instant;

use alun_db::prelude::*;
use alun_db_core::stmt::Value;

use common::assertions::count_by;
use common::fixtures::*;
use common::harness::ctx;

const ROWS: i64 = 5_000;

async fn seed_invoices(c: &common::harness::LedgerCtx, tenant: i64, rows: i64) {
    reset_tenant(c.db(), tenant).await.unwrap();
    seed_tenant(c, tenant, "perf").await.unwrap();
    // 按 1000 行分块：`insert_many` 不按方言参数上限分块（缺陷 D5），
    // 5000 行 × 10 列 = 50000 个占位符会超过 SQLite 变量上限。
    const CHUNK: i64 = 1000;
    let mut i = 0;
    while i < rows {
        let end = (i + CHUNK).min(rows);
        let chunk: Vec<_> = (i..end)
            .map(|n| alun_ledger::models::Invoice {
                id: tenant * 1_000_000 + n,
                tenant_id: tenant,
                customer_id: n % 97,
                status: alun_ledger::models::InvoiceStatus::Draft,
                total: money(10, 0),
                metadata: None,
                issued_at: None,
                paid_at: None,
                created_at: t0() + chrono::Duration::seconds(n),
                note: None,
            })
            .collect();
        alun_ledger::models::Invoice::insert_many(&chunk, c.db())
            .await
            .expect("batch seed");
        i = end;
    }
}

/// D005 回归：**宽行批量插入必须自动分块**。
///
/// 5000 行 × 10 列的未分块语句会超出方言占位符上限（SQLite `too many SQL variables`、
/// PG/MySQL 的参数上限），旧实现把整批拼成单条 `INSERT` 后直接失败。修复后应自动分块，
/// 行数完整、返回值语义不变。
#[tokio::test]
async fn perf_insert_many_wide_rows_is_chunked() {
    let Some(c) = ctx().await else { return };
    let tenant = TENANT_T3;
    reset_tenant(c.db(), tenant).await.unwrap();
    seed_tenant(&c, tenant, "perf-limit").await.unwrap();

    let rows: Vec<_> = (0..5000)
        .map(|i| alun_ledger::models::Invoice {
            id: tenant * 1_000_000 + i,
            tenant_id: tenant,
            customer_id: i % 97,
            status: alun_ledger::models::InvoiceStatus::Draft,
            total: money(1, 0),
            metadata: None,
            issued_at: None,
            paid_at: None,
            created_at: t0(),
            note: None,
        })
        .collect();
    let t = Instant::now();
    alun_ledger::models::Invoice::insert_many(&rows, c.db())
        .await
        .unwrap_or_else(|e| {
            panic!(
                "{}: 5000 行 × 10 列批量插入应自动分块成功，实际 {e:?}",
                c.dialect.name()
            )
        });
    let ms = t.elapsed().as_millis();

    let inserted: i64 = count_by(
        c.db(),
        "SELECT COUNT(*) FROM invoices WHERE tenant_id = ?",
        vec![Value::I64(tenant)],
    )
    .await;
    assert_eq!(inserted, 5000, "分块后不得丢行/重复行");
    eprintln!(
        "[D5] {}: 5000 行 × 10 列自动分块成功（{ms}ms）",
        c.dialect.name()
    );
}

#[tokio::test]
async fn perf_batch_insert_1000_and_point_query() {
    let Some(c) = ctx().await else { return };
    let tenant = TENANT_T3;
    reset_tenant(c.db(), tenant).await.unwrap();
    seed_tenant(&c, tenant, "perf-batch").await.unwrap();

    // 批量插入 1000：需求目标 < 500ms
    let invoices: Vec<_> = (0..1000)
        .map(|i| alun_ledger::models::Invoice {
            id: tenant * 1_000_000 + i,
            tenant_id: tenant,
            customer_id: i % 97,
            status: alun_ledger::models::InvoiceStatus::Draft,
            total: money(1, 0),
            metadata: None,
            issued_at: None,
            paid_at: None,
            created_at: t0(),
            note: None,
        })
        .collect();
    let t = Instant::now();
    alun_ledger::models::Invoice::insert_many(&invoices, c.db())
        .await
        .expect("insert_many");
    let batch_ms = t.elapsed().as_millis();
    eprintln!("[perf] insert_many(1000) = {batch_ms}ms (目标 <500ms)");
    assert!(batch_ms < 5_000, "insert_many(1000) 过慢: {batch_ms}ms");

    // 主键查询：需求目标 < 1ms（含驱动往返，放宽到 50ms 以抗 CI 噪声）
    let t = Instant::now();
    let _ = c
        .db()
        .find_with_params(
            "SELECT id FROM invoices WHERE id = ?",
            vec![Value::I64(tenant * 1_000_000 + 500)],
        )
        .await
        .unwrap();
    let point_ms = t.elapsed().as_millis();
    eprintln!("[perf] 主键查询 = {point_ms}ms (目标 <1ms)");
    assert!(point_ms < 200, "主键查询过慢: {point_ms}ms");
}

#[tokio::test]
async fn perf_deep_offset_versus_keyset() {
    let Some(c) = ctx().await else { return };
    let tenant = TENANT_T3;
    seed_invoices(&c, tenant, ROWS).await;

    let deep_page = ROWS / 100;
    let t = Instant::now();
    let offset_rows = alun_ledger::services::report::deep_page(c.db(), tenant, deep_page, 100)
        .await
        .unwrap();
    let offset_ms = t.elapsed().as_millis();

    let t = Instant::now();
    let keyset_rows = alun_ledger::services::report::keyset_page(
        c.db(),
        tenant,
        Some(tenant * 1_000_000 + ROWS - 200),
        100,
    )
    .await
    .unwrap();
    let keyset_ms = t.elapsed().as_millis();

    eprintln!(
        "[perf] OFFSET(5000 行取尾部) = {offset_ms}ms / 键集 = {keyset_ms}ms（行数 {} / {}）",
        offset_rows.len(),
        keyset_rows.len()
    );
    assert!(!keyset_rows.is_empty());
    assert!(offset_ms < 2_000, "深分页过慢: {offset_ms}ms");
}

#[tokio::test]
async fn perf_aggregate_and_json_path() {
    let Some(c) = ctx().await else { return };
    let tenant = TENANT_T1;
    seed_tenant(&c, tenant, "perf-agg").await.unwrap();
    seed_subscription(&c, tenant, 9001, t0(), at(3600), "active")
        .await
        .unwrap();
    for i in 0..500 {
        seed_usage(&c, 9001, 9_100_000 + i, at(i), 2).await.unwrap();
    }

    let t = Instant::now();
    let agg = alun_ledger::services::billing::aggregate_usage(c.db(), 9001)
        .await
        .unwrap();
    let agg_ms = t.elapsed().as_millis();
    eprintln!(
        "[perf] 聚合 COUNT(DISTINCT)/SUM ≈ {agg_ms}ms（days={}, total={}）",
        agg.days, agg.total
    );
    assert_eq!(agg.total, rust_decimal::Decimal::from(1000));

    let t = Instant::now();
    let _ = alun_ledger::services::report::invoices_by_json_path(c.db(), "gold")
        .await
        .unwrap();
    let json_ms = t.elapsed().as_millis();
    eprintln!("[perf] JSON 路径过滤 ≈ {json_ms}ms");
    assert!(json_ms < 1_000);
}

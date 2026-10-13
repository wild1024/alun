//! 聚合与 JSON 路径（记录基线，无硬阈值）。

mod common;

use criterion::{Criterion, criterion_group, criterion_main};

use alun_db::prelude::*;
use alun_db_core::stmt::Value;

use alun_ledger::services::{billing, report};

fn bench_aggregate(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let db = rt.block_on(common::bench_db());
    rt.block_on(common::seed_accounts(&db, 1, "1000.00"));
    rt.block_on(common::seed_invoices(&db, 1, 2_000));

    rt.block_on(async {
        db.execute_with_params(
            "INSERT INTO subscriptions (id, tenant_id, customer_id, plan, period_start, period_end, status) VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(9001),
                Value::I64(1),
                Value::I64(7),
                Value::Text("pro".into()),
                Value::Text("2026-01-01 00:00:00.000000".into()),
                Value::Text("2026-02-01 00:00:00.000000".into()),
                Value::Text("active".into()),
            ],
        )
        .await
        .unwrap();
        for i in 0..2_000 {
            db.execute_with_params(
                "INSERT INTO usage_records (id, subscription_id, usage_date, quantity, metadata) VALUES (?, ?, ?, ?, ?)",
                vec![
                    Value::I64(9_100_000 + i),
                    Value::I64(9001),
                    Value::Text(format!("2026-01-{:02} 00:00:00.000000", (i % 28) + 1)),
                    Value::I64(1 + (i % 5)),
                    Value::Json(serde_json::json!({"unit": "call"}).to_string()),
                ],
            )
            .await
            .unwrap();
        }
    });

    let mut group = c.benchmark_group("aggregate");
    group.bench_function("usage_group_by_having", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move { billing::aggregate_usage(&db, 9001).await.unwrap() }
        })
    });
    group.bench_function("mrr_cte_rust_cumulative", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                let rows = report::mrr_by_month(&db, 1).await.unwrap();
                report::cumulative(&rows)
            }
        })
    });
    group.bench_function("json_path_filter", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move { report::invoices_by_json_path(&db, "gold").await.unwrap() }
        })
    });
    group.finish();
}

criterion_group!(benches, bench_aggregate);
criterion_main!(benches);

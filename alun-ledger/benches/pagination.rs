//! 分页：全表扫描 / 深 OFFSET / 键集分页（需求 §6.7：OFFSET 1000000 <500ms）。

mod common;

use criterion::{Criterion, criterion_group, criterion_main};

use alun_ledger::services::report;

fn bench_pagination(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let db = rt.block_on(common::bench_db());
    let rows = std::env::var("ALUN_LEDGER_SEED_ROWS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(10_000);
    rt.block_on(common::seed_invoices(&db, 1, rows));

    let mut group = c.benchmark_group("pagination");
    group.bench_function("keyset_after_mid", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                report::keyset_page(&db, 1, Some(1_000_000 + rows / 2), 100)
                    .await
                    .unwrap()
            }
        })
    });
    group.bench_function("offset_deep", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move { report::deep_page(&db, 1, rows / 100, 100).await.unwrap() }
        })
    });
    group.finish();
}

criterion_group!(benches, bench_pagination);
criterion_main!(benches);

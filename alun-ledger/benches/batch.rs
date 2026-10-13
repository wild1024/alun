//! 批量插入：`insert_many` 100 / 1000 行 vs 逐行插入（需求 §6.7：1000 条 <500ms）。

mod common;

use criterion::{Criterion, criterion_group, criterion_main};
use rust_decimal::Decimal;

use alun_db::prelude::*;
use std::sync::atomic::{AtomicI64, Ordering};

/// 全局单调主键分配器：criterion 的多次迭代不得复用主键。
static NEXT_ID: AtomicI64 = AtomicI64::new(100_000_000);

fn invoice(id: i64) -> alun_ledger::models::Invoice {
    alun_ledger::models::Invoice {
        id,
        tenant_id: 1,
        customer_id: 7,
        status: alun_ledger::models::InvoiceStatus::Draft,
        total: Decimal::new(1000, 2),
        metadata: None,
        issued_at: None,
        paid_at: None,
        created_at: chrono::DateTime::from_timestamp(1_767_225_600, 0).unwrap(),
        note: None,
    }
}

fn bench_batch(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let db = rt.block_on(common::bench_db());

    let mut group = c.benchmark_group("insert_many");
    for rows in [100usize, 1000] {
        group.bench_function(format!("batch_{rows}"), |b| {
            b.to_async(&rt).iter(|| {
                let base = NEXT_ID.fetch_add(rows as i64 + 1, Ordering::SeqCst);
                let items: Vec<_> = (0..rows).map(|i| invoice(base + i as i64)).collect();
                let db = db.clone();
                async move {
                    alun_ledger::models::Invoice::insert_many(&items, &db)
                        .await
                        .unwrap()
                }
            })
        });
    }

    group.bench_function("row_by_row_100", |b| {
        b.to_async(&rt).iter(|| {
            let base = NEXT_ID.fetch_add(101, Ordering::SeqCst);
            let db = db.clone();
            async move {
                for i in 0..100 {
                    let mut inv = invoice(base + i);
                    inv.insert_or_update(&db).await.unwrap();
                }
            }
        })
    });
    group.finish();
}

criterion_group!(benches, bench_batch);
criterion_main!(benches);

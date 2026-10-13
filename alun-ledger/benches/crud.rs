//! 单条插入 / 主键查询（需求 §6.7：<1ms）。

mod common;

use criterion::{Criterion, criterion_group, criterion_main};
use rust_decimal::Decimal;

use alun_db::prelude::*;

fn bench_crud(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let db = rt.block_on(common::bench_db());
    rt.block_on(common::seed_accounts(&db, 1, "1000.00"));

    let mut counter = 0i64;
    c.bench_function("single_insert_account", |b| {
        b.to_async(&rt).iter(|| {
            counter += 1;
            let id = 900_000 + counter;
            let db = db.clone();
            async move {
                db.execute_with_params(
                    "INSERT INTO accounts (id, tenant_id, code, name, kind, balance, currency) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    vec![
                        alun_db_core::stmt::Value::I64(id),
                        alun_db_core::stmt::Value::I64(1),
                        alun_db_core::stmt::Value::Text(format!("B{id}")),
                        alun_db_core::stmt::Value::Text("bench".into()),
                        alun_db_core::stmt::Value::Text("asset".into()),
                        alun_db_core::traits::TypeMapper::to_value(&Decimal::new(100, 2)),
                        alun_db_core::stmt::Value::Text("CNY".into()),
                    ],
                )
                .await
                .unwrap()
            }
        })
    });

    c.bench_function("find_account", |b| {
        b.to_async(&rt).iter(|| {
            let db = db.clone();
            async move {
                let _ = alun_ledger::models::Account::find(&db, 1001i64).await;
            }
        })
    });
}

criterion_group!(benches, bench_crud);
criterion_main!(benches);

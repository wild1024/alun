//! bench 公共设施：SQLite 内存库 + 迁移 + 种子。

#![allow(dead_code)]

use alun_db::prelude::*;
use alun_db_core::stmt::Value;

/// 建立并迁移一个内存库。
pub async fn bench_db() -> Db {
    let db = Db::connect("sqlite::memory:").await.expect("connect");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    for dir in [
        root.join("sqlite/01-base"),
        root.join("common"),
        root.join("sqlite/02-evolve"),
    ] {
        Migrator::from_dir(&dir).run(&db).await.expect("migrate");
    }
    db
}

/// 种入租户 + 账户。
pub async fn seed_accounts(db: &Db, tenant: i64, balance: &str) {
    db.execute_with_params(
        "INSERT INTO tenants (id, name, plan, status, created_at, updated_at, region) VALUES (?, ?, ?, ?, ?, ?, ?)",
        vec![
            Value::I64(tenant),
            Value::Text("bench".into()),
            Value::Text("pro".into()),
            Value::Text("active".into()),
            Value::Text("2026-01-01 00:00:00.000000".into()),
            Value::Text("2026-01-01 00:00:00.000000".into()),
            Value::Text("cn".into()),
        ],
    )
    .await
    .expect("tenant");
    for n in 1..=5 {
        db.execute_with_params(
            "INSERT INTO accounts (id, tenant_id, code, name, kind, balance, currency) VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(tenant * 1000 + n),
                Value::I64(tenant),
                Value::Text(format!("{n}001")),
                Value::Text(format!("acct-{n}")),
                Value::Text("asset".into()),
                Value::Text(balance.to_string()),
                Value::Text("CNY".into()),
            ],
        )
        .await
        .expect("account");
    }
}

/// 批量种入发票（按 1000 行分块：`insert_many` 不分块，见缺陷 D5）。
pub async fn seed_invoices(db: &Db, tenant: i64, rows: i64) {
    let mut i = 0;
    while i < rows {
        let end = (i + 1000).min(rows);
        let chunk: Vec<alun_ledger::models::Invoice> = (i..end)
            .map(|n| alun_ledger::models::Invoice {
                id: tenant * 1_000_000 + n,
                tenant_id: tenant,
                customer_id: n % 97,
                status: alun_ledger::models::InvoiceStatus::Draft,
                total: rust_decimal::Decimal::new(1000, 2),
                metadata: None,
                issued_at: None,
                paid_at: None,
                created_at: chrono::DateTime::from_timestamp(1_767_225_600 + n, 0).unwrap(),
                note: None,
            })
            .collect();
        alun_ledger::models::Invoice::insert_many(&chunk, db)
            .await
            .expect("seed invoices");
        i = end;
    }
}

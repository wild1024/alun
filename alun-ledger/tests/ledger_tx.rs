//! M1 复式记账（用例 LGR-001..012）。

mod common;

use alun_db::prelude::*;
use chrono::Utc;
use rust_decimal::Decimal;

use alun_ledger::services::{faults::FailPoint, ledger, money};

use common::assertions::*;
use common::fixtures::*;
use common::harness::ctx;

fn cmd(transfer_id: i64, amount: Decimal) -> ledger::TransferCommand {
    ledger::TransferCommand {
        transfer_id,
        debit_entry_id: transfer_id * 10 + 1,
        credit_entry_id: transfer_id * 10 + 2,
        tenant_id: TENANT_T1,
        from_account: t1_asset(),
        to_account: t1_liability(),
        amount,
        at: Utc::now(),
    }
}

#[tokio::test]
async fn lgr_001_transfer_happy_path() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(2000, 0)).await.unwrap();

    let transfer = ledger::post_transfer(c.db(), cmd(100, money(1000, 1)))
        .await
        .expect("transfer");

    assert_eq!(transfer.amount, money(1000, 1));
    assert_balance(c.db(), TENANT_T1, t1_asset(), "999.99").await;
    assert_balance(c.db(), TENANT_T1, t1_liability(), "1000.01").await;
    assert_eq!(ledger::entry_count(c.db(), 100).await.unwrap(), 2);
    assert_double_entry_balanced(c.db(), 100).await;
    assert_total_balance(c.db(), TENANT_T1, money(2000, 0)).await;
}

#[tokio::test]
async fn lgr_002_insufficient_balance_rejects_without_writes() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(10, 0)).await.unwrap();

    let err = ledger::post_transfer(c.db(), cmd(101, money(1000, 0)))
        .await
        .expect_err("should reject");
    assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");

    assert_balance(c.db(), TENANT_T1, t1_asset(), "10").await;
    assert_balance(c.db(), TENANT_T1, t1_liability(), "0").await;
    assert_eq!(ledger::entry_count(c.db(), 101).await.unwrap(), 0);
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM transfers WHERE tenant_id = ?",
            vec![alun_db_core::stmt::Value::I64(TENANT_T1)]
        )
        .await,
        0
    );
}

#[tokio::test]
async fn lgr_003_failpoint_before_credit_rolls_back_everything() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(2000, 0)).await.unwrap();

    let _armed = common::faults::Armed::new(FailPoint::BeforeCredit);
    let err = ledger::post_transfer(c.db(), cmd(102, money(500, 0)))
        .await
        .expect_err("failpoint must abort");
    assert!(matches!(err, DbError::Transaction(_)), "got {err:?}");

    // 扣款也被回滚
    assert_balance(c.db(), TENANT_T1, t1_asset(), "2000").await;
    assert_balance(c.db(), TENANT_T1, t1_liability(), "0").await;
    assert_eq!(ledger::entry_count(c.db(), 102).await.unwrap(), 0);
}

#[tokio::test]
async fn lgr_005_deadlock_code_maps_to_query_not_notfound() {
    let Some(c) = ctx().await else { return };
    // 用非法 SQL 触发驱动错误，验证未命中映射码 → Query{sql,source}
    let err = c
        .db()
        .execute("SELECT * FROM __no_such_table__")
        .await
        .expect_err("must fail");
    match err {
        DbError::Query { .. } => {}
        DbError::NotFound => panic!("错误被吞成 NotFound"),
        other => panic!("期望 Query，实际 {other:?}"),
    }
}

#[tokio::test]
async fn lgr_006_optimistic_cas_second_update_loses() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(100, 0)).await.unwrap();

    let first = ledger::cas_rename(c.db(), TENANT_T1, t1_asset(), "account-1001", "renamed").await;
    let second = ledger::cas_rename(c.db(), TENANT_T1, t1_asset(), "account-1001", "other").await;
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(first, 1, "首次 CAS 应成功");
    assert_eq!(
        second, 0,
        "二次 CAS 应因版本不匹配失败（框架无 #[version]，缺口 G5）"
    );
}

#[tokio::test]
async fn lgr_009_nested_transaction_is_independent_connection() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(50, 0)).await.unwrap();

    // 外层事务内再开事务：无 savepoint，内层从池另借独立连接（缺口 G4）。
    // 注意：内层**必须写不同的行**——若写外层已锁定的同一行，PG 上会自死锁
    // （内层等外层的行锁、外层等内层返回），这正是"没有 savepoint 就无法安全嵌套"的实证。
    let inner_db = c.db().clone();
    let outer = c
        .db()
        .transaction(move |tx| async move {
            tx.execute_with_params(
                "UPDATE accounts SET balance = ? WHERE id = ? AND tenant_id = ?",
                vec![
                    alun_db_core::traits::TypeMapper::to_value(&money(999, 0)),
                    alun_db_core::stmt::Value::I64(t1_asset()),
                    alun_db_core::stmt::Value::I64(TENANT_T1),
                ],
            )
            .await?;

            // 内层事务（独立连接）回滚，不影响外层
            let inner = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                inner_db.transaction(|tx2| async move {
                    tx2.execute_with_params(
                        "UPDATE accounts SET name = ? WHERE id = ? AND tenant_id = ?",
                        vec![
                            alun_db_core::stmt::Value::Text("inner".into()),
                            alun_db_core::stmt::Value::I64(t1_liability()),
                            alun_db_core::stmt::Value::I64(TENANT_T1),
                        ],
                    )
                    .await?;
                    Err::<(), DbError>(DbError::Transaction("inner rollback".into()))
                }),
            )
            .await;
            match inner {
                Ok(Err(_)) | Ok(Ok(())) => {}
                Err(_) => {
                    // SQLite 内存库池容量为 1：内层事务拿不到第二个连接 → 超时。
                    // 这正是「无 savepoint 时嵌套事务不可用」的实证（缺口 G4）。
                    assert!(
                        c.dialect.is_sqlite(),
                        "非 SQLite 方言不应因连接竞争阻塞（{}）",
                        c.dialect.name()
                    );
                    eprintln!(
                        "[G4] {} 上嵌套事务因连接竞争超时（无 savepoint，内层需独立连接）",
                        c.dialect.name()
                    );
                }
            }
            Ok(())
        })
        .await;
    outer.expect("外层事务应提交成功（内层回滚不影响外层）");

    // 外层提交生效；内层回滚未生效
    assert_balance(c.db(), TENANT_T1, t1_asset(), "999").await;
    let rows = c
        .db()
        .find_with_params(
            "SELECT name FROM accounts WHERE id = ?",
            vec![alun_db_core::stmt::Value::I64(t1_liability())],
        )
        .await
        .unwrap();
    let name = rows
        .first()
        .and_then(|r| r.get("name"))
        .map(|v| format!("{v:?}"))
        .unwrap_or_default();
    assert!(
        !name.contains("inner"),
        "内层事务的回滚应生效，实际 name={name}"
    );
}

#[tokio::test]
async fn lgr_010_explicit_transaction_commit_and_rollback_idempotent() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(10, 0)).await.unwrap();

    let tx = c.db().begin().await.expect("begin");
    tx.find("SELECT 1").await.expect("work");
    tx.commit().await.expect("commit #1");
    tx.commit().await.expect("commit #2 幂等");

    let tx2 = c.db().begin().await.expect("begin2");
    tx2.rollback().await.expect("rollback #1");
    tx2.rollback().await.expect("rollback #2 幂等");
}

#[tokio::test]
async fn lgr_011_decimal_roundtrip_typed_vs_adapter() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    let amounts = [
        money(0, 1),
        money(0, 10),
        Decimal::new(1, 1),
        Decimal::new(-1, 0),
        Decimal::from_i128_with_scale(999_999_999_999_999, 2),
        // 尾随零分组（曾触发 PG NUMERIC 静默错值 D025，已修复）：这里直接断言往返
        Decimal::from(10000),
        Decimal::from_i128_with_scale(10_i128.pow(10), 0),
    ];
    seed_accounts(&c, TENANT_T1, amounts[0]).await.unwrap();

    // 适配器路径：三库一致（显式记录为缺陷 D1）
    for (i, want) in amounts.iter().enumerate() {
        let id = account_id(TENANT_T1, 1) + 100 + i as i64;
        c.db()
            .execute_with_params(
                "INSERT INTO accounts (id, tenant_id, code, name, kind, balance, currency) VALUES (?, ?, ?, ?, ?, ?, ?)",
                vec![
                    alun_db_core::stmt::Value::I64(id),
                    alun_db_core::stmt::Value::I64(TENANT_T1),
                    alun_db_core::stmt::Value::Text(format!("A{i}")),
                    alun_db_core::stmt::Value::Text("acct".into()),
                    alun_db_core::stmt::Value::Text("asset".into()),
                    alun_db_core::traits::TypeMapper::to_value(want),
                    alun_db_core::stmt::Value::Text("CNY".into()),
                ],
            )
            .await
            .unwrap();
        let got = money::account_balance(c.db(), TENANT_T1, id).await.unwrap();
        assert_eq!(got, *want, "适配器读回 {i}");
    }

    // 类型化路径：三方言都应成功（SQLite 的 TEXT 承载经类型化读边界解析，D001 已修复）
    let typed = alun_ledger::models::Account::find(c.db(), t1_asset())
        .await
        .unwrap_or_else(|e| panic!("{}: 类型化读取应成功，实际 {e:?}", c.dialect.name()));
    assert_eq!(typed.balance, amounts[0], "类型化读回金额");

    // 38 位十进制：SQLite 用 TEXT 承载 → 由**适配器**显式报错（不得 panic）；
    // PG/MySQL 的 NUMERIC/DECIMAL(38,10) 列在写入侧就被数据库拒绝（列类型本身即边界）。
    let huge_id = account_id(TENANT_T1, 1) + 900;
    let huge_insert = c
        .db()
        .execute_with_params(
            "INSERT INTO accounts (id, tenant_id, code, name, kind, balance, currency) VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                alun_db_core::stmt::Value::I64(huge_id),
                alun_db_core::stmt::Value::I64(TENANT_T1),
                alun_db_core::stmt::Value::Text("HUGE".into()),
                alun_db_core::stmt::Value::Text("acct".into()),
                alun_db_core::stmt::Value::Text("asset".into()),
                alun_db_core::stmt::Value::Text("12345678901234567890123456789012345678".into()),
                alun_db_core::stmt::Value::Text("CNY".into()),
            ],
        )
        .await;
    if c.dialect.is_sqlite() {
        huge_insert.expect("SQLite TEXT 列可写入超范围字面量");
        let huge = money::account_balance(c.db(), TENANT_T1, huge_id).await;
        assert!(
            matches!(huge, Err(DbError::InvalidValue(_))),
            "超范围 Decimal 应显式 InvalidValue，实际 {huge:?}"
        );
    } else {
        assert!(
            huge_insert.is_err(),
            "{} 的 NUMERIC/DECIMAL 列应在写入侧拒绝超范围值",
            c.dialect.name()
        );
    }
}

#[tokio::test]
async fn lgr_012_uncommitted_write_invisible_to_other_connection() {
    let Some(c) = ctx().await else { return };
    if c.dialect.is_sqlite() {
        eprintln!("[差异登记] SQLite 内存库单连接，跳过可见性用例");
        return;
    }
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(10, 0)).await.unwrap();

    let tx = c.db().begin().await.expect("begin");
    tx.execute_with_params(
        "UPDATE accounts SET name = ? WHERE id = ?",
        vec![
            alun_db_core::stmt::Value::Text("uncommitted".into()),
            alun_db_core::stmt::Value::I64(t1_asset()),
        ],
    )
    .await
    .expect("update");

    let other = c
        .db()
        .find_with_params(
            "SELECT name FROM accounts WHERE id = ?",
            vec![alun_db_core::stmt::Value::I64(t1_asset())],
        )
        .await
        .expect("read from pool");
    let seen = other
        .first()
        .and_then(|r| r.get("name"))
        .map(|v| format!("{v:?}"))
        .unwrap_or_default();
    assert!(
        !seen.contains("uncommitted"),
        "未提交写入被其他连接看到: {seen}"
    );
    tx.rollback().await.expect("rollback");
}

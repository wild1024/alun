//! M13 并发对抗（规划 §7「并发对抗」）。
//!
//! 覆盖：100 并发转账（总额守恒 + 分录/余额一致 + 无负余额）、100 并发读、
//! 交叉加锁探测、4 路并发迁移、事务中开事务（savepoint 语义）、异常路径
//! 归还连接。
//!
//! 单连接方言（`sqlite-mem` 容量为 1）上并发被池串行化：用例如实记录
//! 「串行化 = 无死锁」；多连接方言（sqlite-file / PG / MySQL）上才可能
//! 出现真实的交叉锁定与丢失更新。

mod common;

use std::collections::HashMap;
use std::time::Duration;

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use chrono::Utc;
use rust_decimal::Decimal;

use alun_ledger::services::{ledger, money, rowx};

use common::assertions::*;
use common::fixtures::*;
use common::harness::{Dialect, LedgerCtx, ctx};

const T1_INITIAL: i64 = 1000;

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

/// 分录 ↔ 余额一致性：`Σ(符号 × 金额) == 当前余额 − 初始余额`。
///
/// 复式记账的硬不变量。并发「读旧值 → 覆盖写」会丢更新：分录记了两笔，
/// 余额只动一次 → 该等式被破坏（比「总额守恒」更强，能捕获丢失更新）。
async fn assert_entries_match_balance_delta(c: &LedgerCtx, initial: &[(i64, Decimal)]) {
    let balances: HashMap<i64, Decimal> = ledger::all_balances(c.db(), TENANT_T1)
        .await
        .expect("all_balances")
        .into_iter()
        .collect();
    let rows = c
        .db()
        .find_with_params(
            "SELECT account_id, direction, amount FROM ledger_entries WHERE tenant_id = ?",
            vec![Value::I64(TENANT_T1)],
        )
        .await
        .expect("read entries");
    let mut delta: HashMap<i64, Decimal> = HashMap::new();
    for row in &rows {
        let acct = rowx::i64_of(row, "account_id").expect("account_id");
        let dir = rowx::text_of(row, "direction").expect("direction");
        let amount = rowx::decimal_of(row, "amount").expect("amount");
        let signed = if dir == "debit" { amount } else { -amount };
        *delta.entry(acct).or_insert(Decimal::ZERO) += signed;
    }
    for (account, init) in initial {
        let current = balances.get(account).copied().unwrap_or(Decimal::ZERO);
        let moved = delta.get(account).copied().unwrap_or(Decimal::ZERO);
        assert_eq!(
            moved,
            current - *init,
            "账户 {account} 分录净额必须等于余额变动（丢失更新会破坏该不变量）"
        );
    }
}

// ==================== 100 并发转账 ====================

/// 并发对抗：100 并发转账——成功 + 失败 = 100，无负余额，总额守恒，
/// 分录与余额一致。
#[tokio::test]
async fn conc_001_hundred_concurrent_transfers_conserve_money() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(T1_INITIAL, 0))
        .await
        .unwrap();

    const N: i64 = 100;
    let mut handles = Vec::with_capacity(N as usize);
    for i in 0..N {
        let db = c.db().clone();
        handles.push(tokio::spawn(async move {
            ledger::post_transfer(&db, cmd(10_000 + i, money(1, 0))).await
        }));
    }

    let mut successes = 0_i64;
    let mut failures: Vec<String> = Vec::new();
    for handle in handles {
        match handle.await.expect("join") {
            Ok(_) => successes += 1,
            Err(e) => failures.push(format!("{e:?}")),
        }
    }

    assert_eq!(
        successes + failures.len() as i64,
        N,
        "成功 + 失败必须 = 100"
    );
    if c.dialect.is_single_connection() {
        assert_eq!(
            successes, N,
            "单连接池方言（sqlite-mem）上并发被池串行化 → 全部成功；失败样本 {failures:?}"
        );
    }
    // 服务层用 CAS 更新（无 `#[version]` 时的手写乐观锁，缺口 G5）：
    // 多连接方言上并发读到同一版本 → 部分事务以并发冲突失败，属**预期**行为，
    // 关键是守恒与无负余额；单连接池方言（sqlite-mem）被池串行化则全部成功。
    for f in &failures {
        // 并发失败的合法形态：
        // - 服务层 CAS 冲突（`DbError::Transaction` / "concurrent modification"）
        // - SQLite（含文件库）无 MVCC，写锁冲突映射为 `DbError::LockTimeout`（D013）
        assert!(
            f.contains("concurrent modification")
                || f.contains("Transaction")
                || (c.dialect.is_sqlite() && f.contains("LockTimeout")),
            "失败原因应为并发冲突，实际：{f}"
        );
    }

    // 无负余额（红线）
    assert_no_negative_balance(c.db(), TENANT_T1).await;
    // 复式记账：租户内部总额不变
    assert_total_balance(c.db(), TENANT_T1, money(T1_INITIAL, 0)).await;
    // 成功笔数决定余额变动（每笔 1 元）
    assert_balance(
        c.db(),
        TENANT_T1,
        t1_asset(),
        &(T1_INITIAL - successes).to_string(),
    )
    .await;
    assert_balance(c.db(), TENANT_T1, t1_liability(), &successes.to_string()).await;
    // 分录数 = 2 × 成功笔数，且与余额变动一致
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM ledger_entries WHERE tenant_id = ?",
            vec![Value::I64(TENANT_T1)]
        )
        .await,
        successes * 2
    );
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM transfers WHERE tenant_id = ?",
            vec![Value::I64(TENANT_T1)]
        )
        .await,
        successes
    );
    assert_entries_match_balance_delta(
        &c,
        &[
            (t1_asset(), money(T1_INITIAL, 0)),
            (t1_liability(), Decimal::ZERO),
        ],
    )
    .await;
    eprintln!(
        "[CONC-001] {}: {successes} 成功 / {} 失败，总额守恒、分录与余额一致",
        c.dialect.name(),
        failures.len()
    );
}

// ==================== 100 并发读 ====================

/// 并发对抗：100 并发读同一行——不超时、值一致。
#[tokio::test]
async fn conc_002_hundred_concurrent_reads_same_row() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(123, 45)).await.unwrap();

    let expected = money(123, 45);
    let mut handles = Vec::with_capacity(100);
    for _ in 0..100 {
        let db = c.db().clone();
        handles.push(tokio::spawn(async move {
            money::account_balance(&db, TENANT_T1, t1_asset()).await
        }));
    }
    let mut ok = 0;
    for handle in handles {
        let got = handle
            .await
            .expect("join")
            .unwrap_or_else(|e| panic!("并发读失败: {e}"));
        assert_eq!(got, expected, "并发读到的值必须一致");
        ok += 1;
    }
    assert_eq!(ok, 100);
    eprintln!(
        "[CONC-002] {}: 100 并发读全部成功且值一致",
        c.dialect.name()
    );
}

// ==================== 交叉加锁 ====================

/// 并发对抗：交叉加锁（A→B / B→A）——记录真实结果，数据不得被改坏。
#[tokio::test]
async fn conc_003_crossed_lock_probe() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(500, 0)).await.unwrap();
    let (first, second) = (t1_asset(), t1_liability());

    // 两个任务以相反顺序更新同一对账户（交叉锁定）
    let mut handles = Vec::new();
    for order in [[first, second], [second, first]] {
        let db = c.db().clone();
        handles.push(tokio::spawn(async move {
            db.transaction(move |tx| async move {
                tx.execute_with_params(
                    "UPDATE accounts SET balance = balance WHERE id = ?",
                    vec![Value::I64(order[0])],
                )
                .await?;
                // 让另一任务有机会拿到第二个账户的锁
                tokio::time::sleep(Duration::from_millis(100)).await;
                tx.execute_with_params(
                    "UPDATE accounts SET balance = balance WHERE id = ?",
                    vec![Value::I64(order[1])],
                )
                .await?;
                Ok(())
            })
            .await
        }));
    }

    let mut outcomes = Vec::new();
    for handle in handles {
        let result = tokio::time::timeout(Duration::from_secs(15), handle)
            .await
            .expect("交叉加锁探测超时（真死锁）")
            .expect("join");
        outcomes.push(result);
    }
    let errors: Vec<String> = outcomes
        .iter()
        .filter_map(|r| r.as_ref().err().map(|e| format!("{e:?}")))
        .collect();

    if c.dialect.is_single_connection() {
        // 单连接池（sqlite-mem）：第二个事务在池上等待第一个结束 → 无真交叉锁定
        assert!(
            errors.is_empty(),
            "单连接方言上交叉加锁被池串行化，不应报错：{errors:?}"
        );
        eprintln!("[CONC-003] sqlite-mem: 单连接池把交叉加锁串行化，两个事务均提交（无死锁）");
    } else {
        assert!(errors.len() <= 1, "死锁检测应只中止一方，实际 {errors:?}");
        for e in &errors {
            // D013 修复后死锁/锁等待有专用变体；仍可接受 Query/Transaction（不同方言路径）
            assert!(
                [
                    "Deadlock",
                    "SerializationFailure",
                    "LockTimeout",
                    "Query",
                    "Transaction"
                ]
                .iter()
                .any(|k| e.contains(k)),
                "死锁错误应落可识别的冲突变体，实际 {e}"
            );
        }
        eprintln!(
            "[CONC-003] {}: 交叉加锁结果 = {:?}，错误 {:?}",
            c.dialect.name(),
            outcomes
                .iter()
                .map(|r| if r.is_ok() { "ok" } else { "err" })
                .collect::<Vec<_>>(),
            errors
        );
    }

    // 无论谁让步：金额未变、无负余额、连接池仍可用
    assert_balance(c.db(), TENANT_T1, first, "500").await;
    assert_balance(c.db(), TENANT_T1, second, "0").await;
    assert_no_negative_balance(c.db(), TENANT_T1).await;
    let t = ledger::post_transfer(c.db(), cmd(30_001, money(1, 0)))
        .await
        .expect("交叉加锁后池仍可用");
    assert_eq!(t.amount, money(1, 0));
}

// ==================== 事务中开事务 ====================

/// 并发对抗：事务中开事务——有无 savepoint？内层在单连接池上自等。
#[tokio::test]
async fn conc_004_nested_transaction_savepoint_probe() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(10, 0)).await.unwrap();

    let outer_db = c.db().clone();
    let inner_db = c.db().clone();
    let probe = tokio::time::timeout(Duration::from_secs(10), async move {
        outer_db
            .transaction(move |tx| async move {
                tx.execute_with_params(
                    "UPDATE accounts SET name = ? WHERE id = ?",
                    vec![Value::Text("outer".into()), Value::I64(t1_asset())],
                )
                .await?;
                // 内层事务：有 savepoint 则为嵌套；否则需要另一条连接。
                // 注意写**不同行**：写外层已锁定的行会在 PG 上自死锁（无 savepoint 的实证）。
                let inner = tokio::time::timeout(
                    Duration::from_secs(5),
                    inner_db.transaction(|tx2| async move {
                        tx2.execute_with_params(
                            "UPDATE accounts SET name = ? WHERE id = ?",
                            vec![Value::Text("inner".into()), Value::I64(t1_liability())],
                        )
                        .await?;
                        Ok(())
                    }),
                )
                .await;
                Ok(inner)
            })
            .await
    })
    .await;
    let inner = probe
        .expect("外层事务在 10s 内未完成（真死锁）")
        .expect("外层事务失败");

    let name: String = c
        .db()
        .scalar_with_params(
            "SELECT name FROM accounts WHERE id = ?",
            vec![Value::I64(t1_asset())],
        )
        .await
        .expect("读回账户名");

    if c.dialect.is_sqlite() {
        // SQLite 家族都做不到「内层事务真正并发」：
        // - `sqlite-mem`：池容量 1，内层借连接时自等到超时；
        // - `sqlite-file`：文件级写锁 + 无 MVCC，内层独立连接阻塞在外层未提交的写锁上。
        // 两者都表现为内层失败，且外层结果不受影响。
        assert!(
            inner.is_err(),
            "SQLite 方言上嵌套事务应失败（内层无法与外层并发），实际 {inner:?}"
        );
        assert_eq!(name, "outer", "内层未写入；外层提交后可见");
        eprintln!(
            "[CONC-004][红线] {}: 嵌套事务无 savepoint，内层无法并发执行；外层提交成功",
            c.dialect.name()
        );
    } else {
        inner
            .expect("多连接方言上内层事务应成功（独立连接，非 savepoint）")
            .expect("内层事务提交");
        // 内层独立提交：外层回滚不会撤销内层（lgr_009 的语义）
        let inner_name: String = c
            .db()
            .scalar_with_params(
                "SELECT name FROM accounts WHERE id = ?",
                vec![Value::I64(t1_liability())],
            )
            .await
            .expect("读回内层账户名");
        assert_eq!(inner_name, "inner", "内层事务独立提交生效");
        eprintln!(
            "[CONC-004] {}: 内层事务走独立连接（非 savepoint），独立提交",
            c.dialect.name()
        );
    }
}

// ==================== 异常路径归还连接 ====================

/// 并发对抗：20 次异常路径（余额不足）后连接归还池，池仍可用。
#[tokio::test]
async fn conc_005_exception_path_returns_connection() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(10, 0)).await.unwrap();

    for i in 0..20 {
        let err = ledger::post_transfer(c.db(), cmd(40_000 + i, money(1_000_000, 0)))
            .await
            .expect_err("余额不足必须失败");
        assert!(
            matches!(err, DbError::InvalidValue(_)),
            "余额不足应为 InvalidValue，实际 {err:?}"
        );
    }
    // 池未泄漏：正常写入仍可完成（若连接泄漏，这里会等待池超时）
    let ok = ledger::post_transfer(c.db(), cmd(40_100, money(1, 0)))
        .await
        .expect("异常路径后池仍可用");
    assert_eq!(ok.amount, money(1, 0));
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM transfers WHERE tenant_id = ?",
            vec![Value::I64(TENANT_T1)]
        )
        .await,
        1,
        "20 次失败不得留下任何转账记录"
    );
    assert_balance(c.db(), TENANT_T1, t1_asset(), "9").await;
    eprintln!(
        "[CONC-005] {}: 20 次异常路径后连接归还，池仍可用",
        c.dialect.name()
    );
}

// ==================== 4 路并发迁移 ====================

/// 并发对抗：4 个任务并发 `Migrator::run`——版本记录唯一，迁移不重复。
#[tokio::test]
async fn conc_006_four_concurrent_migrations() {
    let Some(c) = ctx().await else { return };

    // 临时迁移目录：单版本、单语句（版本号取业务迁移之外的空档）
    let dir = std::env::temp_dir().join(format!("alun_conc_mig_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("创建临时迁移目录");
    let ddl = match c.dialect {
        Dialect::SqliteMem | Dialect::SqliteFile => {
            "CREATE TABLE conc_probe (id INTEGER NOT NULL PRIMARY KEY, label TEXT NOT NULL)"
        }
        Dialect::Postgres | Dialect::Mysql => {
            "CREATE TABLE conc_probe (id BIGINT NOT NULL PRIMARY KEY, label VARCHAR(64) NOT NULL)"
        }
    };
    std::fs::write(dir.join("V9001__conc_probe.sql"), format!("{ddl}\n")).expect("写入迁移文件");

    // 清场：确保该版本与表均不存在
    c.db()
        .execute("DROP TABLE IF EXISTS conc_probe")
        .await
        .expect("drop conc_probe");
    c.db()
        .execute_with_params(
            "DELETE FROM schema_migrations WHERE version = ?",
            vec![Value::Text("9001".into())],
        )
        .await
        .expect("delete version 9001");

    // 4 路并发
    let mut handles = Vec::with_capacity(4);
    for _ in 0..4 {
        let db = c.db().clone();
        let dir = dir.clone();
        handles.push(tokio::spawn(async move {
            Migrator::from_dir(&dir)
                .run(&db)
                .await
                .map_err(|e| format!("{e:?}"))
        }));
    }
    let mut applied = 0;
    let mut already = 0;
    let mut errors: Vec<String> = Vec::new();
    for handle in handles {
        match handle.await.expect("join") {
            Ok(n) => {
                assert!(n <= 1, "单版本目录最多应用 1 条，实际 {n}");
                applied += n;
                if n == 0 {
                    // 其他任务已应用该版本（无咨询锁下的合法结果）
                    already += 1;
                }
            }
            Err(e) => errors.push(e),
        }
    }
    assert!(applied >= 1, "至少一方应成功应用，错误：{errors:?}");
    assert_eq!(
        applied + already + errors.len(),
        4,
        "4 个任务的结果必须完整统计（applied={applied} already={already} errors={})",
        errors.len()
    );
    for e in &errors {
        assert!(
            e.contains("Query") || e.contains("Migration"),
            "并发迁移失败应落 Query/Migration，实际 {e}"
        );
    }

    // 版本记录唯一 + 表存在 + 幂等
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM schema_migrations WHERE version = ?",
            vec![Value::Text("9001".into())]
        )
        .await,
        1,
        "并发迁移不得重复记录版本"
    );
    let n = count_by(c.db(), "SELECT COUNT(*) FROM conc_probe", vec![]).await;
    assert_eq!(n, 0, "探针表存在且为空");
    let again = {
        let db = c.db().clone();
        let dir = dir.clone();
        Migrator::from_dir(&dir)
            .run(&db)
            .await
            .expect("串行重跑应幂等")
    };
    assert_eq!(again, 0, "已应用版本不再重复执行");
    eprintln!(
        "[CONC-006] {}: 4 路并发迁移 → 成功 {applied} / 失败 {}（{errors:?}），版本记录唯一、重跑幂等",
        c.dialect.name(),
        errors.len()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

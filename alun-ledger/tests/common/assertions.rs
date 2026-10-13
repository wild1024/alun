//! 断言助手。

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use rust_decimal::Decimal;

/// 断言：租户内所有账户余额非负（金额经方言无关解码）。
pub async fn assert_no_negative_balance(db: &Db, tenant_id: i64) {
    let balances = alun_ledger::services::ledger::all_balances(db, tenant_id)
        .await
        .expect("read balances");
    for (id, b) in balances {
        assert!(b >= Decimal::ZERO, "账户 {id} 余额为负: {b}");
    }
}

/// 断言：指定转账借贷平衡（Σ debit = Σ credit）。
pub async fn assert_double_entry_balanced(db: &Db, transfer_id: i64) {
    assert!(
        alun_ledger::services::ledger::transfer_is_balanced(db, transfer_id)
            .await
            .expect("balanced check"),
        "转账 {transfer_id} 借贷不平衡"
    );
}

/// 断言：租户内所有账户余额之和等于期望值。
pub async fn assert_total_balance(db: &Db, tenant_id: i64, expected: Decimal) {
    let balances = alun_ledger::services::ledger::all_balances(db, tenant_id)
        .await
        .expect("read balances");
    let sum: Decimal = balances.iter().map(|(_, b)| *b).sum();
    assert_eq!(sum, expected, "租户 {tenant_id} 余额总和");
}

/// 断言：不存在越租户数据（扫描全部业务表）。
pub async fn assert_tenant_scope(db: &Db, tenant_id: i64) {
    let violations = alun_ledger::services::tenant::scope_violations(db, tenant_id)
        .await
        .expect("scope scan");
    for (table, n) in violations {
        assert_eq!(n, 0, "表 {table} 存在 {n} 行 tenant_id <> {tenant_id}");
    }
}

/// 统计表内行数（带可选过滤）。
pub async fn count_by(db: &Db, sql: &str, params: Vec<Value>) -> i64 {
    db.scalar_with_params::<i64>(sql, params)
        .await
        .expect("count")
}

/// 断言：某账户余额等于期望值（字符串规范化比较，避免 scale 差异）。
pub async fn assert_balance(db: &Db, tenant_id: i64, account_id: i64, expected: &str) {
    let actual = alun_ledger::services::money::account_balance(db, tenant_id, account_id)
        .await
        .expect("read balance");
    assert_eq!(
        alun_ledger::services::money::money_str(actual),
        expected,
        "账户 {account_id} 余额"
    );
}

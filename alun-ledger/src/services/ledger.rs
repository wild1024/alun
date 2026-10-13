//! 复式记账：转账服务（用例 LGR-001..012）。
//!
//! 全部写入在**同一个显式事务**内完成；事务内只能使用 raw SQL，
//! 因为 `PersistExt::save(&Db)` 不接受 `&Transaction`（缺陷 D3）。

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use alun_db_core::traits::TypeMapper;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::models::{Direction, Transfer, TransferStatus};
use crate::services::audit::{self, AuditRecord};
use crate::services::faults::{self, FailPoint};
use crate::services::{money, rowx};

/// 转账指令（主键由调用方给定，保证测试确定性）。
#[derive(Debug, Clone)]
pub struct TransferCommand {
    /// 转账单主键。
    pub transfer_id: i64,
    /// 借方分录主键（源账户）。
    pub debit_entry_id: i64,
    /// 贷方分录主键（目标账户）。
    pub credit_entry_id: i64,
    /// 租户。
    pub tenant_id: i64,
    /// 源账户。
    pub from_account: i64,
    /// 目标账户。
    pub to_account: i64,
    /// 金额（必须为正）。
    pub amount: Decimal,
    /// 操作时间（调用方传入）。
    pub at: DateTime<Utc>,
}

/// 业务错误：余额不足。
pub const ERR_INSUFFICIENT: &str = "insufficient balance";

/// 执行转账：扣源账户、增目标账户、写两条分录 + 一条转账记录，同一事务提交。
pub async fn post_transfer(db: &Db, cmd: TransferCommand) -> Result<Transfer> {
    db.transaction(|tx| async move { post_transfer_tx(&tx, cmd).await })
        .await
}

async fn post_transfer_tx(tx: &Transaction, cmd: TransferCommand) -> Result<Transfer> {
    if cmd.amount <= Decimal::ZERO {
        return Err(DbError::InvalidValue("amount must be positive".into()));
    }

    let from = money::read_one_tx(
        tx,
        "SELECT balance FROM accounts WHERE id = ? AND tenant_id = ?",
        vec![Value::I64(cmd.from_account), Value::I64(cmd.tenant_id)],
    )
    .await?;
    let to = money::read_one_tx(
        tx,
        "SELECT balance FROM accounts WHERE id = ? AND tenant_id = ?",
        vec![Value::I64(cmd.to_account), Value::I64(cmd.tenant_id)],
    )
    .await?;

    if from < cmd.amount {
        // 业务拒绝：此时尚未写入任何行
        return Err(DbError::InvalidValue(ERR_INSUFFICIENT.into()));
    }

    // 1) 扣款：CAS 更新（`balance = 读到的值`），并发修改时影响行数为 0
    let debited = tx
        .execute_with_params(
            "UPDATE accounts SET balance = ? WHERE id = ? AND tenant_id = ? AND balance = ?",
            vec![
                (from - cmd.amount).to_value(),
                Value::I64(cmd.from_account),
                Value::I64(cmd.tenant_id),
                from.to_value(),
            ],
        )
        .await?;
    if debited == 0 {
        // 并发账户被改（框架无 `#[version]` 乐观锁，缺口 G5）→ 显式冲突，交由调用方重试
        return Err(DbError::Transaction(format!(
            "concurrent modification on account {}",
            cmd.from_account
        )));
    }
    faults::check(FailPoint::AfterDebit)?;

    // 2) 入账（同样 CAS）
    faults::check(FailPoint::BeforeCredit)?;
    let credited = tx
        .execute_with_params(
            "UPDATE accounts SET balance = ? WHERE id = ? AND tenant_id = ? AND balance = ?",
            vec![
                (to + cmd.amount).to_value(),
                Value::I64(cmd.to_account),
                Value::I64(cmd.tenant_id),
                to.to_value(),
            ],
        )
        .await?;
    if credited == 0 {
        return Err(DbError::Transaction(format!(
            "concurrent modification on account {}",
            cmd.to_account
        )));
    }

    // 3) 两条分录（借贷平衡）
    for (entry_id, account_id, direction) in [
        (cmd.debit_entry_id, cmd.from_account, Direction::Credit),
        (cmd.credit_entry_id, cmd.to_account, Direction::Debit),
    ] {
        tx.execute_with_params(
            "INSERT INTO ledger_entries (id, tenant_id, account_id, transfer_id, amount, direction, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(entry_id),
                Value::I64(cmd.tenant_id),
                Value::I64(account_id),
                Value::I64(cmd.transfer_id),
                cmd.amount.to_value(),
                Value::Text(direction_key(direction).to_string()),
                cmd.at.to_value(),
            ],
        )
        .await?;
    }

    // 4) 审计（与业务同事务；失败则整体回滚）
    let after = serde_json::json!({
        "transfer_id": cmd.transfer_id,
        "amount": money::money_str(cmd.amount),
        "status": "posted",
    });
    audit::record(
        tx,
        &AuditRecord {
            id: cmd.transfer_id,
            tenant_id: cmd.tenant_id,
            entity_type: "transfer",
            entity_id: cmd.transfer_id,
            action: "post",
            before: None,
            after: Some(&after),
            actor_id: 0,
            at: cmd.at,
        },
    )
    .await?;

    // 5) 转账单
    tx.execute_with_params(
        "INSERT INTO transfers (id, tenant_id, from_account, to_account, amount, status, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        vec![
            Value::I64(cmd.transfer_id),
            Value::I64(cmd.tenant_id),
            Value::I64(cmd.from_account),
            Value::I64(cmd.to_account),
            cmd.amount.to_value(),
            Value::Text("posted".into()),
            cmd.at.to_value(),
        ],
    )
    .await?;

    faults::check(FailPoint::Commit)?;

    Ok(Transfer {
        id: cmd.transfer_id,
        tenant_id: cmd.tenant_id,
        from_account: cmd.from_account,
        to_account: cmd.to_account,
        amount: cmd.amount,
        status: TransferStatus::Posted,
        created_at: cmd.at,
    })
}

fn direction_key(d: Direction) -> &'static str {
    match d {
        Direction::Debit => "debit",
        Direction::Credit => "credit",
    }
}

/// 读取转账（金额走方言无关解码，见缺陷 D1）。
pub async fn load(db: &Db, tenant_id: i64, transfer_id: i64) -> Result<Transfer> {
    let rows = db
        .find_with_params(
            "SELECT * FROM transfers WHERE id = ? AND tenant_id = ?",
            vec![Value::I64(transfer_id), Value::I64(tenant_id)],
        )
        .await?;
    let row = rows.first().ok_or(DbError::NotFound)?;
    Ok(Transfer {
        id: rowx::i64_of(row, "id")?,
        tenant_id: rowx::i64_of(row, "tenant_id")?,
        from_account: rowx::i64_of(row, "from_account")?,
        to_account: rowx::i64_of(row, "to_account")?,
        amount: rowx::decimal_of(row, "amount")?,
        status: TransferStatus::from_value(rowx::value(row, "status")?)?,
        created_at: rowx::ts_of(row, "created_at")?,
    })
}

/// 读取所有账户余额（方言无关；用于「无负余额」等断言）。
pub async fn all_balances(db: &Db, tenant_id: i64) -> Result<Vec<(i64, Decimal)>> {
    let rows = db
        .find_with_params(
            "SELECT id, balance FROM accounts WHERE tenant_id = ? ORDER BY id",
            vec![Value::I64(tenant_id)],
        )
        .await?;
    rows.iter()
        .map(|r| {
            Ok((
                crate::services::rowx::i64_of(r, "id")?,
                crate::services::rowx::decimal_of(r, "balance")?,
            ))
        })
        .collect()
}

/// 分录借贷是否平衡（Σ(amount × sign) == 0）。
pub async fn transfer_is_balanced(db: &Db, transfer_id: i64) -> Result<bool> {
    let rows = db
        .find_with_params(
            "SELECT amount, direction FROM ledger_entries WHERE transfer_id = ?",
            vec![Value::I64(transfer_id)],
        )
        .await?;
    let mut sum = Decimal::ZERO;
    for r in &rows {
        let amount = crate::services::rowx::decimal_of(r, "amount")?;
        let direction = crate::services::rowx::text_of(r, "direction")?;
        sum += if direction == "debit" {
            amount
        } else {
            -amount
        };
    }
    Ok(sum == Decimal::ZERO)
}

/// 某转账的分录条数。
pub async fn entry_count(db: &Db, transfer_id: i64) -> Result<i64> {
    db.scalar_with_params::<i64>(
        "SELECT COUNT(*) FROM ledger_entries WHERE transfer_id = ?",
        vec![Value::I64(transfer_id)],
    )
    .await
}

/// 乐观锁：手写 `UPDATE ... WHERE id = ? AND <version 列> = ?` 的等价演示。
///
/// 框架无 `#[version]` 属性（缺口 G5），这里以账户名做「版本令牌」模拟 CAS。
pub async fn cas_rename(
    db: &Db,
    tenant_id: i64,
    account_id: i64,
    expected_name: &str,
    new_name: &str,
) -> Result<u64> {
    db.execute_with_params(
        "UPDATE accounts SET name = ? WHERE id = ? AND tenant_id = ? AND name = ?",
        vec![
            Value::Text(new_name.to_string()),
            Value::I64(account_id),
            Value::I64(tenant_id),
            Value::Text(expected_name.to_string()),
        ],
    )
    .await
}

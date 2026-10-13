//! 金额适配器。
//!
//! 各方言对金额列的承载方式不同：
//!
//! | 方言 | 列类型 | 读回 `Value` | 类型化读 (`Account::find`) |
//! |---|---|---|---|
//! | SQLite | `TEXT`（无原生 decimal） | `Text` | ✅（类型化读边界解析文本，D001 已修复） |
//! | PostgreSQL | `NUMERIC` | `Decimal` | ✅ |
//! | MySQL | `DECIMAL(38, 10)` | `Decimal` | ✅ |
//!
//! 仍保留本模块的原因：①SQL 侧聚合（`SUM`/`AVG`）在 SQLite 上会把 `TEXT` 列转成
//! `REAL`（浮点误差），必须由本模块显式解码并拒绝浮点结果；②`read_one` 统一了
//! 「金额标量查询」的取值与租户作用域。

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use alun_db_core::traits::TypeMapper;
use rust_decimal::Decimal;
use std::str::FromStr;

/// 把任意方言读回的 `Value` 归一为 [`Decimal`]。
pub fn decode_decimal(v: &Value) -> Result<Decimal> {
    match v {
        // 复用核心的解码（含按 10 的幂约简，容忍驱动按列标度补齐的尾数）
        Value::Decimal(c) => alun_db_core::traits::decimal_from_compact(c.mantissa, c.scale),
        Value::Text(s) => Decimal::from_str(s.trim())
            .map_err(|e| DbError::InvalidValue(format!("invalid decimal text `{s}`: {e}"))),
        Value::I8(n) => Ok(Decimal::from(*n)),
        Value::I16(n) => Ok(Decimal::from(*n)),
        Value::I32(n) => Ok(Decimal::from(*n)),
        Value::I64(n) => Ok(Decimal::from(*n)),
        Value::U8(n) => Ok(Decimal::from(*n)),
        Value::U16(n) => Ok(Decimal::from(*n)),
        Value::U32(n) => Ok(Decimal::from(*n)),
        Value::U64(n) => Ok(Decimal::from(*n)),
        Value::F32(f) => Decimal::from_f32_retain(*f)
            .ok_or_else(|| DbError::InvalidValue(format!("float not representable: {f}"))),
        Value::F64(f) => Decimal::from_f64_retain(*f)
            .ok_or_else(|| DbError::InvalidValue(format!("float not representable: {f}"))),
        Value::Null => Err(DbError::UnexpectedNull("money column")),
        other => Err(DbError::TypeMismatch {
            expected: "Decimal",
            actual: other.ty().name(),
        }),
    }
}

/// 绑定用：`Decimal → Value::Decimal`。
pub fn encode_decimal(d: Decimal) -> Value {
    d.to_value()
}

/// 规范化字符串（去尾随零），用于断言与日志。
pub fn money_str(d: Decimal) -> String {
    d.normalize().to_string()
}

/// 读取单值金额（`SELECT <money> ...` 的首行首列）。
pub async fn read_one(db: &Db, sql: &str, params: Vec<Value>) -> Result<Decimal> {
    let rows = db.find_with_params(sql, params).await?;
    first_decimal(rows)
}

/// 事务内读取单值金额。
pub async fn read_one_tx(tx: &Transaction, sql: &str, params: Vec<Value>) -> Result<Decimal> {
    let rows = tx.find_with_params(sql, params).await?;
    first_decimal(rows)
}

fn first_decimal(rows: Vec<Row>) -> Result<Decimal> {
    let row = rows.into_iter().next().ok_or(DbError::NotFound)?;
    let value = row
        .values()
        .next()
        .ok_or_else(|| DbError::ColumnNotFound("money column missing".into()))?;
    decode_decimal(value)
}

/// 读取某账户余额（带租户作用域）。
pub async fn account_balance(db: &Db, tenant_id: i64, account_id: i64) -> Result<Decimal> {
    read_one(
        db,
        "SELECT balance FROM accounts WHERE id = ? AND tenant_id = ?",
        vec![Value::I64(account_id), Value::I64(tenant_id)],
    )
    .await
}

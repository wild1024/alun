//! `Row` 的按列取值助手（含金额/时间/JSON 的方言无关解码）。
//!
//! 用于「raw SQL + 类型化重建」场景：查询走可移植 SQL，重建模型时对
//! 金额列用 [`crate::services::money::decode_decimal`] 兜底（见缺陷 D1）。

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use alun_db_core::traits::TypeMapper;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::services::money;

/// 取列 `Value` 引用。
pub fn value<'a>(row: &'a Row, name: &str) -> Result<&'a Value> {
    row.get(name)
        .ok_or_else(|| DbError::ColumnNotFound(name.to_string()))
}

/// 取 `i64`。
pub fn i64_of(row: &Row, name: &str) -> Result<i64> {
    i64::from_value(value(row, name)?)
}

/// 取 `String`。
pub fn text_of(row: &Row, name: &str) -> Result<String> {
    String::from_value(value(row, name)?)
}

/// 取可空 `String`。
pub fn opt_text_of(row: &Row, name: &str) -> Result<Option<String>> {
    match value(row, name)? {
        Value::Null => Ok(None),
        v => Ok(Some(String::from_value(v)?)),
    }
}

/// 取金额（方言无关）。
pub fn decimal_of(row: &Row, name: &str) -> Result<Decimal> {
    money::decode_decimal(value(row, name)?)
}

/// 取 `DateTime<Utc>`。
pub fn ts_of(row: &Row, name: &str) -> Result<DateTime<Utc>> {
    DateTime::<Utc>::from_value(value(row, name)?)
}

/// 取可空 `DateTime<Utc>`。
pub fn opt_ts_of(row: &Row, name: &str) -> Result<Option<DateTime<Utc>>> {
    match value(row, name)? {
        Value::Null => Ok(None),
        v => Ok(Some(DateTime::<Utc>::from_value(v)?)),
    }
}

/// 取可空 JSON（`Json` / `Text` 载荷均可）。
pub fn opt_json_of(row: &Row, name: &str) -> Result<Option<serde_json::Value>> {
    match value(row, name)? {
        Value::Null => Ok(None),
        Value::Json(s) | Value::Text(s) => serde_json::from_str(s)
            .map(Some)
            .map_err(|e| DbError::InvalidValue(format!("json decode `{name}`: {e}"))),
        other => Err(DbError::TypeMismatch {
            expected: "Json",
            actual: other.ty().name(),
        }),
    }
}

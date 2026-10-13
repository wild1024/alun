//! `TypeMapper` trait —— Rust 类型与 `stmt::Value` 的转换契约。

use crate::error::{DbError, Result};
// Compact 类型只在对应 feature 的实现模块中使用，按消费方分开通关
#[cfg(feature = "ipnetwork")]
use crate::stmt::compact::CompactCidr;
#[cfg(any(feature = "rust_decimal", feature = "jiff"))]
use crate::stmt::compact::CompactDecimal;
#[cfg(any(feature = "uuid", feature = "ulid"))]
use crate::stmt::compact::CompactId;
#[cfg(any(feature = "chrono", feature = "jiff", feature = "time"))]
use crate::stmt::compact::{CompactDate, CompactTimestampTz};
#[cfg(any(feature = "chrono", feature = "time"))]
use crate::stmt::compact::{CompactTime, CompactTimestamp};
use crate::stmt::{Type, Value};

/// Rust 类型 ↔ `stmt::Value` 的转换契约。
///
/// ## 实现指南
///
/// - `stmt_type()` 返回该 Rust 类型对应的中间类型
/// - `to_value()` 将 Rust 值转为 `Value`
/// - `from_value()` 从 `Value` 恢复 Rust 值，类型不匹配时返回
///   `DbError::TypeMismatch`
///
/// ## 双向可逆性
///
/// 实现者应保证 `from_value(&x.to_value())` 与 `x` 语义等价（在精度
/// 范围内）。这是跨数据库读写不丢失信息的前提。
pub trait TypeMapper: Sized {
    /// 该类型对应的中间类型。
    fn stmt_type() -> Type;

    /// 转为运行时值。
    fn to_value(&self) -> Value;

    /// 从运行时值恢复。
    fn from_value(v: &Value) -> Result<Self>;
}

// ==================== 基础类型 ====================

macro_rules! impl_type_mapper_int {
    // i64：精确变体即 I64（驱动统一返回宽度），单臂直达，避免重复分支
    ($rust_ty:ty, $stmt_ty:ident, I64) => {
        impl TypeMapper for $rust_ty {
            fn stmt_type() -> Type {
                Type::$stmt_ty
            }
            fn to_value(&self) -> Value {
                Value::I64(*self)
            }
            fn from_value(v: &Value) -> Result<Self> {
                match v {
                    Value::I64(n) => Ok(*n),
                    Value::U64(n) => <$rust_ty>::try_from(*n).map_err(|_| DbError::TypeMismatch {
                        expected: stringify!($stmt_ty),
                        actual: v.ty().name(),
                    }),
                    _ => Err(DbError::TypeMismatch {
                        expected: stringify!($stmt_ty),
                        actual: v.ty().name(),
                    }),
                }
            }
        }
    };
    // u64：与 i64 对称
    ($rust_ty:ty, $stmt_ty:ident, U64) => {
        impl TypeMapper for $rust_ty {
            fn stmt_type() -> Type {
                Type::$stmt_ty
            }
            fn to_value(&self) -> Value {
                Value::U64(*self)
            }
            fn from_value(v: &Value) -> Result<Self> {
                match v {
                    Value::I64(n) => <$rust_ty>::try_from(*n).map_err(|_| DbError::TypeMismatch {
                        expected: stringify!($stmt_ty),
                        actual: v.ty().name(),
                    }),
                    Value::U64(n) => Ok(*n),
                    _ => Err(DbError::TypeMismatch {
                        expected: stringify!($stmt_ty),
                        actual: v.ty().name(),
                    }),
                }
            }
        }
    };
    // 其余宽度：精确变体 + 驱动统一返回的 I64/U64 宽容匹配
    ($rust_ty:ty, $stmt_ty:ident, $value_variant:ident) => {
        impl TypeMapper for $rust_ty {
            fn stmt_type() -> Type {
                Type::$stmt_ty
            }
            fn to_value(&self) -> Value {
                Value::$value_variant(*self)
            }
            fn from_value(v: &Value) -> Result<Self> {
                match v {
                    Value::$value_variant(x) => Ok(*x),
                    Value::I64(n) => <$rust_ty>::try_from(*n).map_err(|_| DbError::TypeMismatch {
                        expected: stringify!($stmt_ty),
                        actual: v.ty().name(),
                    }),
                    Value::U64(n) => <$rust_ty>::try_from(*n).map_err(|_| DbError::TypeMismatch {
                        expected: stringify!($stmt_ty),
                        actual: v.ty().name(),
                    }),
                    _ => Err(DbError::TypeMismatch {
                        expected: stringify!($stmt_ty),
                        actual: v.ty().name(),
                    }),
                }
            }
        }
    };
}

impl_type_mapper_int!(i8, I8, I8);
impl_type_mapper_int!(i16, I16, I16);
impl_type_mapper_int!(i32, I32, I32);
impl_type_mapper_int!(i64, I64, I64);
impl_type_mapper_int!(u8, U8, U8);
impl_type_mapper_int!(u16, U16, U16);
impl_type_mapper_int!(u32, U32, U32);
impl_type_mapper_int!(u64, U64, U64);

/// 取 `#[int]` 枚举的判别值。
///
/// `#[int]` 模式的枚举以 `Value::I32` 写出，而驱动对整数列统一以
/// `Value::I64` 读回（宽度归一）。判别值读取在此收口：接受任意宽度整数
/// （`I8`/`I16`/`I32`/`I64`），非整数变体或超出 `i32` 范围时返回
/// `TypeMismatch { expected: "I32", .. }`。
pub fn int_discriminant(v: &Value) -> Result<i32> {
    match v {
        Value::I8(n) => Ok(*n as i32),
        Value::I16(n) => Ok(*n as i32),
        Value::I32(n) => Ok(*n),
        Value::I64(n) => i32::try_from(*n).map_err(|_| DbError::TypeMismatch {
            expected: "I32",
            actual: v.ty().name(),
        }),
        _ => Err(DbError::TypeMismatch {
            expected: "I32",
            actual: v.ty().name(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_discriminant_accepts_every_integer_width() {
        assert_eq!(int_discriminant(&Value::I8(2)).unwrap(), 2);
        assert_eq!(int_discriminant(&Value::I16(2)).unwrap(), 2);
        assert_eq!(int_discriminant(&Value::I32(2)).unwrap(), 2);
        // 驱动统一读回宽度（typ_007：整数列一律 I64）
        assert_eq!(int_discriminant(&Value::I64(2)).unwrap(), 2);
        assert_eq!(int_discriminant(&Value::I64(-1)).unwrap(), -1);
    }

    #[test]
    fn int_discriminant_rejects_non_integer_and_overflow() {
        for bad in [
            Value::Text("2".into()),
            Value::Null,
            Value::Bool(true),
            Value::F64(2.0),
            Value::I64(i64::MAX),
        ] {
            match int_discriminant(&bad) {
                Err(DbError::TypeMismatch { expected, actual }) => {
                    assert_eq!(expected, "I32");
                    assert_eq!(actual, bad.ty().name());
                }
                other => panic!("{bad:?} 应 TypeMismatch，实际 {other:?}"),
            }
        }
    }

    #[test]
    fn bool_is_not_silently_a_string_or_integer() {
        // `Value::Bool` 只能落回 bool：不得被文本/整数读法静默接受
        assert!(matches!(
            String::from_value(&Value::Bool(false)),
            Err(DbError::TypeMismatch {
                expected: "Text",
                actual: "Bool"
            })
        ));
        assert!(matches!(
            i64::from_value(&Value::Bool(false)),
            Err(DbError::TypeMismatch {
                expected: "I64",
                actual: "Bool"
            })
        ));
        assert!(!bool::from_value(&Value::Bool(false)).unwrap());
    }
}

impl TypeMapper for bool {
    fn stmt_type() -> Type {
        Type::Bool
    }
    fn to_value(&self) -> Value {
        Value::Bool(*self)
    }
    fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::Bool(b) => Ok(*b),
            _ => Err(DbError::TypeMismatch {
                expected: "Bool",
                actual: v.ty().name(),
            }),
        }
    }
}

impl TypeMapper for f32 {
    fn stmt_type() -> Type {
        Type::F32
    }
    fn to_value(&self) -> Value {
        Value::F32(*self)
    }
    fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::F32(x) => Ok(*x),
            _ => Err(DbError::TypeMismatch {
                expected: "F32",
                actual: v.ty().name(),
            }),
        }
    }
}

impl TypeMapper for f64 {
    fn stmt_type() -> Type {
        Type::F64
    }
    fn to_value(&self) -> Value {
        Value::F64(*self)
    }
    fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::F64(x) => Ok(*x),
            _ => Err(DbError::TypeMismatch {
                expected: "F64",
                actual: v.ty().name(),
            }),
        }
    }
}

impl TypeMapper for String {
    fn stmt_type() -> Type {
        Type::Text
    }
    fn to_value(&self) -> Value {
        Value::Text(self.clone())
    }
    fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::Text(s) => Ok(s.clone()),
            _ => Err(DbError::TypeMismatch {
                expected: "Text",
                actual: v.ty().name(),
            }),
        }
    }
}

impl TypeMapper for Vec<u8> {
    fn stmt_type() -> Type {
        Type::Bytes
    }
    fn to_value(&self) -> Value {
        Value::Bytes(self.clone())
    }
    fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::Bytes(b) => Ok(b.clone()),
            _ => Err(DbError::TypeMismatch {
                expected: "Bytes",
                actual: v.ty().name(),
            }),
        }
    }
}

// ==================== 复合类型 ====================

impl<T: TypeMapper> TypeMapper for Option<T> {
    fn stmt_type() -> Type {
        T::stmt_type().nullable()
    }
    fn to_value(&self) -> Value {
        match self {
            None => Value::Null,
            Some(v) => v.to_value(),
        }
    }
    fn from_value(v: &Value) -> Result<Self> {
        match v {
            Value::Null => Ok(None),
            _ => T::from_value(v).map(Some),
        }
    }
}

// 注意：`Vec<T>` 的数组类型映射（`Type::Array`）在 Phase 2 随 PostgreSQL
// 数组类型支持一并加入。这里不提供 blanket impl，因为它会与上方
// `impl TypeMapper for Vec<u8> { Type::Bytes }` 冲突（E0119）。

// ==================== 内部辅助 ====================

/// 从 `Value` 中提取 `compact` 类型。转换失败返回 `TypeMismatch`。
// 该宏仅被启用了对应 feature（如 chrono 等）的条件编译模块使用，
// 因此在默认特性编译下会触发 unused_macros，此处保留并抑制。
#[allow(unused_macros)]
macro_rules! extract_compact {
    ($v:expr, $variant:ident, $ty_name:literal) => {
        match $v {
            // compact 类型（如 CompactCidr）非 Copy，取所有权需 clone
            Value::$variant(x) => Ok(x.clone()),
            _ => Err(DbError::TypeMismatch {
                expected: $ty_name,
                actual: $v.ty().name(),
            }),
        }
    };
}

// ==================== 生态类型实现 ====================

#[cfg(feature = "chrono")]
mod chrono_impls {
    use super::*;
    use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};

    impl TypeMapper for NaiveDate {
        fn stmt_type() -> Type {
            Type::Date
        }
        fn to_value(&self) -> Value {
            let days = (*self - NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()).num_days() as i32;
            Value::Date(CompactDate(days))
        }
        fn from_value(v: &Value) -> Result<Self> {
            let CompactDate(days) = extract_compact!(v, Date, "Date")?;
            NaiveDate::from_ymd_opt(1970, 1, 1)
                .unwrap()
                .checked_add_signed(chrono::Duration::days(days as i64))
                .ok_or_else(|| DbError::InvalidValue(format!("invalid date: {days}")))
        }
    }

    impl TypeMapper for NaiveTime {
        fn stmt_type() -> Type {
            Type::Time
        }
        fn to_value(&self) -> Value {
            let secs = self.hour() as u64 * 3600 + self.minute() as u64 * 60 + self.second() as u64;
            let nanos = secs * 1_000_000_000 + self.nanosecond() as u64;
            Value::Time(CompactTime(nanos))
        }
        fn from_value(v: &Value) -> Result<Self> {
            let CompactTime(nanos) = extract_compact!(v, Time, "Time")?;
            let secs = (nanos / 1_000_000_000) as u32;
            let nsecs = (nanos % 1_000_000_000) as u32;
            NaiveTime::from_num_seconds_from_midnight_opt(secs, nsecs)
                .ok_or_else(|| DbError::InvalidValue(format!("invalid time: {nanos}")))
        }
    }

    impl TypeMapper for NaiveDateTime {
        fn stmt_type() -> Type {
            Type::Timestamp
        }
        fn to_value(&self) -> Value {
            let micros = self.and_utc().timestamp_micros();
            Value::Timestamp(CompactTimestamp(micros))
        }
        fn from_value(v: &Value) -> Result<Self> {
            let CompactTimestamp(micros) = extract_compact!(v, Timestamp, "Timestamp")?;
            DateTime::from_timestamp_micros(micros)
                .map(|dt| dt.naive_utc())
                .ok_or_else(|| DbError::InvalidValue(format!("invalid timestamp: {micros}")))
        }
    }

    impl TypeMapper for DateTime<Utc> {
        fn stmt_type() -> Type {
            Type::TimestampTz
        }
        fn to_value(&self) -> Value {
            Value::TimestampTz(CompactTimestampTz {
                utc_micros: self.timestamp_micros(),
                offset_secs: 0,
            })
        }
        fn from_value(v: &Value) -> Result<Self> {
            match v {
                Value::TimestampTz(ts) => DateTime::from_timestamp_micros(ts.utc_micros)
                    .ok_or_else(|| {
                        DbError::InvalidValue(format!("invalid timestamp: {}", ts.utc_micros))
                    }),
                // SQLite 等驱动把时间戳序列化为文本存储，读出为 Text，这里解析。
                Value::Text(s) => NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.6f")
                    .map(|ndt| DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc))
                    .map_err(|e| DbError::InvalidValue(format!("parse timestamp `{s}`: {e}"))),
                _ => Err(DbError::TypeMismatch {
                    expected: "TimestampTz",
                    actual: v.ty().name(),
                }),
            }
        }
    }

    impl TypeMapper for DateTime<chrono::FixedOffset> {
        fn stmt_type() -> Type {
            Type::TimestampTz
        }
        fn to_value(&self) -> Value {
            Value::TimestampTz(CompactTimestampTz {
                utc_micros: self.timestamp_micros(),
                offset_secs: self.offset().local_minus_utc(),
            })
        }
        fn from_value(v: &Value) -> Result<Self> {
            match v {
                Value::TimestampTz(ts) => {
                    let offset =
                        chrono::FixedOffset::east_opt(ts.offset_secs).ok_or_else(|| {
                            DbError::InvalidValue(format!("invalid offset: {}", ts.offset_secs))
                        })?;
                    let utc = DateTime::from_timestamp_micros(ts.utc_micros).ok_or_else(|| {
                        DbError::InvalidValue(format!("invalid timestamp: {}", ts.utc_micros))
                    })?;
                    Ok(utc.with_timezone(&offset))
                }
                _ => Err(DbError::TypeMismatch {
                    expected: "TimestampTz",
                    actual: v.ty().name(),
                }),
            }
        }
    }
}

#[cfg(feature = "time")]
mod time_impls {
    use super::*;
    use time::{Date, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

    impl TypeMapper for Date {
        fn stmt_type() -> Type {
            Type::Date
        }
        fn to_value(&self) -> Value {
            // 借道 UTC 午夜时刻换算 epoch 天数，与 `CompactDate` 对应
            let days =
                self.midnight().assume_utc().unix_timestamp_nanos() / (86_400 * 1_000_000_000);
            Value::Date(CompactDate(days as i32))
        }
        fn from_value(v: &Value) -> Result<Self> {
            let CompactDate(days) = extract_compact!(v, Date, "Date")?;
            OffsetDateTime::from_unix_timestamp_nanos(days as i128 * 86_400 * 1_000_000_000)
                .map(|odt| odt.to_offset(UtcOffset::UTC).date())
                .map_err(|e| DbError::InvalidValue(format!("invalid date: {e}")))
        }
    }

    impl TypeMapper for Time {
        fn stmt_type() -> Type {
            Type::Time
        }
        fn to_value(&self) -> Value {
            // 自午夜起的纳秒数，与 `CompactTime` 不变量一致
            let nanos =
                (self.hour() as u64 * 3600 + self.minute() as u64 * 60 + self.second() as u64)
                    * 1_000_000_000
                    + self.nanosecond() as u64;
            Value::Time(CompactTime(nanos))
        }
        fn from_value(v: &Value) -> Result<Self> {
            let CompactTime(nanos) = extract_compact!(v, Time, "Time")?;
            let secs = nanos / 1_000_000_000;
            Time::from_hms_nano(
                (secs / 3600) as u8,
                ((secs % 3600) / 60) as u8,
                (secs % 60) as u8,
                (nanos % 1_000_000_000) as u32,
            )
            .map_err(|e| DbError::InvalidValue(format!("invalid time: {e}")))
        }
    }

    impl TypeMapper for PrimitiveDateTime {
        fn stmt_type() -> Type {
            Type::Timestamp
        }
        fn to_value(&self) -> Value {
            // 无时区时间戳按 UTC 假定换算为 epoch 微秒，与 chrono NaiveDateTime 语义对齐
            Value::Timestamp(CompactTimestamp(
                (self.assume_utc().unix_timestamp_nanos() / 1000) as i64,
            ))
        }
        fn from_value(v: &Value) -> Result<Self> {
            let CompactTimestamp(micros) = extract_compact!(v, Timestamp, "Timestamp")?;
            OffsetDateTime::from_unix_timestamp_nanos(micros as i128 * 1000)
                .map_err(|e| DbError::InvalidValue(format!("invalid timestamp: {e}")))
                .map(|odt| {
                    // 取 UTC 时刻的日历日期与时刻，重组无时区时间戳
                    let utc = odt.to_offset(UtcOffset::UTC);
                    PrimitiveDateTime::new(utc.date(), utc.time())
                })
        }
    }

    impl TypeMapper for OffsetDateTime {
        fn stmt_type() -> Type {
            Type::TimestampTz
        }
        fn to_value(&self) -> Value {
            Value::TimestampTz(CompactTimestampTz {
                utc_micros: (self.unix_timestamp_nanos() / 1000) as i64,
                offset_secs: self.offset().whole_seconds(),
            })
        }
        fn from_value(v: &Value) -> Result<Self> {
            let CompactTimestampTz {
                utc_micros,
                offset_secs,
            } = extract_compact!(v, TimestampTz, "TimestampTz")?;
            let offset = UtcOffset::from_whole_seconds(offset_secs)
                .map_err(|e| DbError::InvalidValue(format!("invalid offset: {e}")))?;
            OffsetDateTime::from_unix_timestamp_nanos(utc_micros as i128 * 1000)
                .map(|odt| odt.to_offset(offset))
                .map_err(|e| DbError::InvalidValue(format!("invalid timestamp: {e}")))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use time::Month;

        #[test]
        fn date_roundtrip() {
            let d = Date::from_calendar_date(2024, Month::March, 15).unwrap();
            assert_eq!(Date::from_value(&d.to_value()).unwrap(), d);
        }

        #[test]
        fn time_roundtrip() {
            let t = Time::from_hms_nano(13, 45, 30, 123).unwrap();
            assert_eq!(Time::from_value(&t.to_value()).unwrap(), t);
        }

        #[test]
        fn timestamp_roundtrip() {
            let dt = PrimitiveDateTime::new(
                Date::from_calendar_date(2020, Month::January, 2).unwrap(),
                Time::from_hms_nano(0, 0, 0, 0).unwrap(),
            );
            assert_eq!(PrimitiveDateTime::from_value(&dt.to_value()).unwrap(), dt);
        }

        #[test]
        fn offsetdatetime_roundtrip() {
            let ts = OffsetDateTime::from_unix_timestamp_nanos(1_700_000_000_000_000_000)
                .unwrap()
                .to_offset(UtcOffset::from_whole_seconds(8 * 3600).unwrap());
            assert_eq!(OffsetDateTime::from_value(&ts.to_value()).unwrap(), ts);
        }
    }
}
#[cfg(feature = "rust_decimal")]
/// 由 `CompactDecimal` 构造 `rust_decimal::Decimal`。
///
/// 驱动可能返回「已按列标度补齐」的表示（如 `DECIMAL(38,10)` 列读出
/// `mantissa = 10^36, scale = 10`），此时尾数超出 `rust_decimal` 的 96 位容量，
/// 但**数值本身可表示**（= 10^26）。因此先尝试直接构造，失败时按 10 的幂约简
/// 尾数/标度（不改变数值）后重试。
pub fn decimal_from_compact(
    mantissa: i128,
    scale: u32,
) -> crate::error::Result<rust_decimal::Decimal> {
    if let Ok(d) = rust_decimal::Decimal::try_from_i128_with_scale(mantissa, scale) {
        return Ok(d);
    }
    let mut mantissa = mantissa;
    let mut scale = scale;
    while scale > 0 && mantissa % 10 == 0 {
        mantissa /= 10;
        scale -= 1;
        if let Ok(d) = rust_decimal::Decimal::try_from_i128_with_scale(mantissa, scale) {
            return Ok(d);
        }
    }
    Err(DbError::InvalidValue(
        "invalid decimal: Number exceeds maximum value that can be represented.".into(),
    ))
}

#[cfg(feature = "rust_decimal")]
mod rust_decimal_impls {
    use super::*;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    impl TypeMapper for Decimal {
        fn stmt_type() -> Type {
            Type::Decimal
        }
        fn to_value(&self) -> Value {
            Value::Decimal(CompactDecimal {
                mantissa: self.mantissa(),
                scale: self.scale(),
            })
        }
        fn from_value(v: &Value) -> Result<Self> {
            match v {
                Value::Decimal(CompactDecimal { mantissa, scale }) => {
                    decimal_from_compact(*mantissa, *scale)
                }
                // SQLite 等驱动以 TEXT 承载十进制（列声明为 TEXT，避免 REAL
                // 精度丢失），读出为 Text，这里解析。文本无法表示时给出
                // `InvalidValue` 而非静默截断。
                Value::Text(s) => Decimal::from_str(s.trim())
                    .map_err(|e| DbError::InvalidValue(format!("parse decimal `{s}`: {e}"))),
                _ => Err(DbError::TypeMismatch {
                    expected: "Decimal",
                    actual: v.ty().name(),
                }),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn decimal_text_tolerance() {
            // SQLite 金额列（TEXT）读回后仍可落回 Decimal
            let from_text =
                Decimal::from_value(&Value::Text("-0.01".into())).expect("TEXT 承载的十进制可解析");
            assert_eq!(
                from_text,
                Decimal::from_str("-0.01").unwrap(),
                "文本十进制解析结果与实际值一致"
            );
            assert!(from_text.is_sign_negative(), "-0.01 的符号位保留");

            // 与写入方向（to_value → Value::Decimal）往返一致
            let d = Decimal::from_str("123.45").unwrap();
            assert_eq!(Decimal::from_value(&d.to_value()).unwrap(), d);
            // 文本往返（驱动以 TEXT 落库）同样一致
            assert_eq!(
                Decimal::from_value(&Value::Text("123.45".into())).unwrap(),
                d
            );
        }

        #[test]
        fn decimal_text_rejects_unrepresentable() {
            // 38 位字面量超出 rust_decimal 96 位尾数：显式报错，不静默截断
            let huge = "12345678901234567890123456789012345678";
            assert!(matches!(
                Decimal::from_value(&Value::Text(huge.into())),
                Err(DbError::InvalidValue(_))
            ));
            assert!(matches!(
                Decimal::from_value(&Value::Text("not a number".into())),
                Err(DbError::InvalidValue(_))
            ));
            // 非十进制变体仍是 TypeMismatch
            assert!(matches!(
                Decimal::from_value(&Value::Bool(false)),
                Err(DbError::TypeMismatch {
                    expected: "Decimal",
                    actual: "Bool"
                })
            ));
            assert!(matches!(
                Decimal::from_value(&Value::Null),
                Err(DbError::TypeMismatch {
                    expected: "Decimal",
                    ..
                })
            ));
        }
    }
}

#[cfg(feature = "uuid")]
mod uuid_impls {
    use super::*;
    use uuid::Uuid;

    impl TypeMapper for Uuid {
        fn stmt_type() -> Type {
            Type::Uuid
        }
        fn to_value(&self) -> Value {
            Value::Uuid(CompactId(*self.as_bytes()))
        }
        fn from_value(v: &Value) -> Result<Self> {
            match v {
                Value::Uuid(CompactId(bytes)) => Ok(Uuid::from_bytes(*bytes)),
                // SQLite 以 TEXT 承载 UUID（列声明为 TEXT），读出为 Text。
                // 只接受两种规范形式：32 位十六进制、36 位连字符形式；
                // `urn:uuid:` / 花括号 / 短横线错位等变体一律拒绝（避免标识符被宽松解析）。
                Value::Text(s) => {
                    let text = s.trim();
                    let canonical = match text.len() {
                        32 => text.bytes().all(|b| b.is_ascii_hexdigit()),
                        36 => {
                            text.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
                                && text.bytes().filter(|b| *b == b'-').count() == 4
                        }
                        _ => false,
                    };
                    if !canonical {
                        return Err(DbError::TypeMismatch {
                            expected: "Uuid",
                            actual: v.ty().name(),
                        });
                    }
                    Uuid::try_parse(text).map_err(|_| DbError::TypeMismatch {
                        expected: "Uuid",
                        actual: v.ty().name(),
                    })
                }
                _ => Err(DbError::TypeMismatch {
                    expected: "Uuid",
                    actual: v.ty().name(),
                }),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn uuid_text_tolerance() {
            let uid = Uuid::parse_str("018f4b2c-1111-7222-8333-444455556666").unwrap();
            // SQLite 落 TEXT → 读回 Text → 类型化读仍可恢复标识类型
            assert_eq!(
                Uuid::from_value(&Value::Text(uid.to_string())).unwrap(),
                uid,
                "36 位连字符形式"
            );
            assert_eq!(
                Uuid::from_value(&Value::Text(uid.simple().to_string())).unwrap(),
                uid,
                "32 位简单形式"
            );
            assert_eq!(
                Uuid::from_value(&Value::Text(uid.to_string().to_uppercase())).unwrap(),
                uid,
                "大写十六进制"
            );
            assert_eq!(Uuid::from_value(&uid.to_value()).unwrap(), uid);
        }

        #[test]
        fn uuid_text_rejects_malformed() {
            for bad in [
                "not-a-uuid",
                "018f4b2c-11117-222-8333-444455556666",
                "urn:uuid:018f4b2c-1111-7222-8333-444455556666",
                "018f4b2c1111",
                "",
            ] {
                match Uuid::from_value(&Value::Text(bad.into())) {
                    Err(DbError::TypeMismatch { expected, actual }) => {
                        assert_eq!((expected, actual), ("Uuid", "Text"), "非法文本 `{bad}`");
                    }
                    other => panic!("非法 UUID 文本 `{bad}` 应 TypeMismatch，实际 {other:?}"),
                }
            }
            assert!(matches!(
                Uuid::from_value(&Value::I64(0)),
                Err(DbError::TypeMismatch {
                    expected: "Uuid",
                    actual: "I64"
                })
            ));
        }
    }
}

#[cfg(feature = "ulid")]
mod ulid_impls {
    use super::*;
    use ulid::Ulid;

    impl TypeMapper for Ulid {
        fn stmt_type() -> Type {
            Type::Ulid
        }
        fn to_value(&self) -> Value {
            Value::Ulid(CompactId(self.to_bytes()))
        }
        fn from_value(v: &Value) -> Result<Self> {
            let CompactId(bytes) = extract_compact!(v, Ulid, "Ulid")?;
            Ok(Ulid::from_bytes(bytes))
        }
    }
}

#[cfg(feature = "json")]
mod json_impls {
    use super::*;
    use serde_json::Value as JsonValue;

    impl TypeMapper for JsonValue {
        fn stmt_type() -> Type {
            Type::Json
        }
        fn to_value(&self) -> Value {
            Value::Json(self.to_string())
        }
        fn from_value(v: &Value) -> Result<Self> {
            match v {
                Value::Json(s) => serde_json::from_str(s)
                    .map_err(|e| DbError::InvalidValue(format!("invalid JSON: {e}"))),
                _ => Err(DbError::TypeMismatch {
                    expected: "Json",
                    actual: v.ty().name(),
                }),
            }
        }
    }
}

#[cfg(feature = "ipnetwork")]
mod ipnetwork_impls {
    use super::*;
    use ipnetwork::IpNetwork;

    impl TypeMapper for std::net::IpAddr {
        fn stmt_type() -> Type {
            Type::Inet
        }
        fn to_value(&self) -> Value {
            Value::Inet(*self)
        }
        fn from_value(v: &Value) -> Result<Self> {
            match v {
                Value::Inet(a) => Ok(*a),
                _ => Err(DbError::TypeMismatch {
                    expected: "Inet",
                    actual: v.ty().name(),
                }),
            }
        }
    }

    impl TypeMapper for IpNetwork {
        fn stmt_type() -> Type {
            Type::Cidr
        }
        fn to_value(&self) -> Value {
            Value::Cidr(CompactCidr {
                addr: self.ip(),
                prefix_len: self.prefix(),
            })
        }
        fn from_value(v: &Value) -> Result<Self> {
            let cidr = extract_compact!(v, Cidr, "Cidr")?;
            IpNetwork::new(cidr.addr, cidr.prefix_len)
                .map_err(|e| DbError::InvalidValue(format!("invalid CIDR: {e}")))
        }
    }
}

#[cfg(feature = "jiff")]
mod jiff_impls {
    use super::*;
    use jiff::{Timestamp, Zoned, civil};

    /// jiff::civil::Date → stmt::Type::Date
    impl TypeMapper for civil::Date {
        fn stmt_type() -> Type {
            Type::Date
        }
        fn to_value(&self) -> Value {
            // jiff::civil::Date 无直接 epoch 天数；借道 UTC Zoned 换算，与 CompactDate 对应
            let ts = self.to_zoned(jiff::tz::TimeZone::UTC).unwrap();
            Value::Date(CompactDate((ts.timestamp().as_second() / 86400) as i32))
        }
        fn from_value(v: &Value) -> Result<Self> {
            let CompactDate(days) = extract_compact!(v, Date, "Date")?;
            let ts = Timestamp::from_second(days as i64 * 86400)
                .map_err(|e| DbError::InvalidValue(format!("invalid date: {e}")))?;
            // 从 UTC 时刻取日历日期（`Zoned::date()` 返回 civil::Date）
            Ok(ts.to_zoned(jiff::tz::TimeZone::UTC).date())
        }
    }

    /// jiff::Timestamp → stmt::Type::TimestampTz
    impl TypeMapper for Timestamp {
        fn stmt_type() -> Type {
            Type::TimestampTz
        }
        fn to_value(&self) -> Value {
            Value::TimestampTz(CompactTimestampTz {
                utc_micros: self.as_microsecond(),
                offset_secs: 0,
            })
        }
        fn from_value(v: &Value) -> Result<Self> {
            match v {
                Value::TimestampTz(ts) => Timestamp::from_microsecond(ts.utc_micros)
                    .map_err(|e| DbError::InvalidValue(format!("invalid timestamp: {e}"))),
                _ => Err(DbError::TypeMismatch {
                    expected: "TimestampTz",
                    actual: v.ty().name(),
                }),
            }
        }
    }

    /// jiff::Zoned → stmt::Type::TimestampTz（保留时区）
    impl TypeMapper for Zoned {
        fn stmt_type() -> Type {
            Type::TimestampTz
        }
        fn to_value(&self) -> Value {
            Value::TimestampTz(CompactTimestampTz {
                utc_micros: self.timestamp().as_microsecond(),
                offset_secs: self.offset().seconds(),
            })
        }
        fn from_value(v: &Value) -> Result<Self> {
            let CompactTimestampTz {
                utc_micros,
                offset_secs,
            } = extract_compact!(v, TimestampTz, "TimestampTz")?;
            let ts = Timestamp::from_microsecond(utc_micros)
                .map_err(|e| DbError::InvalidValue(format!("invalid timestamp: {e}")))?;
            let offset = jiff::tz::Offset::from_seconds(offset_secs)
                .map_err(|e| DbError::InvalidValue(format!("invalid offset: {e}")))?;
            Ok(ts.to_zoned(jiff::tz::TimeZone::fixed(offset)))
        }
    }
}
#[cfg(feature = "bigdecimal")]
mod bigdecimal_impls {
    use super::*;
    use bigdecimal::BigDecimal;
    use std::str::FromStr;

    impl TypeMapper for BigDecimal {
        fn stmt_type() -> Type {
            Type::Decimal
        }
        fn to_value(&self) -> Value {
            // BigDecimal 没有直接的 mantissa/scale 访问
            // 通过字符串表示中转
            let s = self.to_string();
            let (mantissa, scale) = parse_decimal_string(&s);
            Value::Decimal(CompactDecimal { mantissa, scale })
        }
        fn from_value(v: &Value) -> Result<Self> {
            let CompactDecimal { mantissa, scale } = extract_compact!(v, Decimal, "Decimal")?;
            let s = format_decimal(mantissa, scale);
            BigDecimal::from_str(&s)
                .map_err(|e| DbError::InvalidValue(format!("invalid decimal: {e}")))
        }
    }

    /// 将十进制字符串解析为 (mantissa, scale)
    fn parse_decimal_string(s: &str) -> (i128, u32) {
        let s = s.trim();
        let (int_part, frac_part) = s.split_once('.').unwrap_or((s, ""));
        let scale = frac_part.len() as u32;
        let combined = format!("{}{}", int_part, frac_part);
        let mantissa = combined.parse::<i128>().unwrap_or(0);
        (mantissa, scale)
    }

    /// 将 (mantissa, scale) 格式化为十进制字符串
    fn format_decimal(mantissa: i128, scale: u32) -> String {
        let s = mantissa.abs().to_string();
        let sign = if mantissa < 0 { "-" } else { "" };
        if scale == 0 {
            format!("{sign}{s}")
        } else {
            let s = if s.len() <= scale as usize {
                format!("0.{}{}", "0".repeat(scale as usize - s.len()), s)
            } else {
                let (int_part, frac_part) = s.split_at(s.len() - scale as usize);
                format!("{int_part}.{frac_part}")
            };
            format!("{sign}{s}")
        }
    }
}

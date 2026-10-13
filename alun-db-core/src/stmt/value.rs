//! `stmt::Value` —— 运行时值的统一表示。

use super::Type;
use super::compact::{
    CompactCidr, CompactDate, CompactDecimal, CompactId, CompactTime, CompactTimestamp,
    CompactTimestampTz,
};
use std::net::IpAddr;

/// 运行时值的统一表示。
///
/// 所有参数绑定、查询结果、行读取都经过此类型。`TypeMapper` 负责
/// 将具体的 Rust 类型转为 `Value`，方言驱动负责将 `Value` 绑定到
/// 数据库参数或从结果集构造 `Value`。
///
/// ## 不变量
///
/// 1. `Null` 只能出现在 nullable 列或 nullable 参数位置；非 nullable
///    列读到 `Null` 时报 `DbError::UnexpectedNull`。
/// 2. `Json` 存储的必须是合法 JSON 字符串；构造时不校验（由 `TypeMapper`
///    保证），但读取时驱动若发现非法 JSON 则报错。
/// 3. `Array` 内元素类型必须一致；由 `TypeMapper` 和方言驱动共同保证。
/// 4. `Enum(String)` 的字符串必须是 schema 中声明的枚举值之一；方言渲染
///    为原生 ENUM 时，驱动在 bind 前校验。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    // ==================== 空值 ====================
    /// 空值。仅用于 nullable 列或 nullable 参数。
    Null,

    // ==================== 布尔 ====================
    /// 布尔值。
    Bool(bool),

    // ==================== 有符号整数 ====================
    /// 8 位有符号整数。
    I8(i8),
    /// 16 位有符号整数。
    I16(i16),
    /// 32 位有符号整数。
    I32(i32),
    /// 64 位有符号整数。
    I64(i64),

    // ==================== 无符号整数 ====================
    /// 8 位无符号整数。
    U8(u8),
    /// 16 位无符号整数。
    U16(u16),
    /// 32 位无符号整数。
    U32(u32),
    /// 64 位无符号整数。
    U64(u64),

    // ==================== 浮点数 ====================
    /// 32 位浮点数。
    F32(f32),
    /// 64 位浮点数。
    F64(f64),

    // ==================== 文本与二进制 ====================
    /// 变长文本。
    Text(String),
    /// 变长二进制。
    Bytes(Vec<u8>),

    // ==================== 时间 ====================
    /// 无时区日期。
    Date(CompactDate),
    /// 无时区时间。
    Time(CompactTime),
    /// 无时区时间戳。
    Timestamp(CompactTimestamp),
    /// 带时区时间戳。
    TimestampTz(CompactTimestampTz),

    // ==================== 标识 ====================
    /// UUID。
    Uuid(CompactId),
    /// ULID。
    Ulid(CompactId),

    // ==================== 精确数值 ====================
    /// 十进制数。
    Decimal(CompactDecimal),

    // ==================== 结构化 ====================
    /// JSON 文档的序列化字符串。
    Json(String),
    /// 枚举的字符串表示。
    Enum(String),

    // ==================== 网络 ====================
    /// IP 地址。
    Inet(IpAddr),
    /// CIDR 网段。
    Cidr(CompactCidr),

    // ==================== 复合 ====================
    /// 数组。
    Array(Vec<Value>),
}

impl Value {
    /// 返回该值对应的 `stmt::Type`。
    ///
    /// `Null` 返回 `Type::Unknown`——调用者需要结合列定义判断。
    pub fn ty(&self) -> Type {
        match self {
            Value::Null => Type::Unknown,
            Value::Bool(_) => Type::Bool,
            Value::I8(_) => Type::I8,
            Value::I16(_) => Type::I16,
            Value::I32(_) => Type::I32,
            Value::I64(_) => Type::I64,
            Value::U8(_) => Type::U8,
            Value::U16(_) => Type::U16,
            Value::U32(_) => Type::U32,
            Value::U64(_) => Type::U64,
            Value::F32(_) => Type::F32,
            Value::F64(_) => Type::F64,
            Value::Text(_) => Type::Text,
            Value::Bytes(_) => Type::Bytes,
            Value::Date(_) => Type::Date,
            Value::Time(_) => Type::Time,
            Value::Timestamp(_) => Type::Timestamp,
            Value::TimestampTz(_) => Type::TimestampTz,
            Value::Uuid(_) => Type::Uuid,
            Value::Ulid(_) => Type::Ulid,
            Value::Decimal(_) => Type::Decimal,
            Value::Json(_) => Type::Json,
            Value::Enum(_) => Type::Unknown, // 具体枚举名由列定义决定
            Value::Inet(_) => Type::Inet,
            Value::Cidr(_) => Type::Cidr,
            Value::Array(items) => {
                let elem_ty = items.first().map_or(Type::Unknown, Value::ty);
                Type::Array(Box::new(elem_ty))
            }
        }
    }

    /// 是否为 `Null`。
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// 是否为整型主键的「未赋值」占位值：`Null`，或数值 `0`。
    ///
    /// 自增主键在 INSERT 前处于该状态，写入时排除该列由数据库生成；
    /// 非零主键（雪花 ID、自然键）视为已赋值，照常参与写入。
    pub fn is_unset_pk(&self) -> bool {
        matches!(
            self,
            Value::Null
                | Value::I8(0)
                | Value::I16(0)
                | Value::I32(0)
                | Value::I64(0)
                | Value::U8(0)
                | Value::U16(0)
                | Value::U32(0)
                | Value::U64(0)
        )
    }
}

// ==================== 便捷 From 实现 ====================

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::I32(v)
    }
}
impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::I64(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::F64(v)
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_string())
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

// 生态类型：chrono。`Row::insert` 接受 `impl Into<Value>`，这里为常用
// chrono 类型提供直接转换，便于 Row 模式下写入时间字段。
#[cfg(feature = "chrono")]
mod chrono_from {
    use super::Value;
    use crate::traits::TypeMapper;
    use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Utc};

    impl From<DateTime<Utc>> for Value {
        fn from(v: DateTime<Utc>) -> Self {
            v.to_value()
        }
    }
    impl From<DateTime<FixedOffset>> for Value {
        fn from(v: DateTime<FixedOffset>) -> Self {
            v.to_value()
        }
    }
    impl From<NaiveDate> for Value {
        fn from(v: NaiveDate) -> Self {
            v.to_value()
        }
    }
    impl From<NaiveDateTime> for Value {
        fn from(v: NaiveDateTime) -> Self {
            v.to_value()
        }
    }
    impl From<NaiveTime> for Value {
        fn from(v: NaiveTime) -> Self {
            v.to_value()
        }
    }
}

// 生态类型：uuid。`Params`（`find_by` / `count_by` / `delete_by` 的 WHERE 参数元组）
// 的约束是 `Into<Value>`，而 `find` / `find_or_none` / `delete_by_pk` 的主键参数同样
// 要求 `Into<Value>`。这里为 `Uuid` 及其引用补上直接转换，UUID 主键 / 参数便无需手工
// 经 `TypeMapper` 包装即可传入。同时借用形式（`&Uuid`）一并实现，避免调用方为了满足
// 所有权而 `clone()` 一个 16 字节的标识。
#[cfg(feature = "uuid")]
mod uuid_from {
    use super::Value;
    use crate::traits::TypeMapper;
    use uuid::Uuid;

    impl From<Uuid> for Value {
        fn from(v: Uuid) -> Self {
            v.to_value()
        }
    }
    impl From<&Uuid> for Value {
        fn from(v: &Uuid) -> Self {
            v.to_value()
        }
    }
}

#[cfg(all(test, feature = "uuid"))]
mod uuid_tests {
    use super::*;
    use uuid::Uuid;

    /// `Uuid`（按值 / 借用）与 `Option<Uuid>` 都能直接转成 `Value`，
    /// 使 UUID 主键可直传 `find` / `find_by` / `find_in_ids`。
    #[test]
    fn uuid_converts_into_value() {
        let id = Uuid::parse_str("018f4b2c-1111-7222-8333-444455556666").expect("parse uuid");
        let expected = Value::Uuid(CompactId(*id.as_bytes()));

        assert_eq!(Value::from(id), expected, "按值转换");
        assert_eq!(Value::from(&id), expected, "借用转换");
        assert_eq!(Value::from(Some(id)), expected, "Option::Some");
        assert_eq!(
            Value::from(None::<Uuid>),
            Value::Null,
            "Option::None → NULL"
        );
    }
}

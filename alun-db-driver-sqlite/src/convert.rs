//! `stmt::Value` ↔ `rusqlite::types::Value` 转换。

use alun_db_core::error::{DbError, Result};
use alun_db_core::stmt::Value;
use alun_db_core::stmt::compact::*;
use rusqlite::types::ValueRef;

/// 将 `stmt::Value` 转换为 rusqlite 可绑定的值。
pub fn to_rusqlite(value: &Value) -> Result<rusqlite::types::Value> {
    use rusqlite::types::Value as RValue;
    Ok(match value {
        Value::Null => RValue::Null,
        Value::Bool(b) => RValue::Integer(if *b { 1 } else { 0 }),
        Value::I8(v) => RValue::Integer(*v as i64),
        Value::I16(v) => RValue::Integer(*v as i64),
        Value::I32(v) => RValue::Integer(*v as i64),
        Value::I64(v) => RValue::Integer(*v),
        Value::U8(v) => RValue::Integer(*v as i64),
        Value::U16(v) => RValue::Integer(*v as i64),
        Value::U32(v) => RValue::Integer(*v as i64),
        Value::U64(v) => RValue::Integer(*v as i64),
        Value::F32(v) => RValue::Real(*v as f64),
        Value::F64(v) => RValue::Real(*v),
        Value::Text(s) => RValue::Text(s.clone()),
        Value::Bytes(b) => RValue::Blob(b.clone()),
        Value::Date(d) => RValue::Text(format_date(*d)),
        Value::Time(t) => RValue::Text(format_time(*t)),
        Value::Timestamp(t) => RValue::Text(format_timestamp(*t)),
        Value::TimestampTz(t) => RValue::Text(format_timestamp_tz(*t)),
        Value::Uuid(id) => RValue::Text(format_uuid(id)),
        Value::Ulid(id) => RValue::Text(format_ulid(id)),
        Value::Decimal(d) => RValue::Text(format_decimal(*d)),
        Value::Json(s) => RValue::Text(s.clone()),
        Value::Enum(s) => RValue::Text(s.clone()),
        Value::Inet(a) => RValue::Text(a.to_string()),
        Value::Cidr(c) => RValue::Text(format_cidr(c)),
        // SQLite 无原生数组类型。与 PG/MySQL 一致显式报错，
        // 不得静默降级为 NULL（否则数组写入无声丢值）。
        Value::Array(_) => {
            return Err(DbError::UnsupportedFeature {
                feature: "array binding",
                dialect: "sqlite".into(),
            });
        }
    })
}

/// 将 rusqlite 的 `ValueRef` 转换为 `stmt::Value`。
///
/// ## 声明类型驱动的读回
///
/// SQLite 是动态类型数据库：`BOOLEAN` 列与 `INTEGER` 列、`DECIMAL`/`UUID`
/// 列与 `TEXT` 列在**存储类别**上完全一致。因此读回时除存储类别外还需要
/// 列的声明类型 `decltype`（`sqlite3_column_decltype`，见 [`ColumnDecl`]）：
///
/// - 声明为 `BOOLEAN`/`BOOL` 的列：`0`/`1` 还原为 [`Value::Bool`]
/// - 声明为 `DECIMAL`/`NUMERIC` 的列：十进制文本还原为 [`Value::Decimal`]
/// - 声明为 `UUID` 的列：32/36 位十六进制文本还原为 [`Value::Uuid`]
///
/// 声明类型为 `TEXT`/`INTEGER` 等**无从区分**的类型，或 `decltype` 缺失
/// （表达式列，如 `SUM(x)`）时，一律按存储类别原样返回：宁可由类型化读
/// 报 `TypeMismatch`，也不做「文本能解析就转」的猜测——后者会把普通
/// `String` 列（如内容为 `"0"`）读成 `Value::Decimal`。
///
/// 声明类型识别失败（如 `BOOLEAN` 列里塞了 `2`、`DECIMAL` 列里塞了非法
/// 文本）时同样保留原始存储值，使类型化读报错且原文不丢。
///
/// # 错误
///
/// 文本列非 UTF-8 时返回 [`DbError::InvalidValue`]。
pub fn from_rusqlite(value: ValueRef<'_>, decltype: Option<&str>) -> Result<Value> {
    let Some(kind) = decltype.and_then(ColumnDecl::parse) else {
        return from_storage(value);
    };
    match (kind, value) {
        (ColumnDecl::Boolean, ValueRef::Integer(0)) => Ok(Value::Bool(false)),
        (ColumnDecl::Boolean, ValueRef::Integer(1)) => Ok(Value::Bool(true)),
        (ColumnDecl::Decimal, ValueRef::Text(s)) => {
            let text = utf8_text(s)?;
            Ok(match parse_decimal_text(text) {
                Some(d) => Value::Decimal(d),
                // 非法十进制文本：保留原文，类型化读报 TypeMismatch
                None => Value::Text(text.to_string()),
            })
        }
        // `NUMERIC` 亲和性会把整数字面量存为 INTEGER（如 `"100"`）
        (ColumnDecl::Decimal, ValueRef::Integer(i)) => Ok(Value::Decimal(CompactDecimal {
            mantissa: i as i128,
            scale: 0,
        })),
        (ColumnDecl::Uuid, ValueRef::Text(s)) => {
            let text = utf8_text(s)?;
            Ok(match parse_uuid_text(text) {
                Some(id) => Value::Uuid(id),
                // 非法 UUID 文本：保留原文，类型化读报 TypeMismatch
                None => Value::Text(text.to_string()),
            })
        }
        // 其余组合：声明类型没有给出信号（NULL / 不匹配的存储类别）
        (_, other) => from_storage(other),
    }
}

/// 按存储类别转换（无语义猜测）。
fn from_storage(value: ValueRef<'_>) -> Result<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::I64(i),
        ValueRef::Real(f) => Value::F64(f),
        ValueRef::Text(s) => Value::Text(utf8_text(s)?.to_string()),
        ValueRef::Blob(b) => Value::Bytes(b.to_vec()),
    })
}

fn utf8_text(s: &[u8]) -> Result<&str> {
    std::str::from_utf8(s).map_err(|e| DbError::InvalidValue(format!("invalid UTF-8: {e}")))
}

// ==================== 声明类型 ====================

/// 声明类型中有明确语义的类型族。
///
/// 只列出 SQLite 动态类型下**必须**依赖声明类型才能判定的类型；`TEXT` /
/// `INTEGER` / `REAL` / `BLOB` 等无从区分（与普通文本/整数/浮点/二进制列
/// 同形），故作声明类型处理时视为无信号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnDecl {
    /// `BOOLEAN` / `BOOL`
    Boolean,
    /// `DECIMAL` / `NUMERIC`（含 `DECIMAL(p, s)` 形式）
    Decimal,
    /// `UUID`
    Uuid,
}

impl ColumnDecl {
    /// 解析声明类型（SQLite 类型名大小写不敏感，参数部分忽略）。
    fn parse(decltype: &str) -> Option<ColumnDecl> {
        let base = decltype.split('(').next().unwrap_or(decltype).trim();
        match base.to_ascii_uppercase().as_str() {
            "BOOLEAN" | "BOOL" => Some(ColumnDecl::Boolean),
            "DECIMAL" | "NUMERIC" => Some(ColumnDecl::Decimal),
            "UUID" => Some(ColumnDecl::Uuid),
            _ => None,
        }
    }
}

/// 解析十进制文本为 [`CompactDecimal`]。
///
/// 接受 `[+-]digits[.digits][eE[+-]digits]`。尾数溢出 `i128`、标度超出
/// [`CompactDecimal::MAX_SCALE`]、或含非数字字符时返回 `None`，由调用方
/// 保留原始文本（不得截断、不得近似）。
fn parse_decimal_text(text: &str) -> Option<CompactDecimal> {
    let s = text.trim();
    let (negative, rest) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    // 指数部分：`1.23e-2` == 标度 +2
    let (mantissa_part, exponent) = match rest.find(['e', 'E']) {
        Some(i) => (&rest[..i], rest[i + 1..].parse::<i32>().ok()?),
        None => (rest, 0),
    };
    let (int_part, frac_part) = match mantissa_part.split_once('.') {
        Some((i, f)) => (i, f),
        None => (mantissa_part, ""),
    };
    if (int_part.is_empty() && frac_part.is_empty())
        || !int_part.bytes().all(|b| b.is_ascii_digit())
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }

    let mut mantissa: i128 = 0;
    for b in int_part.bytes().chain(frac_part.bytes()) {
        mantissa = mantissa.checked_mul(10)?.checked_add((b - b'0') as i128)?;
    }

    let scale = frac_part.len() as i32 - exponent;
    let (mantissa, scale) = if scale < 0 {
        // 负标度：尾数补零后标度归零
        let zeros = u32::try_from(-scale).ok()?;
        if zeros > CompactDecimal::MAX_SCALE {
            return None;
        }
        (mantissa.checked_mul(10i128.checked_pow(zeros)?)?, 0)
    } else {
        (mantissa, u32::try_from(scale).ok()?)
    };
    if scale > CompactDecimal::MAX_SCALE {
        return None;
    }

    Some(CompactDecimal {
        mantissa: if negative { -mantissa } else { mantissa },
        scale,
    })
}

/// 解析 UUID 文本为 [`CompactId`]。
///
/// 仅接受两种规范形式：32 位十六进制，或 36 位含连字符的 `8-4-4-4-12`
/// 分组形式（连字符位置必须正确）。其余形式（含 `urn:` / 花括号包裹）
/// 返回 `None`。
fn parse_uuid_text(text: &str) -> Option<CompactId> {
    let s = text.trim();
    let mut hex = [0u8; 32];
    match s.len() {
        32 => hex.copy_from_slice(s.as_bytes()),
        36 => {
            let bytes = s.as_bytes();
            if bytes[8] != b'-' || bytes[13] != b'-' || bytes[18] != b'-' || bytes[23] != b'-' {
                return None;
            }
            let mut i = 0;
            for (idx, b) in bytes.iter().enumerate() {
                if matches!(idx, 8 | 13 | 18 | 23) {
                    continue;
                }
                hex[i] = *b;
                i += 1;
            }
        }
        _ => return None,
    }

    let mut out = [0u8; 16];
    for (i, pair) in hex.chunks_exact(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out[i] = ((hi << 4) | lo) as u8;
    }
    Some(CompactId(out))
}

// ==================== 格式化辅助 ====================

fn format_date(d: CompactDate) -> String {
    // CompactDate 是自 epoch 起的天数
    let epoch_days = d.0 as i64;
    // 使用 chrono 格式化，但核心层不依赖 chrono
    // 因此这里用简单的日期算法
    format_epoch_days(epoch_days)
}

fn format_time(t: CompactTime) -> String {
    let total_secs = t.0 / 1_000_000_000;
    let nanos = t.0 % 1_000_000_000;
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if nanos == 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}.{nanos:09}")
    }
}

fn format_timestamp(t: CompactTimestamp) -> String {
    // 微秒可能为负（1970 之前）。必须按**有符号**拆解：`/` 与 `%` 是截断
    // 除法，直接用于负值会得到 `00:00:-1.000000` 这类非法定宽文本。
    // 这里用 `div_euclid` / `rem_euclid` 取得「向下取整的秒 + 非负余数」，
    // 使 `-1_000_000` 微秒 → `1969-12-31 23:59:59.000000`。
    let micros = t.0;
    let secs = micros.div_euclid(1_000_000);
    let sub_micros = micros.rem_euclid(1_000_000);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    format!(
        "{} {:02}:{:02}:{:02}.{:06}",
        format_epoch_days(days),
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60,
        sub_micros
    )
}

fn format_timestamp_tz(t: CompactTimestampTz) -> String {
    format_timestamp(CompactTimestamp(t.utc_micros))
}

fn format_uuid(id: &CompactId) -> String {
    let b = &id.0;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

fn format_ulid(id: &CompactId) -> String {
    // ULID 使用 Crockford Base32 编码
    // 简化实现：使用 base32 编码 16 字节
    let b = &id.0;
    let mut result = String::with_capacity(26);
    let alphabet = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    // 16 字节 = 128 位，需要 26 个 base32 字符
    let mut bits: u128 = 0;
    for &byte in b.iter() {
        bits = (bits << 8) | byte as u128;
    }
    // 取高 130 位中的 130 位... 实际 ULID 是 128 位编码为 26 字符
    // 这里简化处理
    for i in (0..26).rev() {
        let idx = ((bits >> (i * 5)) & 0x1F) as usize;
        result.push(alphabet[idx] as char);
    }
    result
}

fn format_decimal(d: CompactDecimal) -> String {
    let s = d.mantissa.abs().to_string();
    let sign = if d.mantissa < 0 { "-" } else { "" };
    if d.scale == 0 {
        format!("{sign}{s}")
    } else {
        let s = if s.len() <= d.scale as usize {
            format!("0.{}{}", "0".repeat(d.scale as usize - s.len()), s)
        } else {
            let (int_part, frac_part) = s.split_at(s.len() - d.scale as usize);
            format!("{int_part}.{frac_part}")
        };
        format!("{sign}{s}")
    }
}

fn format_cidr(c: &CompactCidr) -> String {
    format!("{}/{}", c.addr, c.prefix_len)
}

/// 将 epoch 天数格式化为 `YYYY-MM-DD`。
fn format_epoch_days(days: i64) -> String {
    // 使用简单的公历算法
    // 参考 Howard Hinnant 的 civil_from_days 算法
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> ValueRef<'_> {
        ValueRef::Text(s.as_bytes())
    }

    // ==================== D009：数组绑定 ====================

    #[test]
    fn array_binding_is_rejected_not_nulled() {
        let array = Value::Array(vec![Value::I64(1), Value::I64(2)]);
        match to_rusqlite(&array) {
            Err(DbError::UnsupportedFeature { feature, dialect }) => {
                assert_eq!(feature, "array binding");
                assert_eq!(dialect, "sqlite");
            }
            other => panic!("数组绑定应显式报 UnsupportedFeature，实际 {other:?}"),
        }
    }

    #[test]
    fn scalar_binding_still_works() {
        use rusqlite::types::Value as RValue;
        assert_eq!(to_rusqlite(&Value::I64(7)).unwrap(), RValue::Integer(7));
        assert_eq!(
            to_rusqlite(&Value::Text("7".into())).unwrap(),
            RValue::Text("7".into())
        );
        // 十进制仍以 TEXT 落库（改 REAL 会丢精度）
        assert_eq!(
            to_rusqlite(&Value::Decimal(CompactDecimal {
                mantissa: 100,
                scale: 2
            }))
            .unwrap(),
            RValue::Text("1.00".into())
        );
    }

    // ==================== D018：1970 以前的时间戳 ====================

    #[test]
    fn negative_timestamp_formats_by_signed_shift() {
        // 1969-12-31T23:59:59Z == -1_000_000 微秒
        assert_eq!(
            format_timestamp(CompactTimestamp(-1_000_000)),
            "1969-12-31 23:59:59.000000"
        );
        // 1969-12-31T23:59:59.999999Z == -1 微秒
        assert_eq!(
            format_timestamp(CompactTimestamp(-1)),
            "1969-12-31 23:59:59.999999"
        );
        // 1969-12-31T23:59:59.999998Z == -2 微秒
        assert_eq!(
            format_timestamp(CompactTimestamp(-2)),
            "1969-12-31 23:59:59.999998"
        );
        // 负值也必须与正值的定宽格式严格可比（字典序 == 时间序）
        assert_eq!(
            format_timestamp(CompactTimestamp(0)),
            "1970-01-01 00:00:00.000000"
        );
        assert!(
            format_timestamp(CompactTimestamp(-1)) < format_timestamp(CompactTimestamp(0)),
            "1970 前的时间戳文本必须仍小于 epoch"
        );
        assert_eq!(
            format_timestamp(CompactTimestamp(-1_000_000_000)),
            "1969-12-31 23:43:20.000000"
        );
    }

    // ==================== D021：布尔列 ====================

    #[test]
    fn boolean_decltype_restores_bool() {
        assert_eq!(
            from_rusqlite(ValueRef::Integer(0), Some("BOOLEAN")).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            from_rusqlite(ValueRef::Integer(1), Some("BOOL")).unwrap(),
            Value::Bool(true)
        );
        // 大小写不敏感
        assert_eq!(
            from_rusqlite(ValueRef::Integer(1), Some("boolean")).unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            from_rusqlite(ValueRef::Null, Some("BOOLEAN")).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn integer_decltype_stays_i64() {
        // 无 `BOOLEAN` 声明的整数列保持 I64：`false` 与 `0` 的区分只能靠声明类型
        assert_eq!(
            from_rusqlite(ValueRef::Integer(0), Some("INTEGER")).unwrap(),
            Value::I64(0)
        );
        assert_eq!(
            from_rusqlite(ValueRef::Integer(0), None).unwrap(),
            Value::I64(0)
        );
        // 声明类型无从判定（表达式列，如 SUM(x)）时同样保持存储类别
        assert_eq!(
            from_rusqlite(ValueRef::Real(0.0), None).unwrap(),
            Value::F64(0.0)
        );
        assert_eq!(
            from_rusqlite(text("0"), None).unwrap(),
            Value::Text("0".into()),
            "普通文本列（含 \"0\"）不得被猜成其它类型"
        );
    }

    #[test]
    fn boolean_decltype_does_not_invent_bool() {
        // `BOOLEAN` 列里的非 0/1 整数不得被静默当成 true
        assert_eq!(
            from_rusqlite(ValueRef::Integer(7), Some("BOOLEAN")).unwrap(),
            Value::I64(7)
        );
        assert_eq!(
            from_rusqlite(text("yes"), Some("BOOLEAN")).unwrap(),
            Value::Text("yes".into())
        );
    }

    // ==================== D001：DECIMAL / NUMERIC 列 ====================

    #[test]
    fn decimal_decltype_parses_text() {
        assert_eq!(
            from_rusqlite(text("1.00"), Some("DECIMAL(38, 10)")).unwrap(),
            Value::Decimal(CompactDecimal {
                mantissa: 100,
                scale: 2
            })
        );
        assert_eq!(
            from_rusqlite(text("-0.01"), Some("DECIMAL")).unwrap(),
            Value::Decimal(CompactDecimal {
                mantissa: -1,
                scale: 2
            })
        );
        assert_eq!(
            from_rusqlite(text("  12345  "), Some("NUMERIC")).unwrap(),
            Value::Decimal(CompactDecimal {
                mantissa: 12345,
                scale: 0
            })
        );
        assert_eq!(
            from_rusqlite(text("1.5e2"), Some("NUMERIC")).unwrap(),
            Value::Decimal(CompactDecimal {
                mantissa: 150,
                scale: 0
            })
        );
        // NUMERIC 亲和性把整数字面量存为 INTEGER
        assert_eq!(
            from_rusqlite(ValueRef::Integer(100), Some("NUMERIC")).unwrap(),
            Value::Decimal(CompactDecimal {
                mantissa: 100,
                scale: 0
            })
        );
        assert_eq!(
            from_rusqlite(ValueRef::Null, Some("DECIMAL")).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn decimal_decltype_keeps_invalid_text() {
        // 38 位字面量在 `CompactDecimal`（尾数 i128）内可**精确**表示，
        // 驱动层不得截断；类型化读取 `rust_decimal::Decimal` 时才因其
        // 96 位尾数上限报 `InvalidValue`
        let wide = "12345678901234567890123456789012345678";
        assert_eq!(
            from_rusqlite(text(wide), Some("DECIMAL(38, 10)")).unwrap(),
            Value::Decimal(CompactDecimal {
                mantissa: 12345678901234567890123456789012345678,
                scale: 0,
            })
        );
        // 超出 i128 尾数容量（40 位）→ 保留原文，类型化读显式报错且原文不丢
        let overflow = "1234567890123456789012345678901234567890";
        assert_eq!(
            from_rusqlite(text(overflow), Some("DECIMAL")).unwrap(),
            Value::Text(overflow.into())
        );
        // 标度超出 `CompactDecimal::MAX_SCALE`
        let deep = format!("0.{}", "1".repeat(39));
        assert_eq!(
            from_rusqlite(text(&deep), Some("NUMERIC")).unwrap(),
            Value::Text(deep.clone())
        );
        assert_eq!(
            from_rusqlite(text("12a.4"), Some("DECIMAL")).unwrap(),
            Value::Text("12a.4".into())
        );
        assert_eq!(
            from_rusqlite(text(""), Some("DECIMAL")).unwrap(),
            Value::Text(String::new())
        );
    }

    // ==================== D010：UUID 列 ====================

    #[test]
    fn uuid_decltype_parses_text() {
        let bytes = [
            0x01, 0x8f, 0x4b, 0x2c, 0x11, 0x11, 0x72, 0x22, 0x83, 0x33, 0x44, 0x44, 0x55, 0x55,
            0x66, 0x66,
        ];
        let hyphenated = "018f4b2c-1111-7222-8333-444455556666";
        let simple = "018f4b2c111172228333444455556666";
        assert_eq!(
            from_rusqlite(text(hyphenated), Some("UUID")).unwrap(),
            Value::Uuid(CompactId(bytes))
        );
        assert_eq!(
            from_rusqlite(text(simple), Some("UUID")).unwrap(),
            Value::Uuid(CompactId(bytes))
        );
        // 大写十六进制同样接受
        assert_eq!(
            from_rusqlite(text("018F4B2C-1111-7222-8333-444455556666"), Some("UUID")).unwrap(),
            Value::Uuid(CompactId(bytes))
        );
        assert_eq!(
            from_rusqlite(ValueRef::Null, Some("UUID")).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn uuid_decltype_keeps_invalid_text() {
        for bad in [
            "not-a-uuid",
            // 连字符位置错误
            "018f4b2c-11117-222-8333-444455556666",
            // urn 形式
            "urn:uuid:018f4b2c-1111-7222-8333-444455556666",
            // 长度不符
            "018f4b2c1111",
        ] {
            assert_eq!(
                from_rusqlite(text(bad), Some("UUID")).unwrap(),
                Value::Text(bad.into()),
                "非法 UUID 文本 `{bad}` 应保留原文"
            );
        }
    }

    // ==================== 无声明类型 = 不做猜测 ====================

    #[test]
    fn plain_text_column_is_never_coerced() {
        // 关键回归：`TEXT` 声明（或缺失声明）的列一律按原文返回，
        // 否则普通 String 列会被读成 Decimal/Uuid
        let uuid_text = "018f4b2c-1111-7222-8333-444455556666";
        assert_eq!(
            from_rusqlite(text(uuid_text), Some("TEXT")).unwrap(),
            Value::Text(uuid_text.into())
        );
        assert_eq!(
            from_rusqlite(text("0.01"), Some("TEXT")).unwrap(),
            Value::Text("0.01".into())
        );
        assert_eq!(
            from_rusqlite(text("0"), Some("TEXT")).unwrap(),
            Value::Text("0".into())
        );
    }

    /// 声明类型为 `TEXT` 的金额列读回文本：类型化读的十进制容错在
    /// `TypeMapper` 边界完成（`DateTime<Utc>` 的 Text 容错先例），
    /// 驱动层不越界猜测。
    #[test]
    fn money_text_column_roundtrip_is_lossless() {
        let written = format_decimal(CompactDecimal {
            mantissa: -1,
            scale: 2,
        });
        assert_eq!(written, "-0.01");
        assert_eq!(
            from_rusqlite(text(&written), Some("TEXT")).unwrap(),
            Value::Text("-0.01".into())
        );
    }

    #[test]
    fn text_is_utf8_checked() {
        let invalid = ValueRef::Text(&[0xff, 0xfe]);
        assert!(matches!(
            from_rusqlite(invalid, None),
            Err(DbError::InvalidValue(_))
        ));
        assert!(matches!(
            from_rusqlite(invalid, Some("UUID")),
            Err(DbError::InvalidValue(_))
        ));
    }
}

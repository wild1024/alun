//! `stmt::Value` 与 MySQL 二进制协议值的互转。
//!
//! ## 设计
//!
//! mysql_async 的 `Value`（NULL / Bytes / Int / UInt / Float / Double /
//! Date / Time）粒度较粗：文本、二进制、DECIMAL、JSON、枚举均以
//! `Bytes` 承载。读取方向借助**列元数据**（`ColumnType` + `ColumnFlags`）
//! 区分 `Bytes` 载荷的语义。
//!
//! ## 读回统一宽度
//!
//! 整数列统一读回 `Value::I64`（超 i64 的无符号列读回 `U64`），浮点列
//! 统一读回 `Value::F64`，与 SQLite / PostgreSQL 驱动一致——窄类型模型
//! 经 `TypeMapper` 宽容匹配兼容。
//!
//! 例外是**布尔承载**：`TINYINT(1)`（MySQL 无原生 `BOOLEAN`，`BOOL` /
//! `BOOLEAN` 均落为此列型）且取值为 `0` / `1` 时读回 [`Value::Bool`]，
//! 否则 `false` 与 `0` 读回后不可区分（D021）。其余宽度（`TINYINT(4)`、
//! 裸 `TINYINT`）与其余取值仍回 `Value::I64`。
//!
//! ## 标识类型承载（Uuid / Ulid）
//!
//! 方言 DDL 契约（`MysqlDialect::render_type`）为 `Uuid → CHAR(36)`、
//! `Ulid → CHAR(26)`，故绑定按**文本**承载：36 位小写连字符 UUID、
//! 26 位 Crockford Base32 ULID。绑 16 字节原始载荷进 `CHAR(36)` 列会因
//! 载荷非法 UTF-8 被服务端拒绝（`Incorrect string value`，D010）。
//!
//! `BINARY(16)` / `VARBINARY` 列：MySQL 二进制协议把所有字符串参数一律
//! 声明为 `MYSQL_TYPE_VAR_STRING`（按连接字符集解释），且
//! `COM_STMT_PREPARE` **不返回参数的目标列类型**——驱动在绑定期无从判定
//! 目标列家族（实测：任意参数一律上报 `MYSQL_TYPE_VAR_STRING` /
//! `MYSQL_TYPE_LONGLONG`）。这类列请显式绑 [`Value::Bytes`]（16 字节）。
//!
//! 读回方向与 SQLite 驱动同构：文本承载的列读回 [`Value::Text`]，由类型化
//! 读边界（`TypeMapper` 的 `Uuid::from_value` 文本容错）解析标识类型；
//! `BINARY(16)` / `VARBINARY(16)` 列在载荷恰为 16 字节时读回
//! `Value::Uuid`。
//!
//! ## 已知有损点
//!
//! - `TimestampTz`：MySQL `DATETIME` 不保留时区偏移，写入按 UTC、读回
//!   `offset_secs = 0`
//! - `Time`：MySQL `TIME` 支持负值与超过 24 小时，`CompactTime` 不支持，
//!   读回时报错
//! - 数组：绑定与读取均不支持（v1）
//! - 二进制列只在**列元数据给出明确信号**时还原标识类型：定宽 16 字节列
//!   （`BINARY(16)` / `VARBINARY(16)`）且载荷恰为 16 字节；其余二进制
//!   （`BLOB`、其它宽度）保持 [`Value::Bytes`] 原样——宁可由类型化读报
//!   `TypeMismatch`，也不做「长度够就当标识」的猜测。

use alun_db_core::error::DbError;
use alun_db_core::row::Row;
use alun_db_core::stmt::Value;
use alun_db_core::stmt::compact::{
    CompactDate, CompactDecimal, CompactId, CompactTime, CompactTimestampTz,
};
use mysql_async::consts::{ColumnFlags, ColumnType};

// ==================== 绑定方向：Value -> mysql_async::Value ====================

/// 将 `stmt::Value` 列表转为 MySQL 绑定参数。
///
/// # 错误
///
/// 值越界（时间戳超出公历范围）或数组绑定时返回 [`DbError`]。
pub fn to_mysql_params(params: &[Value]) -> alun_db_core::error::Result<Vec<mysql_async::Value>> {
    params.iter().map(to_mysql_param).collect()
}

/// 单个 `Value` → `mysql_async::Value`。
fn to_mysql_param(value: &Value) -> alun_db_core::error::Result<mysql_async::Value> {
    match value {
        Value::Null => Ok(mysql_async::Value::NULL),
        // MySQL 无原生 BOOLEAN，TINYINT(1) 承载
        Value::Bool(b) => Ok(mysql_async::Value::Int(i64::from(*b))),
        Value::I8(n) => Ok(mysql_async::Value::Int(i64::from(*n))),
        Value::I16(n) => Ok(mysql_async::Value::Int(i64::from(*n))),
        Value::I32(n) => Ok(mysql_async::Value::Int(i64::from(*n))),
        Value::I64(n) => Ok(mysql_async::Value::Int(*n)),
        Value::U8(n) => Ok(mysql_async::Value::Int(i64::from(*n))),
        Value::U16(n) => Ok(mysql_async::Value::Int(i64::from(*n))),
        Value::U32(n) => Ok(mysql_async::Value::Int(i64::from(*n))),
        Value::U64(n) => Ok(mysql_async::Value::UInt(*n)),
        Value::F32(f) => Ok(mysql_async::Value::Float(*f)),
        Value::F64(f) => Ok(mysql_async::Value::Double(*f)),
        // 枚举标签与文本同构（MySQL ENUM 以字符串收发）
        Value::Text(s) | Value::Enum(s) => Ok(mysql_async::Value::Bytes(s.clone().into_bytes())),
        Value::Bytes(b) => Ok(mysql_async::Value::Bytes(b.clone())),
        Value::Date(d) => {
            let (y, m, day) = civil_from_days(d.0);
            Ok(mysql_async::Value::Date(y as u16, m, day, 0, 0, 0, 0))
        }
        Value::Time(t) => {
            if !t.is_valid() {
                return Err(DbError::InvalidValue("time exceeds one day".into()));
            }
            let (h, m, s, us) = split_nanos(t.0);
            Ok(mysql_async::Value::Time(false, 0, h, m, s, us))
        }
        Value::Timestamp(ts) => {
            let (y, mo, d, h, mi, s, us) = split_epoch_micros(ts.0)?;
            Ok(mysql_async::Value::Date(y, mo, d, h, mi, s, us))
        }
        // MySQL 不保留时区偏移：统一按 UTC 写入（模块文档「已知有损点」）
        Value::TimestampTz(ts) => {
            if !ts.is_valid() {
                return Err(DbError::InvalidValue(
                    "timestamp offset out of range".into(),
                ));
            }
            let (y, mo, d, h, mi, s, us) = split_epoch_micros(ts.utc_micros)?;
            Ok(mysql_async::Value::Date(y, mo, d, h, mi, s, us))
        }
        // 标识类型按方言 DDL 契约绑文本承载：Uuid → CHAR(36)、Ulid → CHAR(26)。
        // 16 字节原始载荷会被服务端按连接字符集校验，UUID 字节非法 UTF-8
        // → `Incorrect string value`；BINARY(16) 列请显式绑 `Value::Bytes`
        // （见模块文档「标识类型承载」）
        Value::Uuid(id) => Ok(mysql_async::Value::Bytes(format_uuid(id).into_bytes())),
        Value::Ulid(id) => Ok(mysql_async::Value::Bytes(format_ulid(id).into_bytes())),
        Value::Decimal(dec) => {
            if !dec.is_valid() {
                return Err(DbError::InvalidValue("decimal scale exceeds 38".into()));
            }
            // MySQL DECIMAL 以字符串收发
            Ok(mysql_async::Value::Bytes(format_decimal(dec).into_bytes()))
        }
        // JSON 列接受文本载荷
        Value::Json(payload) => Ok(mysql_async::Value::Bytes(payload.clone().into_bytes())),
        // MySQL 无原生网络地址类型，按文本承载（VARCHAR(45)）
        Value::Inet(addr) => Ok(mysql_async::Value::Bytes(addr.to_string().into_bytes())),
        Value::Cidr(cidr) => {
            if !cidr.is_valid() {
                return Err(DbError::InvalidValue("cidr prefix out of range".into()));
            }
            Ok(mysql_async::Value::Bytes(
                format!("{}/{}", cidr.addr, cidr.prefix_len).into_bytes(),
            ))
        }
        Value::Array(_) => Err(DbError::UnsupportedFeature {
            feature: "array binding",
            dialect: "mysql".into(),
        }),
    }
}

// ==================== 读取方向：mysql_async::Value -> Value ====================

/// 将 mysql_async 行转换为 `Row`。
///
/// 值语义由列元数据（类型 + flags）与载荷共同决定。
///
/// # 错误
///
/// 载荷解码失败（非法 UTF-8、DECIMAL 越界、TIME 超范围）时返回
/// [`DbError`]。
pub fn mysql_row_to_row(mysql_row: &mysql_async::Row) -> alun_db_core::error::Result<Row> {
    let columns = mysql_row.columns_ref();
    let mut row = Row::with_capacity(columns.len());
    for (i, column) in columns.iter().enumerate() {
        // 恒等 FromValue：列存在且未 taken 时为 Some
        let value = mysql_row
            .get::<mysql_async::Value, usize>(i)
            .unwrap_or(mysql_async::Value::NULL);
        let converted = mysql_value_to_value(value, column)?;
        row.insert(&*column.name_str(), converted);
    }
    Ok(row)
}

/// 按载荷形态 + 列元数据转换为 `stmt::Value`。
fn mysql_value_to_value(
    value: mysql_async::Value,
    column: &mysql_async::Column,
) -> alun_db_core::error::Result<Value> {
    match value {
        mysql_async::Value::NULL => Ok(Value::Null),
        mysql_async::Value::Int(n) => Ok(int_to_value(n, column)),
        mysql_async::Value::UInt(n) => match i64::try_from(n) {
            Ok(signed) => Ok(int_to_value(signed, column)),
            Err(_) => Ok(Value::U64(n)),
        },
        mysql_async::Value::Float(f) => Ok(Value::F64(f64::from(f))),
        mysql_async::Value::Double(f) => Ok(Value::F64(f)),
        mysql_async::Value::Bytes(b) => bytes_to_value(b, column),
        mysql_async::Value::Date(y, mo, d, h, mi, s, us) => {
            date_to_value(y, mo, d, h, mi, s, us, column)
        }
        mysql_async::Value::Time(is_negative, days, h, mi, s, us) => {
            time_to_value(is_negative, days, h, mi, s, us)
        }
    }
}

/// 整数载荷 → `stmt::Value`。
///
/// 布尔承载列（`TINYINT(1)`）的 `0` / `1` 读回 [`Value::Bool`]，其余一律
/// 读回 [`Value::I64`]——`false` 与 `0` 在布尔列上必须可区分（D021）。
fn int_to_value(n: i64, column: &mysql_async::Column) -> Value {
    match (is_bool_column(column), n) {
        (true, 0) => Value::Bool(false),
        (true, 1) => Value::Bool(true),
        _ => Value::I64(n),
    }
}

/// 列是否为布尔承载。
///
/// MySQL 的 `BOOL` / `BOOLEAN` 是 `TINYINT(1)` 的同义词，协议中均为
/// `MYSQL_TYPE_TINY` 且显示宽度为 `1`；`TINYINT` / `TINYINT(4)` 的显示
/// 宽度为 `4`，据此区分（不做「窄整数即布尔」的猜测）。
fn is_bool_column(column: &mysql_async::Column) -> bool {
    column.column_type() == ColumnType::MYSQL_TYPE_TINY && column.column_length() == 1
}

/// `Bytes` 载荷按列类型分流。
///
/// 文本 / 二进制 / DECIMAL / JSON / 枚举在协议中均为字节串。标识类型
/// （`Uuid`）仅在列元数据给出明确信号时还原：方言 DDL 用 `CHAR(36)` 承载
/// `Uuid`，文本形态由类型化读边界（`TypeMapper`）解析，故这里保持
/// [`Value::Text`]；`BINARY(16)` / `VARBINARY(16)` 的 16 字节载荷还原为
/// `Value::Uuid`。
fn bytes_to_value(
    bytes: Vec<u8>,
    column: &mysql_async::Column,
) -> alun_db_core::error::Result<Value> {
    let ty = column.column_type();
    let flags = column.flags();
    match ty {
        // DECIMAL / NEWDECIMAL 以十进制字符串收发
        ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL => {
            let text = read_utf8(&bytes)?;
            let dec = parse_decimal_str(&text)?;
            Ok(Value::Decimal(dec))
        }
        ColumnType::MYSQL_TYPE_JSON => Ok(Value::Json(read_utf8(&bytes)?)),
        // 枚举标签：ENUM 列读回标签文本
        _ if flags.contains(ColumnFlags::ENUM_FLAG) => Ok(Value::Enum(read_utf8(&bytes)?)),
        // 二进制家族（BINARY_FLAG）：定宽 16 字节列承载 Uuid；
        // 其余二进制载荷（BLOB / VARBINARY(n) 等）原样返回 `Bytes`
        _ if flags.contains(ColumnFlags::BINARY_FLAG) => {
            match <[u8; 16]>::try_from(bytes.as_slice()) {
                Ok(raw) if column.column_length() == 16 => Ok(Value::Uuid(CompactId(raw))),
                _ => Ok(Value::Bytes(bytes)),
            }
        }
        // TEXT 家族（含 `CHAR(36)`：Uuid 的文本承载）
        _ => Ok(Value::Text(read_utf8(&bytes)?)),
    }
}

/// `Date` 载荷按列类型分流：DATE 列 → 日期，DATETIME / TIMESTAMP 列 → 时间戳。
#[allow(clippy::too_many_arguments)]
fn date_to_value(
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    micros: u32,
    column: &mysql_async::Column,
) -> alun_db_core::error::Result<Value> {
    match column.column_type() {
        ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE => Ok(Value::Date(
            CompactDate(days_from_civil(i32::from(year), month, day)),
        )),
        _ => {
            // DATETIME / TIMESTAMP 列不保留时区：按写入协议视为 UTC
            // 语义（offset = 0），与模块文档「已知有损点」一致
            let utc_micros =
                combine_epoch_micros(i32::from(year), month, day, hour, minute, second, micros)?;
            Ok(Value::TimestampTz(CompactTimestampTz {
                utc_micros,
                offset_secs: 0,
            }))
        }
    }
}

/// `Time` 载荷 → `CompactTime`。
///
/// 负值与超过 24 小时（days > 0）的 TIME 无法表达，报错。
fn time_to_value(
    is_negative: bool,
    days: u32,
    hour: u8,
    minute: u8,
    second: u8,
    micros: u32,
) -> alun_db_core::error::Result<Value> {
    if is_negative || days > 0 {
        return Err(DbError::InvalidValue(
            "negative or >24h TIME value not representable".into(),
        ));
    }
    let nanos = u64::from(hour) * 3_600_000_000_000
        + u64::from(minute) * 60_000_000_000
        + u64::from(second) * 1_000_000_000
        + u64::from(micros) * 1_000;
    Ok(Value::Time(CompactTime(nanos)))
}

// ==================== 标识类型文本编码（Uuid / Ulid） ====================

/// ULID 的 Crockford Base32 字母表（规范形式，排除易混淆的 `I`/`L`/`O`/`U`）。
const ULID_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// [`CompactId`] → 36 位小写连字符 UUID 文本。
///
/// 与 `uuid::Uuid` 的 `Display` 形态一致：方言 DDL 以 `CHAR(36)` 承载
/// `Uuid`，落库文本即此形态。
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

/// [`CompactId`] → 26 位 ULID 文本（Crockford Base32，规范大写）。
///
/// 128 位载荷编码为 26 个字符（首位只占 3 位有效载荷），方言 DDL 以
/// `CHAR(26)` 承载 `Ulid`。
fn format_ulid(id: &CompactId) -> String {
    let mut acc = u128::from_be_bytes(id.0);
    let mut buf = [ULID_ALPHABET[0]; 26];
    for slot in buf.iter_mut().rev() {
        *slot = ULID_ALPHABET[(acc & 0x1f) as usize];
        acc >>= 5;
    }
    // 26 个字符均取自字母表，恒为 ASCII
    buf.iter().map(|&b| char::from(b)).collect()
}

// ==================== 公历日期算法（Howard Hinnant） ====================

/// 公历日期 → 自 Unix 纪元（1970-01-01）的天数。
///
/// 经典 civil 算法：以 3 月为岁首消除闰年的不规则性。
fn days_from_civil(y: i32, m: u8, d: u8) -> i32 {
    // 1-2 月视作上一年的 13-14 月
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = i32::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i32::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 自 Unix 纪元的天数 → 公历日期 `(年, 月, 日)`。
fn civil_from_days(z: i32) -> (i32, u8, u8) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ==================== 时间拆解 / 组合辅助 ====================

/// 每天的微秒数。
const MICROS_PER_DAY: i64 = 86_400_000_000;

/// Unix 纪元微秒 → `(年, 月, 日, 时, 分, 秒, 微秒)`。
fn split_epoch_micros(micros: i64) -> alun_db_core::error::Result<(u16, u8, u8, u8, u8, u8, u32)> {
    let days = micros.div_euclid(MICROS_PER_DAY);
    let secs_of_day = micros.rem_euclid(MICROS_PER_DAY);
    // i32 天数覆盖 ±580 万年，远超 MySQL DATETIME(1000-9999) 范围；
    // 越界说明值本身无法被 MySQL 承载
    let days = i32::try_from(days)
        .map_err(|_| DbError::InvalidValue("timestamp out of calendar range".into()))?;
    let (y, m, d) = civil_from_days(days);
    let year = u16::try_from(y)
        .map_err(|_| DbError::InvalidValue("timestamp year out of MySQL range".into()))?;
    let hour = (secs_of_day / 3_600_000_000) as u8;
    let minute = ((secs_of_day % 3_600_000_000) / 60_000_000) as u8;
    let second = ((secs_of_day % 60_000_000) / 1_000_000) as u8;
    let us = (secs_of_day % 1_000_000) as u32;
    Ok((year, m, d, hour, minute, second, us))
}

/// `(年, 月, 日, 时, 分, 秒, 微秒)` → Unix 纪元微秒。
#[allow(clippy::too_many_arguments)]
fn combine_epoch_micros(
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    micros: u32,
) -> alun_db_core::error::Result<i64> {
    let days = i64::from(days_from_civil(year, month, day));
    Ok(days * MICROS_PER_DAY
        + i64::from(hour) * 3_600_000_000
        + i64::from(minute) * 60_000_000
        + i64::from(second) * 1_000_000
        + i64::from(micros))
}

/// 自午夜起的纳秒 → `(时, 分, 秒, 微秒)`。
fn split_nanos(nanos: u64) -> (u8, u8, u8, u32) {
    let hour = (nanos / 3_600_000_000_000) as u8;
    let minute = ((nanos % 3_600_000_000_000) / 60_000_000_000) as u8;
    let second = ((nanos % 60_000_000_000) / 1_000_000_000) as u8;
    let micros = ((nanos % 1_000_000_000) / 1_000) as u32;
    (hour, minute, second, micros)
}

// ==================== DECIMAL 字符串编解码 ====================

/// `CompactDecimal` → 十进制字符串（如 `"-123.45"`）。
///
/// mantissa 按 scale 拆整数 / 小数部分；纯小数左补零对齐。
fn format_decimal(dec: &CompactDecimal) -> String {
    let scale = dec.scale as usize;
    let is_negative = dec.mantissa < 0;
    let digits = dec.mantissa.unsigned_abs().to_string();

    let body = if scale == 0 {
        digits
    } else if digits.len() > scale {
        let split = digits.len() - scale;
        format!("{}.{}", &digits[..split], &digits[split..])
    } else {
        format!("0.{:0>width$}", digits, width = scale)
    };
    if is_negative {
        format!("-{body}")
    } else {
        body
    }
}

/// 解析十进制字符串（如 `"-123.45"`，服务端 DECIMAL 载荷）为 `CompactDecimal`。
///
/// # 错误
///
/// 非法字符、超过 38 位有效数字或 i128 溢出时返回 [`DbError::InvalidValue`]。
fn parse_decimal_str(s: &str) -> alun_db_core::error::Result<CompactDecimal> {
    let invalid = || DbError::InvalidValue(format!("invalid decimal string `{s}`"));
    let negative = s.starts_with('-');
    let body = s.strip_prefix('-').unwrap_or(s);
    let (int_part, frac_part) = body.split_once('.').unwrap_or((body, ""));
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(invalid());
    }
    if !int_part
        .bytes()
        .chain(frac_part.bytes())
        .all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    let combined = format!("{int_part}{frac_part}");
    // CompactDecimal 容量：38 位有效数字
    if combined.len() > 38 {
        return Err(DbError::InvalidValue(format!(
            "`{s}` exceeds 38-digit decimal capacity"
        )));
    }
    let mantissa: i128 = combined
        .parse()
        .map_err(|_| DbError::InvalidValue(format!("`{s}` overflows i128")))?;
    Ok(CompactDecimal {
        mantissa: if negative { -mantissa } else { mantissa },
        scale: frac_part.len() as u32,
    })
}

/// 字节载荷按 UTF-8 解码为文本。
fn read_utf8(bytes: &[u8]) -> alun_db_core::error::Result<String> {
    String::from_utf8(bytes.to_vec())
        .map_err(|e| DbError::InvalidValue(format!("column text is not UTF-8: {e}")))
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    /// 公历日期与 Unix 天数往返。
    #[test]
    fn civil_days_roundtrip() {
        for (y, m, d) in [
            (1970, 1u8, 1u8),
            (2000, 2, 29),
            (2026, 9, 22),
            (1000, 1, 1),
            (9999, 12, 31),
            (1969, 12, 31),
        ] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d), "roundtrip {y}-{m}-{d}");
        }
        // 已知锚点：2026-09-22 距 1970-01-01 的天数
        assert_eq!(days_from_civil(2026, 9, 22), 20718);
    }

    /// DECIMAL 格式化与解析往返。
    #[test]
    fn decimal_str_roundtrip() {
        for (mantissa, scale) in [
            (0i128, 0u32),
            (12345, 2),  // 123.45
            (-6789, 3),  // -6.789
            (5, 2),      // 0.05
            (123456, 6), // 0.123456
            (10000, 2),  // 100.00
            (-1, 0),
        ] {
            let dec = CompactDecimal { mantissa, scale };
            let text = format_decimal(&dec);
            let back = parse_decimal_str(&text).expect("parse");
            assert_eq!(
                (back.mantissa, back.scale),
                (mantissa, scale),
                "roundtrip {text}"
            );
        }
    }

    /// 纯小数的格式化形态（左补零）。
    #[test]
    fn decimal_format_pure_fraction() {
        let dec = CompactDecimal {
            mantissa: 5,
            scale: 2,
        };
        assert_eq!(format_decimal(&dec), "0.05");
        let dec = CompactDecimal {
            mantissa: -5,
            scale: 4,
        };
        assert_eq!(format_decimal(&dec), "-0.0005");
    }

    /// 纳秒拆解。
    #[test]
    fn nanos_split() {
        assert_eq!(split_nanos(0), (0, 0, 0, 0));
        // 23:59:59.999999
        assert_eq!(split_nanos(86_399_999_999_999), (23, 59, 59, 999_999));
    }

    /// Unix 纪元微秒经 `combine_epoch_micros` → `split_epoch_micros` 往返一致。
    ///
    /// 覆盖跨公历边界（1970 起点、闰日、世纪末、1969 负区间）的多种组合。
    #[test]
    fn epoch_micros_roundtrip() {
        for (y, m, d, h, mi, s, us) in [
            (1970i32, 1u8, 1u8, 0u8, 0u8, 0u8, 0u32),
            (2026, 9, 22, 8, 30, 15, 123_456),
            (9999, 12, 31, 23, 59, 59, 999_999),
            (1969, 12, 31, 23, 59, 59, 999_999),
            (2000, 2, 29, 12, 0, 0, 0),
            (1000, 1, 1, 0, 0, 0, 0),
        ] {
            let micros = combine_epoch_micros(y, m, d, h, mi, s, us).expect("combine in range");
            let back = split_epoch_micros(micros).expect("split in range");
            assert_eq!(
                back,
                (y as u16, m, d, h, mi, s, us),
                "roundtrip {y}-{m}-{d} {h}:{mi}:{s}.{us}"
            );
        }
    }

    /// 超公历/超 MySQL 年份范围的微秒值 `split_epoch_micros` 应返回错误。
    #[test]
    fn split_epoch_micros_rejects_overflow() {
        // i64::MAX 对应约 2.9 亿年，远超 u16 年份与 MySQL DATETIME(1000-9999) 范围
        assert!(split_epoch_micros(i64::MAX).is_err());
        assert!(split_epoch_micros(i64::MIN).is_err());
    }

    /// 补充 `split_nanos` 的边界用例（1 时 / 正好 1 天）。
    #[test]
    fn split_nanos_more_cases() {
        // 1 时整
        assert_eq!(split_nanos(3_600_000_000_000), (1, 0, 0, 0));
        // 正好 1 天（24:00:00，超出合法时间范围但可拆解）
        assert_eq!(split_nanos(86_400_000_000_000), (24, 0, 0, 0));
    }

    /// `format_decimal` 的边缘形态：零、整数、纯负小数、38 位小数、大整数。
    #[test]
    fn decimal_format_edge_cases() {
        assert_eq!(
            format_decimal(&CompactDecimal {
                mantissa: 0,
                scale: 0
            }),
            "0"
        );
        assert_eq!(
            format_decimal(&CompactDecimal {
                mantissa: 5,
                scale: 0
            }),
            "5"
        );
        assert_eq!(
            format_decimal(&CompactDecimal {
                mantissa: -12345,
                scale: 2
            }),
            "-123.45"
        );
        // 38 位小数：1 × 10^-38
        assert_eq!(
            format_decimal(&CompactDecimal {
                mantissa: 1,
                scale: 38
            }),
            "0.00000000000000000000000000000000000001"
        );
        assert_eq!(
            format_decimal(&CompactDecimal {
                mantissa: 123_456_789,
                scale: 0
            }),
            "123456789"
        );
    }

    /// 非法十进制字符串解析均应返回 `DbError`。
    #[test]
    fn parse_decimal_str_errors() {
        for bad in [
            "",
            "abc",
            "1.2.3",
            "999999999999999999999999999999999999999",
            "1".repeat(100).as_str(),
        ] {
            assert!(parse_decimal_str(bad).is_err(), "expected Err for `{bad}`");
        }
    }

    /// 十进制字符串解析成 `(mantissa, scale)` 的形态映射。
    #[test]
    fn parse_decimal_str_formats() {
        for (text, mantissa, scale) in [
            ("-6.789", -6789i128, 3u32),
            ("0.05", 5, 2),
            ("100", 100, 0),
            ("0", 0, 0),
        ] {
            let dec = parse_decimal_str(text).expect("valid decimal");
            assert_eq!((dec.mantissa, dec.scale), (mantissa, scale), "parse {text}");
        }
    }

    /// UTF-8 解码：合法字节返回文本，非法字节返回错误。
    #[test]
    fn read_utf8_rejects_invalid() {
        assert_eq!(read_utf8(b"ok").expect("valid utf8"), "ok");
        assert!(read_utf8(&[0xff, 0xfe]).is_err(), "invalid utf8 must err");
        assert!(read_utf8(&[0xe4, 0xbd]).is_err(), "truncated utf8 must err");
    }

    /// `to_mysql_param` 各 `Value` 变体到 MySQL 值的类型映射。
    #[test]
    fn to_mysql_param_type_mapping() {
        // BOOLEAN → TINYINT(1)，即 Int(1)
        let got = to_mysql_param(&Value::Bool(true)).expect("bool");
        assert_eq!(got, mysql_async::Value::Int(1));
        // 有符号整数 → Int
        assert_eq!(
            to_mysql_param(&Value::I8(-5)).expect("i8"),
            mysql_async::Value::Int(-5)
        );
        assert_eq!(
            to_mysql_param(&Value::I64(42)).expect("i64"),
            mysql_async::Value::Int(42)
        );
        // 无符号整数 → UInt（无论 i64 是否可容纳）
        let max = to_mysql_param(&Value::U64(u64::MAX)).expect("u64::MAX");
        assert_eq!(max, mysql_async::Value::UInt(u64::MAX));
        let small = to_mysql_param(&Value::U64(5)).expect("u64(5)");
        assert_eq!(small, mysql_async::Value::UInt(5));
        // F64 → Double
        assert_eq!(
            to_mysql_param(&Value::F64(1.25)).expect("f64"),
            mysql_async::Value::Double(1.25)
        );
        // 文本 → Bytes
        assert_eq!(
            to_mysql_param(&Value::Text("abc".into())).expect("text"),
            mysql_async::Value::Bytes(b"abc".to_vec())
        );
        // 空值 → NULL
        assert_eq!(
            to_mysql_param(&Value::Null).expect("null"),
            mysql_async::Value::NULL
        );
    }

    /// `to_mysql_param` 对非法值（超范围 / 不支持）应返回错误。
    #[test]
    fn to_mysql_param_boundary_errors() {
        // Time 超过一天 → 非法
        let time = Value::Time(CompactTime(CompactTime::NANOS_PER_DAY));
        assert!(to_mysql_param(&time).is_err(), "time >= 1 day must err");

        // TimestampTz 时区偏移越界 → 非法
        let tz = Value::TimestampTz(CompactTimestampTz {
            utc_micros: 0,
            offset_secs: 86_400,
        });
        assert!(to_mysql_param(&tz).is_err(), "offset out of range must err");

        // Cidr 前缀越界 → 非法
        let cidr = Value::Cidr(alun_db_core::stmt::compact::CompactCidr {
            addr: IpAddr::V4("127.0.0.1".parse().expect("valid ipv4")),
            prefix_len: 33,
        });
        assert!(
            to_mysql_param(&cidr).is_err(),
            "prefix out of range must err"
        );

        // 数组绑定时 MySQL 不支持 → UnsupportedFeature
        assert!(matches!(
            to_mysql_param(&Value::Array(vec![Value::I64(1)])),
            Err(alun_db_core::error::DbError::UnsupportedFeature { .. })
        ));

        // DECIMAL 标度超过 38 → 非法
        let dec = Value::Decimal(CompactDecimal {
            mantissa: 1,
            scale: 39,
        });
        assert!(to_mysql_param(&dec).is_err(), "scale > 38 must err");
    }

    /// 公历日期算法的已知数值锚点（含闰日与 epoch 前一天）。
    #[test]
    fn days_from_civil_known_anchors() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        // epoch 前一天
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        // 闰日：1970-01-01 距 2000-02-29 共 11016 天
        assert_eq!(days_from_civil(2000, 2, 29), 11016);
        assert_eq!(civil_from_days(11016), (2000, 2, 29));
    }

    // ==================== 标识类型（D010） ====================

    /// 规范 36 位 UUID 文本。
    const UUID_TEXT: &str = "018f4b2c-1111-7222-8333-444455556666";

    /// 构造十六进制字节。
    fn id_of(hex: &str) -> CompactId {
        let mut out = [0u8; 16];
        for (i, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
            let text = std::str::from_utf8(pair).expect("hex ascii");
            out[i] = u8::from_str_radix(text, 16).expect("hex byte");
        }
        CompactId(out)
    }

    /// [`UUID_TEXT`] 的 [`CompactId`] 形态。
    fn uuid_id() -> CompactId {
        id_of(&UUID_TEXT.replace('-', ""))
    }

    /// 二进制列（`BINARY(16)` / `VARBINARY(16)` / `BLOB`）。
    fn binary_column(column_type: ColumnType, len: u32) -> mysql_async::Column {
        mysql_async::Column::new(column_type)
            .with_name(b"bin")
            .with_character_set(63)
            .with_flags(ColumnFlags::BINARY_FLAG)
            .with_column_length(len)
    }

    /// 绑定：Uuid / Ulid 落为方言 DDL 的文本承载（36 位小写连字符 / 26 位 Crockford）。
    #[test]
    fn id_bind_is_text() {
        assert_eq!(
            to_mysql_param(&Value::Uuid(uuid_id())).expect("uuid bind"),
            mysql_async::Value::Bytes(UUID_TEXT.as_bytes().to_vec())
        );
        // 非规范载荷（大写十六进制 / 32 位紧凑形式）同样落为规范小写连字符文本
        assert_eq!(
            to_mysql_param(&Value::Uuid(id_of("018F4B2C111172228333444455556666"))).expect("uuid"),
            mysql_async::Value::Bytes(UUID_TEXT.as_bytes().to_vec())
        );
        // ULID：128 位载荷 → 26 位 Crockford Base32（首位只占 3 位有效载荷）
        for (hex, text) in [
            (
                "00000000000000000000000000000000",
                "00000000000000000000000000",
            ),
            (
                "01000000000000000000000000000000",
                "01000000000000000000000000",
            ),
            (
                "00000000000000000000000000000010",
                "0000000000000000000000000G",
            ),
            (
                "ffffffffffffffffffffffffffffffff",
                "7ZZZZZZZZZZZZZZZZZZZZZZZZZ",
            ),
        ] {
            assert_eq!(
                to_mysql_param(&Value::Ulid(id_of(hex))).expect("ulid bind"),
                mysql_async::Value::Bytes(text.as_bytes().to_vec()),
                "ULID `{hex}` → `{text}`"
            );
        }
    }

    /// 绑定文本的宽度与方言 DDL 列宽一致（`CHAR(36)` / `CHAR(26)`）。
    #[test]
    fn id_bind_width_fits_ddl() {
        let uuid = to_mysql_param(&Value::Uuid(uuid_id())).expect("uuid");
        let mysql_async::Value::Bytes(bytes) = uuid else {
            panic!("Uuid 必须绑为字节载荷")
        };
        assert_eq!(bytes.len(), 36, "CHAR(36) 列宽");

        let ulid = to_mysql_param(&Value::Ulid(CompactId([0x42; 16]))).expect("ulid");
        let mysql_async::Value::Bytes(bytes) = ulid else {
            panic!("Ulid 必须绑为字节载荷")
        };
        assert_eq!(bytes.len(), 26, "CHAR(26) 列宽");
        assert!(
            bytes.iter().all(|b| ULID_ALPHABET.contains(b)),
            "ULID 文本必须落在 Crockford Base32 字母表内"
        );
    }

    /// 读回：`CHAR(36)` 承载的文本原样保留（类型化读边界解析标识类型）。
    #[test]
    fn char36_text_stays_text() {
        let char36 = mysql_async::Column::new(ColumnType::MYSQL_TYPE_STRING)
            .with_name(b"uid")
            .with_character_set(45)
            .with_column_length(144);
        assert_eq!(
            mysql_value_to_value(
                mysql_async::Value::Bytes(UUID_TEXT.as_bytes().to_vec()),
                &char36
            )
            .expect("char36"),
            Value::Text(UUID_TEXT.to_string()),
            "文本承载不得被驱动层翻译成 Value::Uuid"
        );
    }

    /// 读回：16 字节宽的二进制列还原为 `Value::Uuid`；其余二进制保持字节原样。
    #[test]
    fn binary_columns_decode_uuid() {
        let raw = uuid_id();
        for col in [
            binary_column(ColumnType::MYSQL_TYPE_STRING, 16), // BINARY(16)
            binary_column(ColumnType::MYSQL_TYPE_VAR_STRING, 16), // VARBINARY(16)
        ] {
            assert_eq!(
                mysql_value_to_value(mysql_async::Value::Bytes(raw.0.to_vec()), &col)
                    .expect("decode uuid"),
                Value::Uuid(raw),
                "`{:?}` 的 16 字节载荷应还原为 Uuid",
                col.column_type()
            );
        }

        // 载荷宽度不符：二进制原样返回（不做补零 / 截断猜测）
        let binary16 = binary_column(ColumnType::MYSQL_TYPE_STRING, 16);
        for payload in [vec![], vec![1, 2, 3], vec![0u8; 17]] {
            assert_eq!(
                mysql_value_to_value(mysql_async::Value::Bytes(payload.clone()), &binary16)
                    .expect("bytes"),
                Value::Bytes(payload.clone()),
                "非 16 字节载荷必须保持 Bytes"
            );
        }

        // 其它宽度（BLOB 家族）的 16 字节载荷不得被误判为标识类型
        for col in [
            binary_column(ColumnType::MYSQL_TYPE_BLOB, 65_535),
            binary_column(ColumnType::MYSQL_TYPE_STRING, 32), // BINARY(32)
        ] {
            assert_eq!(
                mysql_value_to_value(mysql_async::Value::Bytes(raw.0.to_vec()), &col)
                    .expect("bytes"),
                Value::Bytes(raw.0.to_vec()),
                "`{:?}` 宽度 {} 不得还原 Uuid",
                col.column_type(),
                col.column_length()
            );
        }
    }

    // ==================== 布尔承载（D021） ====================

    /// `TINYINT` 列按显示宽度构造。
    fn tiny_column(display_width: u32) -> mysql_async::Column {
        mysql_async::Column::new(ColumnType::MYSQL_TYPE_TINY)
            .with_name(b"flag")
            .with_column_length(display_width)
    }

    /// 布尔承载列（`TINYINT(1)` / `BOOL`）的 `0` / `1` 读回 `Value::Bool`。
    #[test]
    fn tinyint1_column_reads_bool() {
        let boolean = tiny_column(1);
        assert_eq!(
            mysql_value_to_value(mysql_async::Value::Int(1), &boolean).expect("int 1"),
            Value::Bool(true)
        );
        assert_eq!(
            mysql_value_to_value(mysql_async::Value::Int(0), &boolean).expect("int 0"),
            Value::Bool(false)
        );
        // UNSIGNED 的 `TINYINT(1)` 读回同样布尔化
        assert_eq!(
            mysql_value_to_value(mysql_async::Value::UInt(1), &boolean).expect("uint 1"),
            Value::Bool(true)
        );
        // 布尔列里的非 0/1 取值：不折算，保留整数（有损转换不可接受）
        assert_eq!(
            mysql_value_to_value(mysql_async::Value::Int(7), &boolean).expect("int 7"),
            Value::I64(7)
        );
        // 超 i64 的无符号值仍为 `U64`
        assert_eq!(
            mysql_value_to_value(mysql_async::Value::UInt(u64::MAX), &boolean).expect("uint max"),
            Value::U64(u64::MAX)
        );
        // `NULL` 恒为 `Value::Null`，与布尔假值不同
        assert_eq!(
            mysql_value_to_value(mysql_async::Value::NULL, &boolean).expect("null"),
            Value::Null
        );
    }

    /// 非布尔宽度的整数列保持 `Value::I64`（`false` 与 `0` 的区分只在布尔列上做）。
    #[test]
    fn wider_integers_stay_i64() {
        // `TINYINT(4)` 与裸 `TINYINT`（协议显示宽度均为 4）
        for width in [4, 0] {
            let col = tiny_column(width);
            for n in [0, 1, 2] {
                assert_eq!(
                    mysql_value_to_value(mysql_async::Value::Int(n), &col).expect("int"),
                    Value::I64(n),
                    "显示宽度 {width} 的整数列不得布尔化"
                );
            }
        }
        // 非 TINYINT 的整数列同样保持 I64
        let bigint = mysql_async::Column::new(ColumnType::MYSQL_TYPE_LONGLONG)
            .with_name(b"n")
            .with_column_length(20);
        assert_eq!(
            mysql_value_to_value(mysql_async::Value::Int(0), &bigint).expect("int 0"),
            Value::I64(0)
        );
    }
}

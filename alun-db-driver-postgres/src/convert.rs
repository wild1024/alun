//! `stmt::Value` 与 PostgreSQL 二进制格式的互转。
//!
//! ## 设计
//!
//! 不启用 tokio-postgres 的 chrono / uuid / serde_json 集成 feature
//! （避免生态版本冲突），通过两个中间表示手写编解码：
//!
//! - [`PgParam`]：绑定参数方向，实现 `ToSql`
//! - [`PgValue`]：读取方向，实现 `FromSql`
//!
//! ## 读回统一宽度
//!
//! 整数列统一读回 `Value::I64`，浮点列统一读回 `Value::F64`，
//! 与 SQLite 驱动一致——窄类型模型经 `TypeMapper` 宽容匹配兼容。
//!
//! ## 已知有损点
//!
//! - `TIMESTAMPTZ`：PG 不保留原始时区偏移，写入按 UTC、读回
//!   `offset_secs = 0`
//! - `INET`：带网络掩码的 inet 读回时丢弃掩码，仅保留地址
//! - 数组：绑定与读取均不支持（v1）

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use alun_db_core::error::DbError;
use alun_db_core::row::Row;
use alun_db_core::stmt::Value;
use alun_db_core::stmt::compact::{
    CompactCidr, CompactDate, CompactDecimal, CompactId, CompactTime, CompactTimestamp,
    CompactTimestampTz,
};
use bytes::BytesMut;
use tokio_postgres::types::{FromSql, IsNull, ToSql, Type};

/// PG `DATE` 纪元（2000-01-01）相对 Unix 纪元（1970-01-01）的天数。
const PG_DATE_EPOCH_DAYS: i32 = 10_957;

/// PG `TIMESTAMP` / `TIMESTAMPTZ` 纪元（2000-01-01 UTC）相对 Unix 纪元的微秒数。
const PG_TIMESTAMP_EPOCH_MICROS: i64 = 946_684_800_000_000;

/// `ToSql` / `FromSql` 约定的错误类型。
type EncodeError = Box<dyn std::error::Error + Sync + Send>;

// ==================== 绑定方向：Value -> PgParam ====================

/// 绑定参数的中间表示。目标列类型在 `to_sql` 时可知，
/// 按列类型做宽容匹配（如 `Int` 按目标 INT2/INT4/INT8 checked 收窄）。
#[derive(Debug)]
pub enum PgParam {
    /// SQL NULL。任意目标类型可空列均可绑定。
    Null,
    /// BOOLEAN。
    Bool(bool),
    /// 有符号整数（i64 统一宽度）。
    Int(i64),
    /// 超出 i64 范围的无符号整数。
    UInt(u64),
    /// 浮点（f64 统一宽度）。
    Float(f64),
    /// 文本（TEXT 家族 / 枚举标签）。
    Text(String),
    /// 二进制（BYTEA）。
    Bytes(Vec<u8>),
    /// DATE：自 PG 纪元（2000-01-01）起的天数。
    Date(i32),
    /// TIME：自午夜起的微秒数。
    Time(i64),
    /// TIMESTAMP：自 PG 纪元起的微秒数。
    Timestamp(i64),
    /// TIMESTAMPTZ：自 PG 纪元起的 UTC 微秒数。
    TimestampTz(i64),
    /// UUID：16 字节。
    Uuid([u8; 16]),
    /// NUMERIC：预编码二进制（编码失败在建参时即报错）。
    Numeric(Vec<u8>),
    /// JSON / JSONB 载荷文本（版本字节按目标类型补写）。
    Json(String),
    /// 网络地址（INET / CIDR 列均可绑定）。
    Inet(IpAddr),
    /// CIDR：地址 + 前缀长度。
    Cidr(CompactCidr),
}

/// 将 `stmt::Value` 列表转为绑定参数。
///
/// # 错误
///
/// 值越界（时间、小数标度、CIDR 前缀）或数组绑定时返回 [`DbError`]。
pub fn to_pg_params(params: &[Value]) -> alun_db_core::error::Result<Vec<PgParam>> {
    params.iter().map(to_pg_param).collect()
}

/// 单个 `Value` → [`PgParam`]。
fn to_pg_param(value: &Value) -> alun_db_core::error::Result<PgParam> {
    match value {
        Value::Null => Ok(PgParam::Null),
        Value::Bool(b) => Ok(PgParam::Bool(*b)),
        Value::I8(n) => Ok(PgParam::Int(i64::from(*n))),
        Value::I16(n) => Ok(PgParam::Int(i64::from(*n))),
        Value::I32(n) => Ok(PgParam::Int(i64::from(*n))),
        Value::I64(n) => Ok(PgParam::Int(*n)),
        Value::U8(n) => Ok(PgParam::Int(i64::from(*n))),
        Value::U16(n) => Ok(PgParam::Int(i64::from(*n))),
        Value::U32(n) => Ok(PgParam::Int(i64::from(*n))),
        // 超出 i64 的 u64 仅 NUMERIC / 文本列可承载
        Value::U64(n) => match i64::try_from(*n) {
            Ok(signed) => Ok(PgParam::Int(signed)),
            Err(_) => Ok(PgParam::UInt(*n)),
        },
        Value::F32(f) => Ok(PgParam::Float(f64::from(*f))),
        Value::F64(f) => Ok(PgParam::Float(*f)),
        // 枚举标签与文本同构（PG 枚举二进制格式即标签文本）
        Value::Text(s) | Value::Enum(s) => Ok(PgParam::Text(s.clone())),
        Value::Bytes(b) => Ok(PgParam::Bytes(b.clone())),
        // PG 二进制时间戳是**自 PG 纪元（2000-01-01 UTC）起**的偏移：
        // 写入 = unix 微秒 − 纪元差；读回 = 库内值 + 纪元差（见 `pg_value_to_value`）。
        // 两侧方向必须相反——曾同时写反（写入 +、读回 −），框架内往返自洽但库内时间 +60 年。
        Value::Date(d) => Ok(PgParam::Date(d.0 - PG_DATE_EPOCH_DAYS)),
        Value::Time(t) => {
            if !t.is_valid() {
                return Err(DbError::InvalidValue("time exceeds one day".into()));
            }
            Ok(PgParam::Time((t.0 / 1_000) as i64))
        }
        Value::Timestamp(ts) => Ok(PgParam::Timestamp(ts.0 - PG_TIMESTAMP_EPOCH_MICROS)),
        Value::TimestampTz(ts) => {
            if !ts.is_valid() {
                return Err(DbError::InvalidValue(
                    "timestamp offset out of range".into(),
                ));
            }
            // PG 不保留时区偏移：统一按 UTC 写入（模块文档「已知有损点」）
            Ok(PgParam::TimestampTz(
                ts.utc_micros - PG_TIMESTAMP_EPOCH_MICROS,
            ))
        }
        Value::Uuid(id) | Value::Ulid(id) => Ok(PgParam::Uuid(id.0)),
        Value::Decimal(dec) => {
            if !dec.is_valid() {
                return Err(DbError::InvalidValue("decimal scale exceeds 38".into()));
            }
            let bytes = encode_numeric(dec).map_err(DbError::InvalidValue)?;
            Ok(PgParam::Numeric(bytes))
        }
        Value::Json(s) => Ok(PgParam::Json(s.clone())),
        Value::Inet(addr) => Ok(PgParam::Inet(*addr)),
        Value::Cidr(cidr) => {
            if !cidr.is_valid() {
                return Err(DbError::InvalidValue("cidr prefix out of range".into()));
            }
            Ok(PgParam::Cidr(cidr.clone()))
        }
        Value::Array(_) => Err(DbError::UnsupportedFeature {
            feature: "array binding",
            dialect: "postgres".into(),
        }),
    }
}

/// 转换为 tokio-postgres 要求的 trait 对象引用切片。
pub(crate) fn as_param_refs(params: &[PgParam]) -> Vec<&(dyn ToSql + Sync)> {
    params.iter().map(|p| p as &(dyn ToSql + Sync)).collect()
}

/// 判断目标类型是否文本家族（含枚举标签）。
fn is_text_family(ty: &Type) -> bool {
    matches!(
        *ty,
        Type::TEXT | Type::VARCHAR | Type::NAME | Type::BPCHAR | Type::UNKNOWN
    ) || matches!(ty.kind(), tokio_postgres::types::Kind::Enum(_))
}

/// 类型不匹配错误。
fn type_mismatch(ty: &Type) -> EncodeError {
    format!("cannot bind parameter to column type {ty}").into()
}

/// 整数收窄越界错误。
fn range_err(n: i64, target: &str) -> EncodeError {
    format!("value {n} out of range for {target}").into()
}

/// 将整数按目标列宽度写入（宽容匹配）。
fn write_int(n: i64, ty: &Type, out: &mut BytesMut) -> std::result::Result<(), EncodeError> {
    match *ty {
        Type::INT2 => {
            let v = i16::try_from(n).map_err(|_| range_err(n, "INT2"))?;
            out.extend_from_slice(&v.to_be_bytes());
        }
        Type::INT4 => {
            let v = i32::try_from(n).map_err(|_| range_err(n, "INT4"))?;
            out.extend_from_slice(&v.to_be_bytes());
        }
        Type::INT8 => out.extend_from_slice(&n.to_be_bytes()),
        Type::FLOAT4 => out.extend_from_slice(&(n as f32).to_be_bytes()),
        Type::FLOAT8 => out.extend_from_slice(&(n as f64).to_be_bytes()),
        Type::NUMERIC => {
            let dec = CompactDecimal {
                mantissa: i128::from(n),
                scale: 0,
            };
            let bytes = encode_numeric(&dec).map_err(|e| -> EncodeError { e.into() })?;
            out.extend_from_slice(&bytes);
        }
        _ if is_text_family(ty) => out.extend_from_slice(n.to_string().as_bytes()),
        _ => return Err(type_mismatch(ty)),
    }
    Ok(())
}

impl ToSql for PgParam {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            // NULL 对任意可空位置合法，不写载荷
            PgParam::Null => return Ok(IsNull::Yes),
            PgParam::Bool(b) => {
                if !matches!(*ty, Type::BOOL) {
                    return Err(type_mismatch(ty));
                }
                out.extend_from_slice(&[*b as u8]);
            }
            PgParam::Int(n) => write_int(*n, ty, out)?,
            PgParam::UInt(n) => match i64::try_from(*n) {
                Ok(signed) => write_int(signed, ty, out)?,
                // 超出 i64：仅 NUMERIC / 文本可承载
                Err(_) => match *ty {
                    Type::NUMERIC => {
                        let dec = CompactDecimal {
                            mantissa: i128::from(*n),
                            scale: 0,
                        };
                        let bytes =
                            encode_numeric(&dec).map_err(|e| -> EncodeError { e.into() })?;
                        out.extend_from_slice(&bytes);
                    }
                    _ if is_text_family(ty) => out.extend_from_slice(n.to_string().as_bytes()),
                    _ => return Err(type_mismatch(ty)),
                },
            },
            PgParam::Float(f) => match *ty {
                Type::FLOAT4 => out.extend_from_slice(&(*f as f32).to_be_bytes()),
                Type::FLOAT8 => out.extend_from_slice(&f.to_be_bytes()),
                Type::NUMERIC => {
                    let dec = decimal_from_f64(*f).map_err(|e| -> EncodeError { e.into() })?;
                    let bytes = encode_numeric(&dec).map_err(|e| -> EncodeError { e.into() })?;
                    out.extend_from_slice(&bytes);
                }
                _ if is_text_family(ty) => out.extend_from_slice(f.to_string().as_bytes()),
                _ => return Err(type_mismatch(ty)),
            },
            PgParam::Text(s) => {
                if !is_text_family(ty) {
                    return Err(type_mismatch(ty));
                }
                out.extend_from_slice(s.as_bytes());
            }
            PgParam::Bytes(b) => {
                if !matches!(*ty, Type::BYTEA) {
                    return Err(type_mismatch(ty));
                }
                out.extend_from_slice(b);
            }
            PgParam::Date(days) => {
                if !matches!(*ty, Type::DATE) {
                    return Err(type_mismatch(ty));
                }
                out.extend_from_slice(&days.to_be_bytes());
            }
            PgParam::Time(micros) => {
                if !matches!(*ty, Type::TIME) {
                    return Err(type_mismatch(ty));
                }
                out.extend_from_slice(&micros.to_be_bytes());
            }
            PgParam::Timestamp(micros) => {
                if !matches!(*ty, Type::TIMESTAMP) {
                    return Err(type_mismatch(ty));
                }
                out.extend_from_slice(&micros.to_be_bytes());
            }
            PgParam::TimestampTz(micros) => {
                if !matches!(*ty, Type::TIMESTAMPTZ) {
                    return Err(type_mismatch(ty));
                }
                out.extend_from_slice(&micros.to_be_bytes());
            }
            PgParam::Uuid(bytes) => {
                if !matches!(*ty, Type::UUID) {
                    return Err(type_mismatch(ty));
                }
                out.extend_from_slice(bytes);
            }
            PgParam::Numeric(bytes) => {
                if !matches!(*ty, Type::NUMERIC) {
                    return Err(type_mismatch(ty));
                }
                out.extend_from_slice(bytes);
            }
            // JSONB 二进制为版本字节 0x01 + 文本；JSON 为纯文本
            PgParam::Json(payload) => match *ty {
                Type::JSONB => {
                    out.extend_from_slice(b"\x01");
                    out.extend_from_slice(payload.as_bytes());
                }
                Type::JSON => out.extend_from_slice(payload.as_bytes()),
                _ => return Err(type_mismatch(ty)),
            },
            PgParam::Inet(addr) => match *ty {
                Type::INET => out.extend_from_slice(&encode_inet(addr, full_prefix(addr), false)),
                Type::CIDR => out.extend_from_slice(&encode_inet(addr, full_prefix(addr), true)),
                _ => return Err(type_mismatch(ty)),
            },
            PgParam::Cidr(cidr) => match *ty {
                // CIDR 列要求主机位为零，否则服务端拒绝
                Type::CIDR => {
                    let masked = mask_address(cidr);
                    out.extend_from_slice(&encode_inet(&masked, cidr.prefix_len, true));
                }
                Type::INET => {
                    out.extend_from_slice(&encode_inet(&cidr.addr, full_prefix(&cidr.addr), false))
                }
                _ => return Err(type_mismatch(ty)),
            },
        }
        Ok(IsNull::No)
    }

    // 宽容匹配：类型检查在 to_sql 内按值与列的组合精确进行
    fn accepts(_ty: &Type) -> bool {
        true
    }

    // accepts 恒真，检查逻辑等价于 to_sql 本体
    fn to_sql_checked(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> std::result::Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        self.to_sql(ty, out)
    }
}

// ==================== 读取方向：PgValue -> Value ====================

/// 读取列值的中间表示。按列类型解码为 PG 原生宽度，
/// 再经 [`pg_value_to_value`] 统一为 `stmt::Value` 宽度。
/// SQL NULL 由 `Option<PgValue>` 承载，不设变体。
#[derive(Debug)]
pub enum PgValue {
    /// BOOLEAN。
    Bool(bool),
    /// INT2。
    SmallInt(i16),
    /// INT4。
    Int(i32),
    /// INT8。
    BigInt(i64),
    /// FLOAT4。
    Float4(f32),
    /// FLOAT8。
    Float8(f64),
    /// TEXT 家族 / 枚举标签。
    Text(String),
    /// BYTEA。
    Bytes(Vec<u8>),
    /// DATE：自 PG 纪元起的天数。
    Date(i32),
    /// TIME：自午夜起的微秒数。
    Time(i64),
    /// TIMESTAMP：自 PG 纪元起的微秒数。
    Timestamp(i64),
    /// TIMESTAMPTZ：自 PG 纪元起的 UTC 微秒数。
    TimestampTz(i64),
    /// UUID：16 字节。
    Uuid([u8; 16]),
    /// NUMERIC。
    Numeric(CompactDecimal),
    /// JSON / JSONB 文本载荷。
    Json(String),
    /// INET 地址（掩码丢弃）。
    Inet(IpAddr),
    /// CIDR：地址 + 前缀长度。
    Cidr(CompactCidr),
}

/// 按固定宽度读取大端整数，长度不符时报错。
fn read_int<const N: usize>(raw: &[u8]) -> Result<[u8; N], EncodeError> {
    if raw.len() != N {
        return Err(format!("expected {N} bytes, got {}", raw.len()).into());
    }
    let mut buf = [0u8; N];
    buf.copy_from_slice(raw);
    Ok(buf)
}

/// 按 UTF-8 读取文本。
fn read_text(raw: &[u8]) -> Result<String, EncodeError> {
    String::from_utf8(raw.to_vec()).map_err(|e| format!("column text is not UTF-8: {e}").into())
}

impl<'a> FromSql<'a> for PgValue {
    fn from_sql(
        ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        let value = match *ty {
            Type::BOOL => PgValue::Bool(read_int::<1>(raw)?[0] != 0),
            Type::INT2 => PgValue::SmallInt(i16::from_be_bytes(read_int::<2>(raw)?)),
            Type::INT4 => PgValue::Int(i32::from_be_bytes(read_int::<4>(raw)?)),
            Type::INT8 => PgValue::BigInt(i64::from_be_bytes(read_int::<8>(raw)?)),
            Type::FLOAT4 => PgValue::Float4(f32::from_be_bytes(read_int::<4>(raw)?)),
            Type::FLOAT8 => PgValue::Float8(f64::from_be_bytes(read_int::<8>(raw)?)),
            Type::TEXT | Type::VARCHAR | Type::NAME | Type::BPCHAR | Type::UNKNOWN => {
                PgValue::Text(read_text(raw)?)
            }
            Type::BYTEA => PgValue::Bytes(raw.to_vec()),
            Type::DATE => PgValue::Date(i32::from_be_bytes(read_int::<4>(raw)?)),
            Type::TIME => PgValue::Time(i64::from_be_bytes(read_int::<8>(raw)?)),
            Type::TIMESTAMP => PgValue::Timestamp(i64::from_be_bytes(read_int::<8>(raw)?)),
            Type::TIMESTAMPTZ => PgValue::TimestampTz(i64::from_be_bytes(read_int::<8>(raw)?)),
            Type::UUID => PgValue::Uuid(read_int::<16>(raw)?),
            Type::NUMERIC => {
                PgValue::Numeric(decode_numeric(raw).map_err(|e| -> EncodeError { e.into() })?)
            }
            Type::JSON => PgValue::Json(read_text(raw)?),
            // JSONB 二进制为版本字节 + 文本
            Type::JSONB => {
                if raw.is_empty() || raw[0] != 0x01 {
                    return Err("invalid JSONB binary (version byte)".into());
                }
                PgValue::Json(read_text(&raw[1..])?)
            }
            Type::INET => {
                let (addr, _bits) = decode_inet(raw).map_err(|e| -> EncodeError { e.into() })?;
                PgValue::Inet(addr)
            }
            Type::CIDR => {
                let (addr, bits) = decode_inet(raw).map_err(|e| -> EncodeError { e.into() })?;
                PgValue::Cidr(CompactCidr {
                    addr,
                    prefix_len: bits,
                })
            }
            // 枚举标签即文本
            _ if matches!(ty.kind(), tokio_postgres::types::Kind::Enum(_)) => {
                PgValue::Text(read_text(raw)?)
            }
            _ => return Err(format!("unsupported column type {ty}").into()),
        };
        Ok(value)
    }

    // 宽容匹配：支持的类型在 from_sql 内精确解码
    fn accepts(_ty: &Type) -> bool {
        true
    }
}

/// [`PgValue`] → [`Value`]。统一读回宽度（见模块文档）。
fn pg_value_to_value(v: PgValue) -> Value {
    match v {
        PgValue::Bool(b) => Value::Bool(b),
        PgValue::SmallInt(n) => Value::I64(i64::from(n)),
        PgValue::Int(n) => Value::I64(i64::from(n)),
        PgValue::BigInt(n) => Value::I64(n),
        PgValue::Float4(f) => Value::F64(f64::from(f)),
        PgValue::Float8(f) => Value::F64(f),
        PgValue::Text(s) => Value::Text(s),
        PgValue::Bytes(b) => Value::Bytes(b),
        PgValue::Date(days) => Value::Date(CompactDate(days + PG_DATE_EPOCH_DAYS)),
        PgValue::Time(micros) => Value::Time(CompactTime(micros as u64 * 1_000)),
        PgValue::Timestamp(micros) => {
            Value::Timestamp(CompactTimestamp(micros + PG_TIMESTAMP_EPOCH_MICROS))
        }
        PgValue::TimestampTz(micros) => Value::TimestampTz(CompactTimestampTz {
            utc_micros: micros + PG_TIMESTAMP_EPOCH_MICROS,
            // PG 不保留原始时区偏移，读回统一为 UTC
            offset_secs: 0,
        }),
        PgValue::Uuid(bytes) => Value::Uuid(CompactId(bytes)),
        PgValue::Numeric(dec) => Value::Decimal(dec),
        PgValue::Json(s) => Value::Json(s),
        PgValue::Inet(addr) => Value::Inet(addr),
        PgValue::Cidr(cidr) => Value::Cidr(cidr),
    }
}

/// 将 tokio-postgres 行转换为 `Row`。
///
/// # 错误
///
/// 列读取或解码失败时返回 [`DbError`]。
pub fn pg_row_to_row(pg_row: &tokio_postgres::Row) -> alun_db_core::error::Result<Row> {
    let mut row = Row::with_capacity(pg_row.len());
    for (i, column) in pg_row.columns().iter().enumerate() {
        let pg_value: Option<PgValue> = pg_row.try_get(i).map_err(|e| DbError::Query {
            sql: String::new(),
            source: Box::new(e),
        })?;
        let value = pg_value.map(pg_value_to_value).unwrap_or(Value::Null);
        row.insert(column.name(), value);
    }
    Ok(row)
}

// ==================== NUMERIC 编解码 ====================

/// 编码 NUMERIC 二进制格式。
///
/// 布局：`ndigits(i16) weight(i16) sign(u16) dscale(u16)` + ndigits 个
/// base-10000 数字（各 i16，大端）。采用十进制字符串分组法：
/// 尾数绝对值按 `scale` 拆整数 / 小数部分，整数从右起、小数从左起
/// 每 4 位一组。
fn encode_numeric(dec: &CompactDecimal) -> Result<Vec<u8>, String> {
    let CompactDecimal { mantissa, scale } = *dec;
    let dscale = u16::try_from(scale).map_err(|_| "scale exceeds u16".to_string())?;

    // 零值特判：无数字组
    if mantissa == 0 {
        let mut out = Vec::with_capacity(8);
        out.extend_from_slice(&0i16.to_be_bytes()); // ndigits
        out.extend_from_slice(&0i16.to_be_bytes()); // weight
        out.extend_from_slice(&0u16.to_be_bytes()); // sign：正
        out.extend_from_slice(&dscale.to_be_bytes());
        return Ok(out);
    }

    // 0x0000 正 / 0x4000 负（NaN 0xC000、Inf 0xD000 不可表示于 CompactDecimal）
    let sign: u16 = if mantissa < 0 { 0x4000 } else { 0x0000 };
    let digits_str = mantissa.unsigned_abs().to_string();
    let scale_len = scale as usize;

    // 按 scale 拆分整数 / 小数数字串；纯小数时小数部分左补零至 scale 位
    let (int_str, frac_str): (&str, String) = if digits_str.len() > scale_len {
        let split = digits_str.len() - scale_len;
        let (int_part, frac_part) = digits_str.split_at(split);
        (int_part, frac_part.to_string())
    } else {
        ("", format!("{:0>width$}", digits_str, width = scale_len))
    };

    // 整数部分：从右起每 4 位一组（首组可不足 4 位）
    let int_groups = group_right(int_str)?;
    // 小数部分：左起每 4 位一组，末组右补零至 4 位
    let frac_groups = group_left(&frac_str)?;

    // 归一化（与 PG 自身的写法一致）：
    //  - 小数部分的**前导全零组**不写进 digits，改由 `weight` 表达
    //    （0.00001234 → digits=[1234]、weight=-2，写 [0,1234] 会被解读为更小的值）
    //  - 尾随全零组无信息量，直接丢弃（10000 → digits=[1]、weight=1）
    let leading_zero_groups = frac_groups.iter().take_while(|g| **g == 0).count();
    let mut digits = int_groups;
    let weight: i16 = if digits.is_empty() {
        // 纯小数：组 0 位于小数点后第 leading_zero_groups + 1 组
        -((leading_zero_groups + 1) as i16)
    } else {
        digits.len() as i16 - 1
    };
    digits.extend(frac_groups.into_iter().skip(leading_zero_groups));
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }

    let mut out = Vec::with_capacity(8 + digits.len() * 2);
    out.extend_from_slice(&(digits.len() as i16).to_be_bytes());
    out.extend_from_slice(&weight.to_be_bytes());
    out.extend_from_slice(&sign.to_be_bytes());
    out.extend_from_slice(&dscale.to_be_bytes());
    for d in &digits {
        out.extend_from_slice(&d.to_be_bytes());
    }
    Ok(out)
}

/// 解码 NUMERIC 二进制格式为 `CompactDecimal`。
fn decode_numeric(bytes: &[u8]) -> Result<CompactDecimal, String> {
    if bytes.len() < 8 {
        return Err("numeric binary too short".into());
    }
    let ndigits = i16::from_be_bytes([bytes[0], bytes[1]]) as usize;
    let weight = i16::from_be_bytes([bytes[2], bytes[3]]) as i32;
    let sign = u16::from_be_bytes([bytes[4], bytes[5]]);
    let dscale = u16::from_be_bytes([bytes[6], bytes[7]]) as u32;

    // NaN / Infinity 无法映射到 mantissa × 10^-scale
    if sign != 0x0000 && sign != 0x4000 {
        return Err(format!("numeric sign code {sign:#06x} not representable"));
    }
    let is_negative = sign == 0x4000;

    if bytes.len() < 8 + ndigits * 2 {
        return Err("numeric binary truncated".into());
    }

    // 以十进制字符串重组：value = Σ digits[i] × 10000^(weight − i)
    //
    // PG 发送的 numeric **已规范化**：尾随零组不出现在 `digits` 里，
    // 而是把 `weight` 抬高（如 10000 = digits[1]、weight[1]），
    // 且 `weight < 0` 时组 0 位于小数点后第 `|weight|` 组。
    // 两种情形都必须按位权补零，否则 10000 会被解成 1、1e10 解成 100。
    let group_at = |i: usize| -> u16 {
        if i < ndigits {
            u16::from_be_bytes([bytes[8 + i * 2], bytes[9 + i * 2]])
        } else {
            0
        }
    };
    let mut int_str = String::new();
    let mut frac_str = String::new();
    if weight >= 0 {
        let weight = weight as usize;
        for i in 0..=weight {
            let digit = group_at(i);
            if i == 0 {
                // 最高整数组：自然宽度（无前导零）
                int_str.push_str(&digit.to_string());
            } else {
                int_str.push_str(&format!("{digit:04}"));
            }
        }
        for i in (weight + 1)..ndigits {
            frac_str.push_str(&format!("{:04}", group_at(i)));
        }
    } else {
        // 纯小数：组 0 之前补 (-weight - 1) 个零组
        for _ in 0..(-weight - 1) {
            frac_str.push_str("0000");
        }
        for i in 0..ndigits {
            frac_str.push_str(&format!("{:04}", group_at(i)));
        }
    }

    // 小数位对齐 dscale：超出部分必为分组补零；不足部分（服务端
    // 裁剪了尾零组）右补零
    if frac_str.len() > dscale as usize {
        let excess = &frac_str[dscale as usize..];
        if excess.bytes().any(|b| b != b'0') {
            return Err("numeric has significant digits beyond dscale".into());
        }
        frac_str.truncate(dscale as usize);
    } else if frac_str.len() < dscale as usize {
        frac_str.extend(std::iter::repeat_n('0', dscale as usize - frac_str.len()));
    }

    if int_str.is_empty() {
        int_str.push('0');
    }
    let combined = format!("{int_str}{frac_str}");
    // CompactDecimal 容量：38 位有效数字。值 < 1 时整数部分仅有占位 "0"，
    // 它不是有效数字，剔除后再计数；前导零不影响解析结果
    let significant = combined.trim_start_matches('0');
    if significant.len() > 38 {
        return Err(format!(
            "numeric with {combined_len} digits exceeds 38-digit capacity",
            combined_len = significant.len()
        ));
    }
    let mantissa: i128 = combined
        .parse()
        .map_err(|e| format!("numeric mantissa parse failed: {e}"))?;
    let mantissa = if is_negative { -mantissa } else { mantissa };
    Ok(CompactDecimal {
        mantissa,
        scale: dscale,
    })
}

/// 校验并解析 1-4 位数字组。
fn parse_digit_group(s: &str) -> Result<u16, String> {
    if s.is_empty() || s.len() > 4 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("invalid numeric digit group `{s}`"));
    }
    s.parse::<u16>()
        .map_err(|e| format!("digit group `{s}` out of range: {e}"))
}

/// 整数部分分组：从右起每 4 位一组，首组（最左）可不足 4 位。
fn group_right(s: &str) -> Result<Vec<u16>, String> {
    let mut groups = Vec::new();
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    while end > 0 {
        let start = end.saturating_sub(4);
        let group = std::str::from_utf8(&bytes[start..end])
            .map_err(|e| format!("numeric digits are not UTF-8: {e}"))?;
        groups.push(parse_digit_group(group)?);
        end = start;
    }
    groups.reverse();
    Ok(groups)
}

/// 小数部分分组：左起每 4 位一组，末组右补零至 4 位。
fn group_left(s: &str) -> Result<Vec<u16>, String> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    let padded_len = s.len().div_ceil(4) * 4;
    let mut padded = s.to_string();
    padded.extend(std::iter::repeat_n('0', padded_len - s.len()));
    padded
        .as_bytes()
        .chunks_exact(4)
        .map(|chunk| {
            let group = std::str::from_utf8(chunk)
                .map_err(|e| format!("numeric digits are not UTF-8: {e}"))?;
            parse_digit_group(group)
        })
        .collect()
}

// ==================== INET / CIDR 编解码 ====================

/// 编码 INET / CIDR 二进制（与 PG `inet_send` 线格式逐字节一致）：
/// `family(1) bits(1) is_cidr(1) len(1) + 地址字节(len)`。
fn encode_inet(addr: &IpAddr, prefix_len: u8, is_cidr: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(20);
    match addr {
        IpAddr::V4(v4) => {
            out.push(2); // PGSQL_AF_INET
            out.push(prefix_len);
            out.push(is_cidr as u8);
            out.push(4); // 地址字节数（varlen）
            out.extend_from_slice(&v4.octets());
        }
        IpAddr::V6(v6) => {
            out.push(3); // PGSQL_AF_INET6
            out.push(prefix_len);
            out.push(is_cidr as u8);
            out.push(16); // 地址字节数（varlen）
            out.extend_from_slice(&v6.octets());
        }
    }
    out
}

/// 解码 INET / CIDR 二进制（与 PG `inet_send` 线格式一致：
/// `family(1) bits(1) is_cidr(1) len(1) + 地址字节(len)`），
/// 返回地址与前缀长度（INET 读回时掩码丢弃）。
fn decode_inet(raw: &[u8]) -> Result<(IpAddr, u8), String> {
    if raw.len() < 4 {
        return Err("inet binary too short".into());
    }
    let family = raw[0];
    let bits = raw[1];
    // raw[2] 为 is_cidr 标志，读回时由目标 Value 类型决定语义，不使用
    // raw[3] 为地址字节数（varlen），必须据此切出地址——早期实现遗漏该字节、
    // 直接从 raw[3..] 取地址，导致所有真实 inet 值解码失败（自造往返测试因
    // 编码/解码同样处理而未能发现与 PG 线格式的不兼容）。
    let addr_len = raw[3] as usize;
    if raw.len() < 4 + addr_len {
        return Err("inet binary truncated".into());
    }
    let addr_bytes = &raw[4..4 + addr_len];
    let addr = match family {
        2 => {
            if addr_bytes.len() != 4 {
                return Err("invalid IPv4 inet binary".into());
            }
            IpAddr::V4(Ipv4Addr::new(
                addr_bytes[0],
                addr_bytes[1],
                addr_bytes[2],
                addr_bytes[3],
            ))
        }
        3 => {
            if addr_bytes.len() != 16 {
                return Err("invalid IPv6 inet binary".into());
            }
            let mut octets = [0u8; 16];
            octets.copy_from_slice(addr_bytes);
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        other => return Err(format!("unknown inet family {other}")),
    };
    Ok((addr, bits))
}

/// 地址族的全长前缀（IPv4 = 32 / IPv6 = 128）。
fn full_prefix(addr: &IpAddr) -> u8 {
    match addr {
        IpAddr::V4(_) => 32,
        IpAddr::V6(_) => 128,
    }
}

/// 掩蔽主机位：CIDR 绑定要求主机位为零，否则服务端拒绝。
fn mask_address(cidr: &CompactCidr) -> IpAddr {
    match cidr.addr {
        IpAddr::V4(v4) => {
            let bits = u32::from(v4);
            // prefix_len == 0 时移位 32 位是 UB，单独处理
            let masked = if cidr.prefix_len == 0 {
                0
            } else {
                bits & (u32::MAX << (32 - u32::from(cidr.prefix_len)))
            };
            IpAddr::V4(Ipv4Addr::from(masked))
        }
        IpAddr::V6(v6) => {
            let bits = u128::from(v6);
            let masked = if cidr.prefix_len == 0 {
                0
            } else {
                bits & (u128::MAX << (128 - u32::from(cidr.prefix_len)))
            };
            IpAddr::V6(Ipv6Addr::from(masked))
        }
    }
}

// ==================== 浮点 → NUMERIC 辅助 ====================

/// f64 → `CompactDecimal`（经最短十进制字符串，仅用于绑定 NUMERIC 列）。
fn decimal_from_f64(f: f64) -> Result<CompactDecimal, String> {
    if !f.is_finite() {
        return Err(format!("float {f} not representable as numeric"));
    }
    parse_decimal_str(&f.to_string())
}

/// 解析无指数记法的十进制字符串（如 `"-123.45"`）。
fn parse_decimal_str(s: &str) -> Result<CompactDecimal, String> {
    let negative = s.starts_with('-');
    let body = s.strip_prefix('-').unwrap_or(s);
    let (int_part, frac_part) = body.split_once('.').unwrap_or((body, ""));
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(format!("invalid decimal string `{s}`"));
    }
    if !int_part
        .bytes()
        .chain(frac_part.bytes())
        .all(|b| b.is_ascii_digit())
    {
        return Err(format!("invalid decimal string `{s}`"));
    }
    let combined = format!("{int_part}{frac_part}");
    if combined.len() > 38 {
        return Err(format!("`{s}` exceeds 38-digit numeric capacity"));
    }
    let mantissa: i128 = combined
        .parse()
        .map_err(|e| format!("`{s}` overflows i128: {e}"))?;
    Ok(CompactDecimal {
        mantissa: if negative { -mantissa } else { mantissa },
        scale: frac_part.len() as u32,
    })
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    /// NUMERIC 编解码往返：mantissa 与 scale 保持一致。
    #[test]
    fn numeric_roundtrip() {
        for (mantissa, scale) in [
            (0i128, 0u32),
            (0, 2),
            (12345, 2),  // 123.45
            (-6789, 3),  // -6.789
            (5, 2),      // 0.05
            (123456, 6), // 0.123456
            (1, 0),
            (10000, 2), // 100.00
            (-1, 0),
        ] {
            let dec = CompactDecimal { mantissa, scale };
            let bytes = encode_numeric(&dec).expect("encode");
            let back = decode_numeric(&bytes).expect("decode");
            assert_eq!(
                (back.mantissa, back.scale),
                (mantissa, scale),
                "roundtrip failed for {mantissa}e-{scale}"
            );
        }
    }

    /// PG 服务端实际产生的裁剪形态：1.50（dscale 2）解码为 1.50。
    #[test]
    fn numeric_decode_server_trimmed() {
        // 手工构造：digits [1, 5000] weight 0 dscale 2 → 1.50
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2i16.to_be_bytes());
        bytes.extend_from_slice(&0i16.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&2u16.to_be_bytes());
        bytes.extend_from_slice(&1i16.to_be_bytes());
        bytes.extend_from_slice(&5000i16.to_be_bytes());
        let dec = decode_numeric(&bytes).expect("decode");
        assert_eq!(dec.mantissa, 150);
        assert_eq!(dec.scale, 2);
    }

    /// INET 编解码往返。
    #[test]
    fn inet_roundtrip() {
        let addr: IpAddr = "192.168.1.5".parse().expect("addr");
        let bytes = encode_inet(&addr, 32, false);
        let (back, bits) = decode_inet(&bytes).expect("decode");
        assert_eq!(back, addr);
        assert_eq!(bits, 32);
    }

    /// 与 PG 真实线格式的兼容性（回归：早期实现遗漏 len 字节，所有真实值解码失败）。
    ///
    /// 字节取自真实服务端：`SELECT encode(inet_send('10.1.2.3'::inet), 'hex')`
    /// → `022000040a010203`（family=2, bits=32, is_cidr=0, len=4, addr）。
    #[test]
    fn inet_matches_real_pg_wire_format() {
        let raw = [0x02u8, 0x20, 0x00, 0x04, 10, 1, 2, 3];
        let (addr, bits) = decode_inet(&raw).expect("decode real pg bytes");
        assert_eq!(addr, "10.1.2.3".parse::<IpAddr>().expect("expected addr"));
        assert_eq!(bits, 32);

        // 编码端必须逐字节还原同一线格式
        let encoded = encode_inet(&"10.1.2.3".parse::<IpAddr>().expect("expected addr"), 32, false);
        assert_eq!(encoded, raw);

        // IPv6：family=3, bits=128, is_cidr=0, len=16
        let v6 = "2001:db8::1".parse::<IpAddr>().expect("expected v6");
        let enc6 = encode_inet(&v6, 128, false);
        assert_eq!(enc6.len(), 4 + 16);
        assert_eq!(&enc6[..4], &[3, 128, 0, 16]);
        let (back6, b6) = decode_inet(&enc6).expect("decode v6");
        assert_eq!(back6, v6);
        assert_eq!(b6, 128);
    }

    /// IPv6 与 CIDR 编解码往返：嵌入的 IPv4 映射地址与带掩码的网段一致。
    #[test]
    fn inet_roundtrip_v6_and_cidr() {
        let addr: IpAddr = "::ffff:c0a8:0105".parse().expect("v6 mapped addr");
        let bytes = encode_inet(&addr, 128, false);
        let (back, bits) = decode_inet(&bytes).expect("decode v6");
        assert_eq!(back, addr);
        assert_eq!(bits, 128);

        // 带前缀长度的 CIDR 往返：is_cidr 标志不影响地址与前缀的解码
        let prefix_len = 120u8;
        let cidr_bytes = encode_inet(&addr, prefix_len, true);
        let (cidr_back, cidr_bits) = decode_inet(&cidr_bytes).expect("decode cidr");
        assert_eq!(cidr_back, addr);
        assert_eq!(cidr_bits, prefix_len);
    }

    /// 全前缀长度：IPv4 为 32，IPv6 为 128。
    #[test]
    fn full_prefix_returns_32_or_128() {
        let v4: IpAddr = "10.0.0.1".parse().expect("v4 addr");
        let v6: IpAddr = "2001:db8::1".parse().expect("v6 addr");
        assert_eq!(full_prefix(&v4), 32);
        assert_eq!(full_prefix(&v6), 128);
    }

    /// 掩蔽主机位：prefix_len=0 全零、满前缀保持原地址、中间前缀清零主机位。
    #[test]
    fn mask_address_zero_and_full_prefix() {
        let v4: IpAddr = "192.168.1.5".parse().expect("v4 addr");
        let cidr_zero = CompactCidr {
            addr: v4,
            prefix_len: 0,
        };
        let masked_zero = mask_address(&cidr_zero);
        assert_eq!(masked_zero, "0.0.0.0".parse::<IpAddr>().expect("zero addr"));

        let cidr_full = CompactCidr {
            addr: v4,
            prefix_len: 32,
        };
        let masked_full = mask_address(&cidr_full);
        assert_eq!(masked_full, v4);

        // /24：仅保留前 24 位（前三字节含 192.168.1），主机位清零
        let cidr_24 = CompactCidr {
            addr: v4,
            prefix_len: 24,
        };
        let masked_24 = mask_address(&cidr_24);
        assert_eq!(masked_24, "192.168.1.0".parse::<IpAddr>().expect("masked"));

        // IPv6 满前缀 /128 保持原地址，/0 全零
        let v6: IpAddr = "2001:db8::1234".parse().expect("v6 addr");
        let cidr_v6_full = CompactCidr {
            addr: v6,
            prefix_len: 128,
        };
        assert_eq!(mask_address(&cidr_v6_full), v6);
        let cidr_v6_zero = CompactCidr {
            addr: v6,
            prefix_len: 0,
        };
        assert_eq!(
            mask_address(&cidr_v6_zero),
            "::".parse::<IpAddr>().expect("zero v6")
        );
    }

    /// 畸形二进制解码：过短、未知 family、地址字节数不匹配均应报错。
    #[test]
    fn decode_inet_rejects_malformed() {
        // 过短（< 4 字节）
        assert!(decode_inet(&[1u8]).is_err(), "too-short should error");
        assert!(decode_inet(&[1u8, 2, 3]).is_err(), "3-byte should error");

        // 未知 family
        let unknown = vec![9u8, 32, 0, 1, 2, 3, 4]; // family=9
        assert!(
            decode_inet(&unknown).is_err(),
            "unknown family should error"
        );

        // IPv4 声明但地址字节数 ≠ 4
        let v4_bad_len: Vec<u8> = vec![2u8, 32, 0, 1, 2, 3]; // family=2 仅 3 字节
        assert!(
            decode_inet(&v4_bad_len).is_err(),
            "v4 with wrong byte count should error"
        );

        // IPv6 声明但地址字节数 ≠ 16（长度 ≥4 以通过过短检查，命中地址字节校验）
        let v6_bad_len: Vec<u8> = vec![3u8, 128, 0, 1, 2, 3]; // family=3 仅 3 字节地址
        assert!(
            decode_inet(&v6_bad_len).is_err(),
            "v6 with wrong byte count should error"
        );
    }

    /// NaN 符号码（0xC000）不可映射到 CompactDecimal，解码应报错。
    #[test]
    fn decode_numeric_rejects_nan_sign() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0i16.to_be_bytes()); // ndigits
        bytes.extend_from_slice(&0i16.to_be_bytes()); // weight
        bytes.extend_from_slice(&0xC000u16.to_be_bytes()); // sign=NaN
        bytes.extend_from_slice(&0u16.to_be_bytes()); // dscale
        assert!(
            decode_numeric(&bytes).is_err(),
            "NaN sign code should error"
        );
    }

    /// ndigits 声明超过可用字节，解码应报截断错误。
    #[test]
    fn decode_numeric_rejects_truncated() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&5i16.to_be_bytes()); // ndigits=5，但无后续数字
        bytes.extend_from_slice(&0i16.to_be_bytes()); // weight
        bytes.extend_from_slice(&0u16.to_be_bytes()); // sign
        bytes.extend_from_slice(&0u16.to_be_bytes()); // dscale
        assert!(
            decode_numeric(&bytes).is_err(),
            "truncated numeric should error"
        );
    }

    /// NUMERIC 编解码边界数值往返：超大整数、负数、零、混合小数。
    ///
    /// scale=38 时值 < 1 的整数占位 "0" 不算有效数字，combined 实际
    /// 38 位小数即可往返；scale=38 且含整数部分时（combined > 38 有效位）
    /// 仍会正确报错（见 `decode_numeric` 容量校验）。
    #[test]
    fn numeric_encode_decode_edge_cases() {
        for (mantissa, scale) in [
            (9999_9999_9999_9999_9999i128, 0u32), // 19 位超大整数
            (-1i128, 5u32),                       // -0.00001
            (0i128, 38u32),                       // 最大标度，纯小数可达
            (1i128, 38u32),                       // 1e-38 最小正值
            (-123456789i128, 9u32),               // -0.123456789
        ] {
            let dec = CompactDecimal { mantissa, scale };
            let bytes = encode_numeric(&dec).expect("encode");
            let back = decode_numeric(&bytes).expect("decode");
            assert_eq!(
                (back.mantissa, back.scale),
                (mantissa, scale),
                "edge roundtrip failed for {mantissa}e-{scale}"
            );
        }
    }

    /// 反向断言：scale=38 且含整数部分时，有效位数 > 38，容量校验仍应拒绝。
    ///
    /// 手工构造 `9` + 38 位小数的 NUMERIC 二进制（1 个整数组 + 10 个
    /// 小数分组），占位补零后 combined 达 39 字符，应触发容量校验报错。
    #[test]
    fn decode_numeric_rejects_scale38_with_integer() {
        // ndigits=11：1 个整数组 + 10 个小数分组（10 组 ×4 位 = 40 位小数，
        // 超出的 2 位补零，对齐 dscale=38 后截断）
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&11i16.to_be_bytes()); // ndigits
        bytes.extend_from_slice(&0i16.to_be_bytes()); // weight＝1 个整数组
        bytes.extend_from_slice(&0u16.to_be_bytes()); // sign：正
        bytes.extend_from_slice(&38u16.to_be_bytes()); // dscale
        bytes.extend_from_slice(&9u16.to_be_bytes()); // 首个整数组：9
        for _ in 0..10 {
            bytes.extend_from_slice(&0u16.to_be_bytes()); // 小数分组补零
        }
        assert!(
            decode_numeric(&bytes).is_err(),
            "scale=38 with integer part should exceed capacity"
        );
    }

    /// 绑定方向字段越界：超一天时间、越界时区偏移、越界 CIDR 前缀、
    /// 非法小数、数组绑定均应报错。
    #[test]
    fn to_pg_param_boundary_errors() {
        // Time 超一天（等于一天的纳秒数即非法）
        let bad_time = Value::Time(CompactTime(CompactTime::NANOS_PER_DAY));
        assert!(
            to_pg_param(&bad_time).is_err(),
            "time >= one day should error"
        );

        // TimestampTz 偏移越界（86_400 秒不满足开区间约束）
        let bad_tz = Value::TimestampTz(CompactTimestampTz {
            utc_micros: 0,
            offset_secs: 86_400,
        });
        assert!(
            to_pg_param(&bad_tz).is_err(),
            "timestamp offset out of range should error"
        );

        // CIDR 前缀越界：IPv4 prefix_len>32
        let bad_cidr_v4 = Value::Cidr(CompactCidr {
            addr: "10.0.0.1".parse().expect("addr"),
            prefix_len: 33,
        });
        assert!(
            to_pg_param(&bad_cidr_v4).is_err(),
            "cidr v4 prefix > 32 should error"
        );

        // CIDR 前缀越界：IPv6 prefix_len>128
        let bad_cidr_v6 = Value::Cidr(CompactCidr {
            addr: "2001:db8::1".parse().expect("addr"),
            prefix_len: 129,
        });
        assert!(
            to_pg_param(&bad_cidr_v6).is_err(),
            "cidr v6 prefix > 128 should error"
        );

        // 数组绑定不支持
        let array = Value::Array(vec![Value::I64(1)]);
        assert!(
            to_pg_param(&array).is_err(),
            "array binding should be unsupported"
        );

        // 非法小数：scale>38
        let bad_decimal = Value::Decimal(CompactDecimal {
            mantissa: 1,
            scale: 39,
        });
        assert!(
            to_pg_param(&bad_decimal).is_err(),
            "decimal scale > 38 should error"
        );
    }

    /// u64 绑定：超出 i64 范围映射为 UInt，否则收窄为 Int。
    #[test]
    fn to_pg_param_u64_overflow() {
        let max = to_pg_param(&Value::U64(u64::MAX)).expect("u64::MAX bind");
        assert!(matches!(max, PgParam::UInt(v) if v == u64::MAX));

        let small = to_pg_param(&Value::U64(5)).expect("u64=5 bind");
        assert!(matches!(small, PgParam::Int(5)));
    }

    /// f64 → 小数：有限值可转换，非有限值报错。
    #[test]
    fn f64_to_decimal_handles_nonfinite() {
        let finite = decimal_from_f64(1.5);
        assert!(
            finite.is_ok(),
            "finite float should convert, got {finite:?}"
        );

        assert!(decimal_from_f64(f64::NAN).is_err(), "NaN should error");
        assert!(
            decimal_from_f64(f64::INFINITY).is_err(),
            "infinity should error"
        );
        assert!(
            decimal_from_f64(f64::NEG_INFINITY).is_err(),
            "negative infinity should error"
        );
    }

    /// 十进制字符串解析：空串、非数字、多小数点、超 38 位有效数字均报错。
    #[test]
    fn parse_decimal_str_errors() {
        assert!(parse_decimal_str("").is_err(), "empty should error");
        assert!(parse_decimal_str("abc").is_err(), "letters should error");
        assert!(
            parse_decimal_str("1.2.3").is_err(),
            "multiple dots should error"
        );

        // 39 个 9，超出 38 位有效数字容量
        let too_many = "9".repeat(39);
        assert!(
            parse_decimal_str(&too_many).is_err(),
            "39 digits should exceed capacity"
        );

        // 合法输入仍正常解析
        let ok = parse_decimal_str("-123.45").expect("valid decimal");
        assert_eq!((ok.mantissa, ok.scale), (-12345, 2));
    }
}

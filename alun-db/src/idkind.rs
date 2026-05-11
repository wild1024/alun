use serde_json::Value;

/// 主键 ID 类型 —— 自动识别并格式化数据库 ID 值
///
/// 设计要点：
/// - `detect()` 依据 JSON Value 的内容推断类型
/// - `format_for_db()` 将值转为合适的数据库参数格式
/// - 支持用户通过 `Row::primary_key("uid")` 自定义主键字段名
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdKind {
    Uuid,
    I64,
    F64,
    String,
    Bool,
}

impl IdKind {
    /// 从 JSON Value 自动检测 ID 类型
    ///
    /// 检测优先级：Number(i64) → F64 → Bool → UUID 字符串 → 普通字符串
    pub fn detect(value: &Value) -> Self {
        match value {
            Value::Number(n) => {
                if let Some(_i) = n.as_i64() {
                    IdKind::I64
                } else if let Some(_f) = n.as_f64() {
                    IdKind::F64
                } else {
                    IdKind::String
                }
            }
            Value::String(s) => {
                if is_uuid_format(s) {
                    IdKind::Uuid
                } else {
                    IdKind::String
                }
            }
            Value::Bool(_) => IdKind::Bool,
            _ => IdKind::String,
        }
    }

    /// 是否为 UUID 类型
    pub fn is_uuid(&self) -> bool { matches!(self, IdKind::Uuid) }
    /// 是否为 64 位整数类型
    pub fn is_i64(&self) -> bool { matches!(self, IdKind::I64) }
    /// 是否为字符串类型
    pub fn is_string(&self) -> bool { matches!(self, IdKind::String) }

    /// 主键 WHERE 子句中的 SQL 类型转换后缀
    pub fn id_cast_sql(&self) -> &'static str {
        match self {
            IdKind::Uuid => "::uuid",
            IdKind::I64 => "::bigint",
            _ => "",
        }
    }

    /// INSERT/UPDATE 值占位符中的 SQL 类型转换后缀
    pub fn value_cast_sql(&self) -> &'static str {
        match self {
            IdKind::Uuid => "::uuid",
            IdKind::I64 => "::bigint",
            IdKind::F64 => "::double precision",
            IdKind::Bool => "::boolean",
            _ => "",
        }
    }
}

impl std::fmt::Display for IdKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdKind::Uuid   => write!(f, "uuid"),
            IdKind::I64    => write!(f, "i64"),
            IdKind::F64    => write!(f, "f64"),
            IdKind::String => write!(f, "string"),
            IdKind::Bool   => write!(f, "bool"),
        }
    }
}

fn is_uuid_format(s: &str) -> bool {
    if s.len() != 36 { return false; }
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match i {
            8 | 13 | 18 | 23 => { if b != b'-' { return false; } }
            _ => { if !b.is_ascii_hexdigit() { return false; } }
        }
    }
    true
}

//! `stmt::Type` —— 方言无关的类型抽象。

/// 方言无关的类型。所有 Rust 类型和数据库列类型都映射到这一层。
///
/// ## 使用场景
///
/// - `TypeMapper::stmt_type()` 返回 Rust 类型对应的 `Type`
/// - `Dialect::render_type()` 将 `Type` 渲染为方言特定的列类型字符串
/// - `build.rs` 生成的 schema 快照中，每个列的 `Type` 用于编译期校验
///
/// ## 不变量
///
/// 1. `Nullable` 只出现在最外层，禁止 `Nullable(Nullable(_))` 和
///    `Array(Nullable(_))`。数组内元素是否可空由列定义声明，不在 `Type` 层表达。
/// 2. `Unknown` 只能作为读取列的占位；写入时若遇到 `Unknown` 直接报错。
/// 3. `Enum` 携带原生枚举类型名，其他方言渲染为 `Text` 并附加 CHECK 约束。
/// 4. `Decimal` 不携带精度信息；精度由 `FieldMeta.precision/scale` 或列定义决定。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    // ==================== 布尔 ====================
    /// 布尔值。PostgreSQL 渲染为 `BOOLEAN`，MySQL 为 `TINYINT(1)`，
    /// SQLite 为 `INTEGER`（0/1）。
    Bool,

    // ==================== 有符号整数 ====================
    /// 8 位有符号整数。
    I8,
    /// 16 位有符号整数。
    I16,
    /// 32 位有符号整数。
    I32,
    /// 64 位有符号整数。
    I64,

    // ==================== 无符号整数 ====================
    /// 8 位无符号整数。
    U8,
    /// 16 位无符号整数。
    U16,
    /// 32 位无符号整数。
    U32,
    /// 64 位无符号整数。
    U64,

    // ==================== 浮点数 ====================
    /// 32 位浮点数。
    F32,
    /// 64 位浮点数。
    F64,

    // ==================== 文本与二进制 ====================
    /// 变长文本。
    Text,
    /// 变长二进制。
    Bytes,

    // ==================== 时间 ====================
    /// 无时区日期。
    Date,
    /// 无时区时间（时刻，不含日期）。
    Time,
    /// 无时区时间戳（日期 + 时刻）。
    Timestamp,
    /// 带时区时间戳。存储为 UTC 时间点，读取时保留原时区偏移。
    TimestampTz,

    // ==================== 标识 ====================
    /// 128 位 UUID。
    Uuid,
    /// 128 位 ULID（字典序可排序）。
    Ulid,

    // ==================== 精确数值 ====================
    /// 任意精度十进制数。精度与标度由列定义决定，不在此携带。
    Decimal,

    // ==================== 结构化 ====================
    /// JSON 文档。PostgreSQL 渲染为 `JSONB`，MySQL 为 `JSON`，SQLite 为 `TEXT`。
    Json,

    /// 数据库原生枚举类型。仅 PostgreSQL 使用。
    /// 其他方言渲染为 `Text` 并附加 CHECK 约束。
    Enum(&'static str),

    // ==================== 网络 ====================
    /// IP 地址（不含前缀长度）。
    Inet,
    /// CIDR 网段（IP + 前缀长度）。
    Cidr,

    // ==================== 复合 ====================
    /// 数组类型。PostgreSQL 原生支持，其他方言由 `TypeMapper` 序列化为 JSON。
    Array(Box<Type>),

    /// 可空包装。只允许出现在最外层。
    Nullable(Box<Type>),

    // ==================== 逃逸口 ====================
    /// 未知类型。仅用于读取未知列，写入时遇到即报错。
    Unknown,
}

impl Type {
    /// 将当前类型包裹为可空类型。
    ///
    /// # Panics
    ///
    /// 如果 `self` 已经是 `Nullable`，则 panic（违反不变量）。
    pub fn nullable(self) -> Type {
        assert!(!self.is_nullable(), "禁止 Nullable(Nullable(_))");
        Type::Nullable(Box::new(self))
    }

    /// 判断当前类型是否为可空。
    pub fn is_nullable(&self) -> bool {
        matches!(self, Type::Nullable(_))
    }

    /// 去除最外层的 `Nullable`，返回内部类型。
    ///
    /// 若 `self` 不是 `Nullable`，返回自身引用。
    pub fn inner(&self) -> &Type {
        match self {
            Type::Nullable(t) => t,
            other => other,
        }
    }

    /// 返回类型的可读名称，用于错误消息。
    pub fn name(&self) -> &'static str {
        match self {
            Type::Bool => "Bool",
            Type::I8 => "I8",
            Type::I16 => "I16",
            Type::I32 => "I32",
            Type::I64 => "I64",
            Type::U8 => "U8",
            Type::U16 => "U16",
            Type::U32 => "U32",
            Type::U64 => "U64",
            Type::F32 => "F32",
            Type::F64 => "F64",
            Type::Text => "Text",
            Type::Bytes => "Bytes",
            Type::Date => "Date",
            Type::Time => "Time",
            Type::Timestamp => "Timestamp",
            Type::TimestampTz => "TimestampTz",
            Type::Uuid => "Uuid",
            Type::Ulid => "Ulid",
            Type::Decimal => "Decimal",
            Type::Json => "Json",
            Type::Enum(_) => "Enum",
            Type::Inet => "Inet",
            Type::Cidr => "Cidr",
            Type::Array(_) => "Array",
            Type::Nullable(_) => "Nullable",
            Type::Unknown => "Unknown",
        }
    }
}

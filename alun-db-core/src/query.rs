//! `SqlQuery` —— `sql!` 宏的产物：SQL 文本与绑定参数的载体。
//!
//! ## 设计
//!
//! `sql!` 宏在编译期拼接静态片段，所有插值一律转为占位符 `?` + 绑定参数，
//! 值永不内联进 SQL 文本。执行时由 facade 层按方言改写占位符
//! （PostgreSQL 的 `$n`）并分发给底层连接。
//!
//! ```text
//! sql!("WHERE a = ", { x })  ──▶  SqlQuery { sql: "WHERE a = ?",
//!                                            params: [Value::I64(x)] }
//! ```

use crate::stmt::Value;

/// `sql!` 宏生成的查询载体。
///
/// 占位符统一为 `?`；参数顺序与占位符出现顺序一致。
#[derive(Debug, Clone, Default)]
pub struct SqlQuery {
    /// SQL 文本，参数占位符统一为 `?`。
    sql: String,
    /// 按占位符出现顺序排列的绑定参数。
    params: Vec<Value>,
}

impl SqlQuery {
    /// 构造查询。由 `sql!` 宏展开使用。
    pub fn new(sql: String, params: Vec<Value>) -> Self {
        SqlQuery { sql, params }
    }

    /// SQL 文本（占位符为 `?` 风格）。
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// 绑定参数列表。
    pub fn params(&self) -> &[Value] {
        &self.params
    }

    /// 消耗自身，返回 SQL 文本与参数。
    pub fn into_parts(self) -> (String, Vec<Value>) {
        (self.sql, self.params)
    }
}

/// 可拼入 SQL 片段流的类型，用于 `sql!` 宏的嵌套组合。
///
/// 实现方负责同时维护 SQL 文本与参数列表，保证占位符与参数对齐。
pub trait SqlFragment {
    /// 追加自身到目标 SQL 文本与参数列表。
    fn push_into(self, sql: &mut String, params: &mut Vec<Value>);
}

impl SqlFragment for SqlQuery {
    fn push_into(self, sql: &mut String, params: &mut Vec<Value>) {
        sql.push_str(&self.sql);
        params.extend(self.params);
    }
}

/// 单向值转换：`sql!` 宏插值的绑定协议。
///
/// 相比 [`TypeMapper`](crate::traits::TypeMapper) 的双向契约，本 trait
/// 只做 Rust → `Value` 方向，因此可以额外覆盖 `&str` 等无法实现
/// `TypeMapper`（`from_value` 需要拥有所有权）的借用类型。
pub trait IntoValue {
    /// 转为绑定参数值。
    fn into_value(self) -> Value;
}

impl<T: crate::traits::TypeMapper> IntoValue for T {
    fn into_value(self) -> Value {
        crate::traits::TypeMapper::to_value(&self)
    }
}

impl IntoValue for &str {
    fn into_value(self) -> Value {
        Value::Text(self.to_string())
    }
}

impl IntoValue for &String {
    fn into_value(self) -> Value {
        Value::Text(self.clone())
    }
}

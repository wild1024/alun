//! `FieldExpr<T>` —— 类型化字段表达式。

use crate::expr::{BinaryOp, Expr};
use crate::statement::{ColumnRef, Order, OrderBy};
use crate::stmt::Value;
use crate::traits::TypeMapper;
use std::marker::PhantomData;

// ==================== FieldExpr ====================

/// 类型化字段表达式。由 `#[derive(Model)]` 为每个字段生成访问器。
///
/// ## 类型安全
///
/// `FieldExpr<T>` 携带字段的 Rust 类型 `T`。比较操作 `eq`、`gt` 等
/// 通过 `FieldValue<T>` trait 约束参数类型，编译期拒绝类型不匹配：
///
/// ```ignore
/// User::age().eq(18)        // ✅ 编译通过
/// User::age().eq("18")      // ❌ 编译失败
/// ```
pub struct FieldExpr<T> {
    /// 表名。
    table: &'static str,
    /// 列名。
    column: &'static str,
    /// 标记字段的 Rust 类型，不占用运行时空间。
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for FieldExpr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for FieldExpr<T> {}

impl<T> FieldExpr<T> {
    /// 构造字段表达式。仅供宏展开使用。
    pub const fn new(table: &'static str, column: &'static str) -> Self {
        FieldExpr {
            table,
            column,
            _marker: PhantomData,
        }
    }

    /// 返回列名。
    pub fn column(&self) -> &'static str {
        self.column
    }

    /// 返回表名。
    pub fn table(&self) -> &'static str {
        self.table
    }

    /// 转为 `Expr::Column` 节点。
    pub fn to_expr(self) -> Expr {
        Expr::Column {
            table: self.table,
            column: self.column,
        }
    }
}

// ==================== 比较操作（T: TypeMapper）====================

impl<T: TypeMapper> FieldExpr<T> {
    /// 等于比较。
    pub fn eq<U: FieldValue<T>>(self, v: U) -> Expr {
        binary(self.to_expr(), BinaryOp::Eq, v.into_value())
    }

    /// 不等于比较。
    pub fn ne<U: FieldValue<T>>(self, v: U) -> Expr {
        binary(self.to_expr(), BinaryOp::Ne, v.into_value())
    }

    /// 大于比较。
    pub fn gt<U: FieldValue<T>>(self, v: U) -> Expr {
        binary(self.to_expr(), BinaryOp::Gt, v.into_value())
    }

    /// 大于等于比较。
    pub fn ge<U: FieldValue<T>>(self, v: U) -> Expr {
        binary(self.to_expr(), BinaryOp::Ge, v.into_value())
    }

    /// 小于比较。
    pub fn lt<U: FieldValue<T>>(self, v: U) -> Expr {
        binary(self.to_expr(), BinaryOp::Lt, v.into_value())
    }

    /// 小于等于比较。
    pub fn le<U: FieldValue<T>>(self, v: U) -> Expr {
        binary(self.to_expr(), BinaryOp::Le, v.into_value())
    }

    /// 是否为空。
    pub fn is_null(self) -> Expr {
        binary(self.to_expr(), BinaryOp::IsNull, Value::Null)
    }

    /// 是否非空。
    pub fn is_not_null(self) -> Expr {
        binary(self.to_expr(), BinaryOp::IsNotNull, Value::Null)
    }

    /// 区间比较 `[lo, hi]`。
    pub fn between<U: FieldValue<T>>(self, lo: U, hi: U) -> Expr {
        Expr::and_all([
            binary(self.to_expr(), BinaryOp::Ge, lo.into_value()),
            binary(self.to_expr(), BinaryOp::Le, hi.into_value()),
        ])
    }

    /// 升序排序。
    pub fn asc(self) -> OrderBy {
        OrderBy {
            expr: ColumnRef::Column(self.column),
            direction: Order::Asc,
            nulls: None,
        }
    }

    /// 降序排序。
    pub fn desc(self) -> OrderBy {
        OrderBy {
            expr: ColumnRef::Column(self.column),
            direction: Order::Desc,
            nulls: None,
        }
    }
}

// ==================== 字符串特有操作 ====================

impl FieldExpr<String> {
    /// LIKE 模糊匹配。
    pub fn like(self, pat: impl Into<String>) -> Expr {
        binary(self.to_expr(), BinaryOp::Like, Value::Text(pat.into()))
    }

    /// NOT LIKE 否定匹配。
    pub fn not_like(self, pat: impl Into<String>) -> Expr {
        binary(self.to_expr(), BinaryOp::NotLike, Value::Text(pat.into()))
    }

    /// ILIKE 大小写不敏感匹配。
    ///
    /// 方言不支持时由渲染器决定降级或报错。
    pub fn ilike(self, pat: impl Into<String>) -> Expr {
        binary(self.to_expr(), BinaryOp::ILike, Value::Text(pat.into()))
    }

    /// 属于集合。
    pub fn in_<I, S>(self, items: I) -> Expr
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let arr: Vec<Value> = items.into_iter().map(|s| Value::Text(s.into())).collect();
        binary(self.to_expr(), BinaryOp::In, Value::Array(arr))
    }
}

// ==================== 数值特有操作 ====================

// `add`/`sub`/`mul`/`div` 是构建 SQL 表达式的公开 API 方法（返回 `Expr`，
// 而非实现对应的 `std::ops` trait），方法名与运算符 trait 同名属有意设计，
// 此处保留方法语义并抑制该 lint。
#[allow(clippy::should_implement_trait)]
impl<T: TypeMapper> FieldExpr<T> {
    /// 加法。
    pub fn add<U: FieldValue<T>>(self, v: U) -> Expr {
        binary(self.to_expr(), BinaryOp::Add, v.into_value())
    }
    /// 减法。
    pub fn sub<U: FieldValue<T>>(self, v: U) -> Expr {
        binary(self.to_expr(), BinaryOp::Sub, v.into_value())
    }
    /// 乘法。
    pub fn mul<U: FieldValue<T>>(self, v: U) -> Expr {
        binary(self.to_expr(), BinaryOp::Mul, v.into_value())
    }
    /// 除法。
    pub fn div<U: FieldValue<T>>(self, v: U) -> Expr {
        binary(self.to_expr(), BinaryOp::Div, v.into_value())
    }
}

/// 内部辅助：构造二元表达式。
fn binary(left: Expr, op: BinaryOp, right: Value) -> Expr {
    Expr::Binary {
        left: Box::new(left),
        op,
        right: Box::new(Expr::Value(right)),
    }
}

// ==================== FieldValue ====================

/// 允许将某种 Rust 值嵌入到 `FieldExpr<T>` 中。
///
/// 当 `T: TypeMapper` 时，自动实现 `FieldValue<T> for T`。
/// 同时为常用的放宽类型（如 `&str` for `String`）提供实现。
pub trait FieldValue<T> {
    /// 将值转为 `stmt::Value`。
    fn into_value(self) -> Value;
}

impl<T: TypeMapper> FieldValue<T> for T {
    fn into_value(self) -> Value {
        self.to_value()
    }
}

impl FieldValue<String> for &str {
    fn into_value(self) -> Value {
        Value::Text(self.to_string())
    }
}

impl<T: TypeMapper> FieldValue<Option<T>> for T {
    fn into_value(self) -> Value {
        self.to_value()
    }
}

// ==================== FieldMeta ====================

/// 字段元数据。由 `#[derive(Model)]` 生成，用于 schema 校验和行映射。
#[derive(Debug, Clone)]
pub struct FieldMeta {
    /// Rust 字段名。
    pub name: &'static str,
    /// 数据库列名（可被 `#[column("...")]` 覆盖）。
    pub column: &'static str,
    /// 中间类型。
    pub ty: crate::stmt::Type,
    /// 是否可空。
    pub nullable: bool,
    /// 是否为主键。
    pub is_key: bool,
    /// 是否自增。
    pub is_auto: bool,
    /// 是否唯一。
    pub is_unique: bool,
    /// 十进制精度。仅 `Type::Decimal` 时有效。
    pub precision: Option<u32>,
    /// 十进制标度。仅 `Type::Decimal` 时有效。
    pub scale: Option<u32>,
}

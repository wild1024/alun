//! # alun-db-core
//!
//! `alun-db` 的核心类型与 trait 定义层。
//!
//! ## 设计约束
//!
//! 本 crate 遵守以下三条硬性约束，任何新增代码都必须满足：
//!
//! 1. **零异步依赖**：不引入 tokio、async-std、futures 等运行时。异步执行
//!    抽象由 facade 层定义（dyn-safe 的内部 trait），core 只承载纯数据结构。
//! 2. **零驱动依赖**：不引入 sqlx、rusqlite、tokio-postgres 等驱动。所有
//!    驱动通过 `Driver` / `Dialect` trait 与本 crate 对接。
//! 3. **零生态类型依赖**：chrono、rust_decimal、uuid 等生态类型均通过
//!    Cargo feature 按需启用，且只在 `TypeMapper` 实现层出现，不影响
//!    核心类型（`stmt::Type` / `stmt::Value`）的定义。
//!
//! ## 模块组织
//!
//! - [`stmt`]      —— 中间类型（`Type`）、运行时值（`Value`）、紧凑表示（`compact`）
//! - [`row`]       —— 动态行视图
//! - [`expr`]      —— 表达式树
//! - [`statement`] —— 语句 AST
//! - [`field`]     —— 类型化字段表达式
//! - [`traits`]    —— 所有核心 trait
//! - [`hook`]      —— 内置 Hook（`TimestampHook`）与洋葱模型执行器
//! - [`error`]     —— 统一错误类型
//! - [`page`]      —— 分页容器
//! - [`query`]     —— `sql!` 宏产物（`SqlQuery`）与片段组合（`SqlFragment`）

#![deny(missing_docs)]
#![deny(unsafe_code)]
#![warn(clippy::all)]

pub mod error;
pub mod expr;
pub mod field;
pub mod hook;
#[cfg(feature = "metrics")]
pub mod metrics;
pub mod page;
pub mod query;
pub mod row;
pub mod statement;
pub mod stmt;
pub mod traits;

/// 重导出 `async_trait` 属性宏：实现 [`traits::AsyncHook`] 时写
/// `#[alun_db_core::async_trait]`，无需自行依赖 `async-trait`。
pub use async_trait::async_trait;

// ---- 常用导出，便于 `use alun_db_core::prelude::*` ----

/// 常用类型的便捷重导出。
pub mod prelude {
    pub use crate::error::{DbError, Result};
    pub use crate::expr::{BinaryOp, Expr, Function, UnaryOp};
    pub use crate::field::{FieldExpr, FieldMeta, FieldValue};
    pub use crate::hook::TimestampHook;
    pub use crate::page::{Cursor, Page};
    pub use crate::query::{IntoValue, SqlFragment, SqlQuery};
    pub use crate::row::Row;
    pub use crate::statement::{
        ColumnRef, ConflictAction, DeleteStmt, InsertStmt, Join, JoinKind, NullsOrder, OnConflict,
        Order, OrderBy, RawStmt, ScopeFilter, ScopeMode, SelectStmt, Statement, UpdateStmt,
    };
    pub use crate::stmt::{Type, Value};
    pub use crate::traits::{
        AsyncHook, Dialect, DialectCapability, Driver, ExecuteResult, Hook, Model, Params,
        Placeholder, PoolConfig, RawDbError, RenderCtx, RenderedSql, Rows, TypeMapper,
    };

    #[cfg(feature = "chrono")]
    pub use chrono;
    #[cfg(feature = "rust_decimal")]
    pub use rust_decimal;
    #[cfg(feature = "json")]
    pub use serde_json;
    #[cfg(feature = "uuid")]
    pub use uuid;
}

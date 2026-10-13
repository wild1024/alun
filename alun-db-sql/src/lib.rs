//! # alun-db-sql
//!
//! `alun-db` 的 SQL 渲染器与方言实现。
//!
//! ## 架构
//!
//! ```text
//! Expr / Statement (来自 alun-db-core)
//!        │
//!        ▼
//!   Dialect::render_expr / render_statement
//!        │
//!        ├── SqliteDialect   ──▶ "SELECT * FROM users WHERE id = ?"
//!        ├── PostgresDialect ──▶ "SELECT * FROM users WHERE id = $1"
//!        └── MysqlDialect    ──▶ "SELECT * FROM users WHERE id = ?"
//!        │
//!        ▼
//!   RenderedSql { sql, params }
//! ```
//!
//! ## 设计原则
//!
//! 1. **渲染器无状态**：`Dialect` 是单例，`RenderCtx` 持有可变状态
//! 2. **参数与 SQL 分离**：`RenderedSql` 始终返回 `(String, Vec<Value>)`
//! 3. **方言差异集中**：所有方言差异在 `Dialect` 实现中处理，不泄漏到 core
//! 4. **占位符统一管理**：`RenderCtx::bind` 负责生成 `?` 或 `$n`

#![deny(missing_docs)]
#![warn(clippy::all)]

pub mod dialects;
pub mod placeholder;
pub mod render;

pub use dialects::{MysqlDialect, PostgresDialect, SqliteDialect};
pub use placeholder::{rewrite_placeholders, rewrite_placeholders_owned};

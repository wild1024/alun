//! 中间类型层（Statement Type Layer）。
//!
//! 本模块定义了 `alun-db` 的**类型解耦核心**：
//!
//! ```text
//! Rust 类型  ──[TypeMapper]──▶  stmt::Value  ──[Dialect]──▶  数据库值
//!    │                              │                            │
//!    └──[TypeMapper::stmt_type]──▶  stmt::Type  ──[Dialect]──▶  数据库列类型
//! ```
//!
//! `stmt::Type` 是**方言无关的**类型抽象。所有 Rust 类型和数据库列类型
//! 都先映射到这一层，再由方言渲染器转换为具体的列类型字符串。这样做
//! 的好处是：Rust 类型 ↔ 中间类型 的映射只需定义一次（全局固定），
//! 中间类型 ↔ 数据库类型 的映射由每个方言独立负责。

pub mod compact;
pub mod ty;
pub mod value;

pub use ty::Type;
pub use value::Value;

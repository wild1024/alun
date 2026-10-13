//! 核心 trait 定义。
//!
//! ## trait 关系
//!
//! ```text
//! Model ──▶ 由 facade 层的 CrudExt / PersistExt 提供关联函数与实例方法
//!
//! TypeMapper ──▶ 内置类型的值转换
//!
//! Dialect ──▶ SQL 渲染与错误映射
//! Driver  ──▶ 连接管理与方言提供
//!
//! Hook ──▶ Model 生命周期拦截
//!
//! 执行抽象（dyn-safe 的 RawExecutor / RawTransaction）由 facade 层定义，
//! core 层不承载执行 trait，避免引入异步运行时依赖。
//! ```

pub mod dialect;
pub mod driver;
pub mod executor;
pub mod hook;
pub mod model;
pub mod params;
pub mod type_mapper;

pub use dialect::{Dialect, DialectCapability, Placeholder, RawDbError, RenderCtx, RenderedSql};
pub use driver::{Driver, PoolConfig};
pub use executor::{ExecuteResult, Rows};
pub use hook::{AsyncHook, Hook};
pub use model::Model;
pub use params::Params;
#[cfg(feature = "rust_decimal")]
pub use type_mapper::decimal_from_compact;
pub use type_mapper::{TypeMapper, int_discriminant};

//! alun-core：应用生命周期、插件系统、统一错误、API 类型
pub mod error;
pub mod plugin;
pub mod api;

pub use error::{Error, Result};
pub use plugin::{Plugin, PluginManager};
pub use api::{Res, ResResult, ApiError, PageData, PageQuery, codes};

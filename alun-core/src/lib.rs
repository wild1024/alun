//! alun-core：应用生命周期、插件系统、统一错误
//!
//! 这里是**纯骨架层**：只管生命周期、插件与统一错误，**零 HTTP 概念、零数据库概念**，
//! 因此可以被非 Web / 非数据库的项目单独复用。
//!
//! 响应形状类型（`Res` / `ApiError` / `ResResult` / `PageQuery` / `PageData` / `codes`）
//! 与「下层错误 → HTTP 语义」的映射都住在 [`alun_web::api`](https://docs.rs/alun-web)，
//! 经 `alun` 门面的 prelude 原样导出，业务端路径不受影响。
pub mod error;
pub mod plugin;

pub use error::{Error, Result};
pub use plugin::{Plugin, PluginManager};

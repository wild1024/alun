//! 标准响应类型（re-export from alun-core）

pub use alun_core::api::{codes, Res, ApiError, PageData, PageQuery, ResResult};

#[deprecated(since = "0.2.0", note = "请使用 `Res` 代替 `ApiBody`")]
pub type ApiBody<T = ()> = Res<T>;

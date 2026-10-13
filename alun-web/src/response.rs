//! 标准响应类型（re-export from [`crate::api`]）

pub use crate::api::{
    ApiError, DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, PageData, PageQuery, Res, ResResult, codes,
};

#[deprecated(since = "0.2.0", note = "请使用 `Res` 代替 `ApiBody`")]
pub type ApiBody<T = ()> = Res<T>;

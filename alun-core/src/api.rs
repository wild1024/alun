//! 框架核心 API 类型：响应体、错误、分页数据、错误码
//!
//! 这些类型是框架的公共"语言"，所有 crate 都依赖它们。
//! `IntoResponse` 实现通过 `features = ["web"]` 开启。

use crate::Error;
use serde::Serialize;

#[cfg(feature = "axum")]
use axum::{
    response::{IntoResponse, Json},
    http::StatusCode,
};

// ──── 错误码常量 ────────────────────────────────────

pub mod codes {
    pub const OK: i32 = 0;
    pub const BAD_REQUEST: i32 = 400;
    pub const UNAUTHORIZED: i32 = 401;
    pub const FORBIDDEN: i32 = 403;
    pub const NOT_FOUND: i32 = 404;
    pub const METHOD_NOT_ALLOWED: i32 = 405;
    pub const CONFLICT: i32 = 409;
    pub const UNPROCESSABLE_ENTITY: i32 = 422;
    pub const TOO_MANY_REQUESTS: i32 = 429;
    pub const INTERNAL: i32 = 500;
    pub const SERVICE_UNAVAILABLE: i32 = 503;
}

// ──── 统一响应体 ─────────────────────────────────────

/// 统一 API 响应结构
#[derive(Debug, Clone, Serialize)]
pub struct Res<T: Serialize = ()> {
    /// 业务码，0 表示成功
    pub code: i32,
    /// 提示信息
    pub msg: String,
    /// 数据载荷
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

/// 分页数据结构
#[derive(Debug, Clone, Serialize)]
pub struct PageData<T: Serialize> {
    /// 数据列表
    pub list: T,
    /// 总条数
    pub total: u64,
    /// 当前页码
    pub page: u64,
    /// 每页条数
    pub page_size: u64,
}

/// API 响应结果类型
pub type ResResult<T> = std::result::Result<Res<T>, ApiError>;

// ──── 分页参数 ──────────────────────────────────────

/// 分页查询参数（公共类型，所有 crate 可用）
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PageQuery {
    /// 页码（从 1 开始）
    pub page: u64,
    /// 每页条数
    pub page_size: u64,
}

impl PageQuery {
    /// 创建分页参数，自动规整到合法范围
    ///
    /// - `page`: 页码，最小为 1
    /// - `page_size`: 每页条数，范围 [1, 1000]
    ///
    /// 超出范围的值会被自动修正到边界。
    pub fn new(page: u64, page_size: u64) -> Self {
        let page = if page < 1 { 1 } else { page };
        let page_size = if page_size < 1 { 10 } else if page_size > 1000 { 1000 } else { page_size };
        Self { page, page_size }
    }

    /// 计算 SQL OFFSET：`(page - 1) * page_size`
    pub fn offset(&self) -> u64 { (self.page - 1) * self.page_size }
    /// 获取 LIMIT 值（即 `page_size`）
    pub fn limit(&self) -> u64 { self.page_size }
}

// ──── Res 实现 ──────────────────────────────────────

impl Res<()> {
    /// 成功（无数据载荷），返回 `{code: 0, msg: "ok", data: null}`
    pub fn ok_empty() -> Self {
        Self { code: codes::OK, msg: "ok".into(), data: None }
    }

    /// 成功（自定义消息，无数据载荷）
    pub fn ok_msg(msg: impl Into<String>) -> Self {
        Self { code: codes::OK, msg: msg.into(), data: None }
    }
}

impl<T: Serialize> Res<T> {
    /// 成功响应，携带数据载荷
    ///
    /// # 示例
    ///
    /// ```ignore
    /// Res::ok(user)          // => {code: 0, msg: "ok", data: user}
    /// Res::ok("hello")       // => {code: 0, msg: "ok", data: "hello"}
    /// ```
    pub fn ok(data: T) -> Self {
        Self { code: codes::OK, msg: "ok".into(), data: Some(data) }
    }

    /// 成功响应，携带数据载荷和自定义消息
    pub fn ok_with_msg(data: T, msg: impl Into<String>) -> Self {
        Self { code: codes::OK, msg: msg.into(), data: Some(data) }
    }

    /// 失败响应（自定义错误码和消息，无数据载荷）
    ///
    /// # 示例
    ///
    /// ```ignore
    /// Res::fail(codes::BAD_REQUEST, "用户名不能为空")
    /// ```
    pub fn fail(code: i32, msg: impl Into<String>) -> Self {
        Self { code, msg: msg.into(), data: None }
    }
}

impl<T: Serialize> Res<PageData<T>> {
    /// 分页响应
    pub fn page(list: T, total: u64, page: u64, page_size: u64) -> Self {
        Self::ok(PageData { list, total, page, page_size })
    }
}

// ──── API 错误 ──────────────────────────────────────

/// API 错误（对外暴露的统一错误类型）
///
/// HTTP 状态码使用 u16 存储，与 Web 框架解耦。
#[derive(Debug)]
pub struct ApiError {
    /// 业务码
    pub code: i32,
    /// 对外消息（已脱敏，不泄露内部信息）
    pub msg: String,
    /// HTTP 状态码（u16，与 axum::StatusCode 互转）
    pub status: u16,
    /// 内部调试信息（仅日志记录，不返回前端）
    pub internal_detail: Option<String>,
}

impl ApiError {
    /// 创建 API 错误
    ///
    /// - `status`: HTTP 状态码（如 400、401、500）
    /// - `code`: 业务错误码
    /// - `msg`: 对外提示消息
    pub fn new(status: u16, code: i32, msg: impl Into<String>) -> Self {
        Self { status, code, msg: msg.into(), internal_detail: None }
    }

    /// 附加内部调试详情（仅写入日志，不暴露给前端）
    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.internal_detail = Some(detail.into());
        self
    }

    // ── 工厂方法 ──

    /// 400 Bad Request：客户端请求格式或参数错误
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(400, codes::BAD_REQUEST, msg)
    }

    /// 401 Unauthorized：未认证或 Token 无效
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::new(401, codes::UNAUTHORIZED, msg)
    }

    /// 403 Forbidden：已认证但权限不足
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::new(403, codes::FORBIDDEN, msg)
    }

    /// 404 Not Found：资源不存在
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(404, codes::NOT_FOUND, msg)
    }

    /// 405 Method Not Allowed：HTTP 方法不正确
    pub fn method_not_allowed(msg: impl Into<String>) -> Self {
        Self::new(405, codes::METHOD_NOT_ALLOWED, msg)
    }

    /// 409 Conflict：资源冲突（如重复创建）
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::new(409, codes::CONFLICT, msg)
    }

    /// 422 Unprocessable Entity：请求体语义错误（如字段校验失败）
    pub fn unprocessable_entity(msg: impl Into<String>) -> Self {
        Self::new(422, codes::UNPROCESSABLE_ENTITY, msg)
    }

    /// 429 Too Many Requests：请求频率超限
    pub fn too_many_requests(msg: impl Into<String>) -> Self {
        Self::new(429, codes::TOO_MANY_REQUESTS, msg)
    }

    /// 500 Internal Server Error：服务端内部错误
    ///
    /// 前端仅看到模糊提示；完整错误需通过日志排查。
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(500, codes::INTERNAL, msg)
    }

    /// 500 Internal Server Error（带调试详情）
    ///
    /// - `public_msg`: 返回前端的模糊提示
    /// - `detail`: 内部日志记录的详细信息（如完整的错误栈）
    pub fn internal_masked(public_msg: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(500, codes::INTERNAL, public_msg)
            .with_detail(detail)
    }

    /// 503 Service Unavailable：服务暂时不可用
    pub fn service_unavailable(msg: impl Into<String>) -> Self {
        Self::new(503, codes::SERVICE_UNAVAILABLE, msg)
    }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        ApiError::internal_masked("服务器内部错误", e.to_string())
    }
}

// ──── IntoResponse 实现（需 web feature） ───────────

#[cfg(feature = "axum")]
impl<T: Serialize> IntoResponse for Res<T> {
    fn into_response(self) -> axum::response::Response {
        let mut resp = Json(self).into_response();
        resp.headers_mut().insert(
            axum::http::HeaderName::from_static("content-type"),
            axum::http::HeaderValue::from_static("application/json; charset=utf-8"),
        );
        resp
    }
}

#[cfg(feature = "axum")]
impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        if let Some(ref detail) = self.internal_detail {
            let status = StatusCode::from_u16(self.status)
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            tracing::error!(status = status.as_u16(), code = self.code, detail = %detail,
                "请求处理异常");
        }
        let body = Res::<()>::fail(self.code, self.msg);
        let status = StatusCode::from_u16(self.status)
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut resp = (status, Json(body)).into_response();
        resp.headers_mut().insert(
            axum::http::HeaderName::from_static("content-type"),
            axum::http::HeaderValue::from_static("application/json; charset=utf-8"),
        );
        resp
    }
}



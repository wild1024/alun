//! 统一出口：响应体、API 错误、分页参数、错误码
//!
//! # 业务端只需要记住三个名字
//!
//! | 你要做的事 | 写法 |
//! |---|---|
//! | handler 签名 | `-> ResResult<T>` |
//! | 成功 | `Ok(Res::ok(data))` |
//! | 业务报错 | `Err(ApiError::not_found("用户不存在"))` |
//! | 数据库 / 框架 / IO 调用 | 直接 `?`，映射全自动 |
//!
//! 其余错误类型（`DbError`、`alun_core::Error`、`std::io::Error`）都只是 `?` 的
//! **过路输入**——业务代码永远不需要把它们写在签名里。
//!
//! # 为什么这些类型住在 `alun-web` 而不是 `alun-core`
//!
//! `Res` / `ApiError` / `PageQuery` 满身都是 HTTP 概念（`status: u16`、404 / 409…），
//! 放在号称「可被非 Web 项目复用」的 `alun-core` 里本身就是分层瑕疵。更实际的是
//! **孤儿规则**：`impl From<DbError> for X` 只能写在定义 `DbError`（`alun-db-core`）
//! 或定义 `X` 的 crate 里。让 `ApiError` 成为 `alun-web` 的**本地类型**之后，
//! `From<DbError> for ApiError` 直接合法 —— 不需要新造一个 `WebError` 当落点，
//! 也不需要让 `alun-core` 依赖数据库内核。
//!
//! 于是 `alun-core` 保持零 HTTP 概念（纯生命周期 / 插件 / 错误），
//! `alun-db-core` 保持零框架依赖，所有跨层转换集中在同时认识两边的 `alun-web`
//! （与 `page.rs` 桥接 `Page` ↔ `PageData` 是同一惯例）。

use alun_core::Error as CoreError;
use serde::{Deserialize, Deserializer, Serialize};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Json},
};

#[cfg(feature = "db")]
use alun_db::__db_core::prelude::DbError;

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
///
/// 注意与数据库层的分页容器 `alun_db::Page<T>` 区分：后者字段为
/// `list / total / page / size / pages`。二者的转换由 `alun-web` 的
/// `PageResponseExt::into_page_data()` / `into_res()` 提供。
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

impl<T: Serialize> PageData<T> {
    /// 创建分页数据
    pub fn new(list: T, total: u64, page: u64, page_size: u64) -> Self {
        Self {
            list,
            total,
            page,
            page_size,
        }
    }
}

/// handler 的**唯一**返回类型：`Result<Res<T>, ApiError>`
pub type ResResult<T> = std::result::Result<Res<T>, ApiError>;

// ──── 分页参数 ──────────────────────────────────────

/// 每页条数默认值：`page_size` 缺失或小于 1 时使用
pub const DEFAULT_PAGE_SIZE: u64 = 10;

/// 每页条数上限：请求值大于该值时被规整
pub const MAX_PAGE_SIZE: u64 = 1000;

/// 分页查询参数（公共类型，全框架唯一来源）
///
/// Web 侧与数据库侧共用本类型，不再各自维护一份：
/// - 请求侧：直接作为 axum 的 `Query` 提取器（见下）
/// - 数据库侧：`page` / `page_size` 直接传给 `alun_db` 的分页方法
///
/// 可直接作为 axum 的 `Query` 提取器使用：`page` / `page_size` 缺失、为 `0`
/// 或超出合法范围时，统一规整为 `page = 1`、`page_size` 落在
/// 1 到 [`MAX_PAGE_SIZE`] 之间（缺失时为 [`DEFAULT_PAGE_SIZE`]）。
/// 因此 [`offset`](Self::offset) / [`limit`](Self::limit) 对任何来源的取值都是安全的，
/// 不会因 `page = 0` 造成 `u64` 下溢。
///
/// 与数据库层对接时把两个字段直接传给 `alun_db`：
///
/// ```ignore
/// let page = db().paginate("SELECT * FROM users ORDER BY id", pq.page, pq.page_size).await?;
/// ```
#[derive(Debug, Clone, Copy)]
pub struct PageQuery {
    /// 页码（从 1 开始）
    pub page: u64,
    /// 每页条数（1 到 [`MAX_PAGE_SIZE`] 之间）
    pub page_size: u64,
}

impl PageQuery {
    /// 创建分页参数，自动规整到合法范围
    ///
    /// - `page`: 页码，最小为 1
    /// - `page_size`: 每页条数，范围 1 到 [`MAX_PAGE_SIZE`]
    ///
    /// 超出范围的值会被自动修正到边界。
    pub fn new(page: u64, page_size: u64) -> Self {
        let page = page.max(1);
        let page_size = if page_size < 1 {
            DEFAULT_PAGE_SIZE
        } else {
            page_size.min(MAX_PAGE_SIZE)
        };
        Self { page, page_size }
    }

    /// 返回规整后的副本
    ///
    /// 直接构造（结构体字面量、反序列化以外的来源）可能得到越界值，本方法兜底。
    pub fn normalized(&self) -> Self {
        Self::new(self.page, self.page_size)
    }

    /// 计算 SQL OFFSET：`(page - 1) * page_size`
    ///
    /// 越界值先规整；极端 `page` 乘法饱和，不 panic。
    pub fn offset(&self) -> u64 {
        let n = self.normalized();
        (n.page - 1).saturating_mul(n.page_size)
    }

    /// 获取 LIMIT 值（即规整后的 `page_size`）
    pub fn limit(&self) -> u64 {
        self.normalized().page_size
    }

    /// 计算总页数：`ceil(total / page_size)`；`total == 0` 时为 0
    ///
    /// 口径与 `alun_db::Page::calc_pages` 一致。
    pub fn calc_pages(&self, total: u64) -> u64 {
        let size = self.limit();
        if total == 0 || size == 0 {
            0
        } else {
            total.div_ceil(size)
        }
    }
}

impl<'de> Deserialize<'de> for PageQuery {
    /// 反序列化即规整：`?page=0&page_size=0`、缺省参数均落到合法范围，
    /// 避免绕过 [`PageQuery::new`] 的校验。
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(default)]
            page: u64,
            #[serde(default)]
            page_size: u64,
        }

        let raw = Raw::deserialize(deserializer)?;
        Ok(PageQuery::new(raw.page, raw.page_size))
    }
}

// ──── 排序参数 ──────────────────────────────────────

/// 排序方向。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    /// 升序（默认）
    #[default]
    Asc,
    /// 降序
    Desc,
}

/// 通用排序参数（与 [`PageQuery`] 配套使用）。
///
/// 之所以单独成类型而不并入 `PageQuery`：并非所有列表接口都需要排序，
/// 且分页参数常被单独复用。
///
/// # 安全：排序字段必须走白名单
///
/// `ORDER BY` 是 SQL 注入的经典入口（列名无法用绑定参数表达）。因此本类型
/// **只负责承载用户输入**，实际校验由
/// `alun_db::ModelQuery::order_by_whitelisted` 完成——字段不在白名单内会
/// 返回错误，而非静默忽略。
///
/// # 示例
///
/// ```ignore
/// use alun::prelude::{PageQuery, SortQuery};
///
/// #[derive(Deserialize)]
/// struct ListQuery { page: PageQuery, sort: SortQuery }
///
/// const SORTABLE: &[&str] = &["id", "created_at", "name"];
///
/// async fn list(Query(q): Query<ListQuery>) -> Res<Page<User>> {
///     let rows = User::query()
///         .order_by_whitelisted(q.sort.sort_by.as_deref(), q.sort.is_desc(), SORTABLE)?
///         .paginate(db(), q.page.page, q.page.page_size)
///         .await?;
///     Ok(Res::ok(rows))
/// }
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SortQuery {
    /// 排序字段名（须命中调用方提供的白名单）；`None` 表示不排序。
    #[serde(default, alias = "sortBy")]
    pub sort_by: Option<String>,
    /// 排序方向，缺省为升序。
    #[serde(default)]
    pub order: SortOrder,
}

impl SortQuery {
    /// 是否降序。
    pub fn is_desc(&self) -> bool {
        matches!(self.order, SortOrder::Desc)
    }

    /// 排序字段名（`&str`），便于直接传给
    /// `ModelQuery::order_by_whitelisted`。
    pub fn field(&self) -> Option<&str> {
        self.sort_by.as_deref()
    }

    /// 空查询参数（不排序）。
    pub fn none() -> Self {
        Self::default()
    }
}

// ──── Res 实现 ──────────────────────────────────────

impl Res<()> {
    /// 成功（无数据载荷），返回 `{code: 0, msg: "ok", data: null}`
    pub fn ok_empty() -> Self {
        Self {
            code: codes::OK,
            msg: "ok".into(),
            data: None,
        }
    }

    /// 成功（自定义消息，无数据载荷）
    pub fn ok_msg(msg: impl Into<String>) -> Self {
        Self {
            code: codes::OK,
            msg: msg.into(),
            data: None,
        }
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
        Self {
            code: codes::OK,
            msg: "ok".into(),
            data: Some(data),
        }
    }

    /// 成功响应，携带数据载荷和自定义消息
    pub fn ok_with_msg(data: T, msg: impl Into<String>) -> Self {
        Self {
            code: codes::OK,
            msg: msg.into(),
            data: Some(data),
        }
    }

    /// 失败响应（自定义错误码和消息，无数据载荷）
    ///
    /// # 示例
    ///
    /// ```ignore
    /// Res::fail(codes::BAD_REQUEST, "用户名不能为空")
    /// ```
    pub fn fail(code: i32, msg: impl Into<String>) -> Self {
        Self {
            code,
            msg: msg.into(),
            data: None,
        }
    }
}

impl<T: Serialize> Res<PageData<T>> {
    /// 分页响应
    ///
    /// `list` 通常来自 `alun_db::Page<T>`；若直接使用数据库返回的 `Page`，
    /// 用 `PageResponseExt::into_res()` 少写一次字段映射。
    pub fn page(list: T, total: u64, page: u64, page_size: u64) -> Self {
        Self::ok(PageData::new(list, total, page, page_size))
    }
}

// ──── API 错误 ──────────────────────────────────────

/// API 错误 —— handler 唯一需要写出的错误类型
///
/// 业务报错直接写 `Err(ApiError::not_found(".."))`，类型精确匹配，连 `.into()`
/// 都不用写；数据库 / 框架 / IO 错误则由 [`From`] 实现自动映射成 HTTP 语义。
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
        Self {
            status,
            code,
            msg: msg.into(),
            internal_detail: None,
        }
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

    /// 500 Internal Server Error：服务端内部错误（已脱敏）
    ///
    /// 前端始终收到模糊提示 `"服务器内部错误"`；
    /// 真实错误存入 `internal_detail` 并通过 `tracing::error!` 写入日志。
    ///
    /// 如需自定义前端提示消息，请使用 [`internal_masked`]。
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(500, codes::INTERNAL, "服务器内部错误").with_detail(msg)
    }

    /// 500 Internal Server Error（带调试详情）
    ///
    /// - `public_msg`: 返回前端的模糊提示
    /// - `detail`: 内部日志记录的详细信息（如完整的错误栈）
    pub fn internal_masked(public_msg: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(500, codes::INTERNAL, public_msg).with_detail(detail)
    }

    /// 503 Service Unavailable：服务暂时不可用
    pub fn service_unavailable(msg: impl Into<String>) -> Self {
        Self::new(503, codes::SERVICE_UNAVAILABLE, msg)
    }
}

// ──── 错误映射：下层错误 → HTTP 语义（单一收敛点）──────

/// `DbError` → `ApiError`：数据库语义 → HTTP 状态码
///
/// | `DbError` 变体 | 对外 |
/// |---|---|
/// | `NotFound` | 404 · 记录不存在 |
/// | `UniqueViolation` | 409 · 数据重复（唯一约束冲突） |
/// | `ForeignKeyViolation` / `CheckViolation` | 409 · 数据约束冲突 |
/// | `Deadlock` / `SerializationFailure` / `LockTimeout` | 500 · 服务繁忙，请重试 |
/// | 其余（连接失败、SQL 执行失败、类型不匹配……） | 500 · 服务器内部错误（脱敏） |
///
/// 每条映射都把 `DbError` 原文（含驱动原因与 SQL）塞进 `internal_detail`，
/// 只进日志，绝不出现在响应体里。
#[cfg(feature = "db")]
pub fn db_error_to_api_error(e: &DbError) -> ApiError {
    let mut api = match e {
        DbError::NotFound => ApiError::not_found("记录不存在"),
        DbError::UniqueViolation { .. } => ApiError::conflict("数据重复：唯一约束冲突"),
        DbError::ForeignKeyViolation { .. } | DbError::CheckViolation { .. } => {
            ApiError::conflict("数据约束冲突")
        }
        DbError::Deadlock { .. }
        | DbError::SerializationFailure { .. }
        | DbError::LockTimeout { .. } => {
            ApiError::internal_masked("服务繁忙，请重试", e.to_string())
        }
        // 连接失败 / 查询失败 / 类型不匹配 / 迁移错误……：对外模糊，细节进日志
        _ => ApiError::internal(e.to_string()),
    };
    // `internal_detail` 是公开字段：统一覆盖成 DbError 原文（含驱动原因），
    // 保证无论走哪条分支，排障信息都不丢。
    api.internal_detail = Some(e.to_string());
    api
}

/// 数据库错误直通（需 `db` feature）：handler 里 `?` 一个 `DbResult` 不用写 `map_err`
#[cfg(feature = "db")]
impl From<DbError> for ApiError {
    fn from(e: DbError) -> Self {
        db_error_to_api_error(&e)
    }
}

/// 框架错误（配置 / 插件 / IO / 模板 / 启动）→ 500 脱敏，原文只进日志
impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        ApiError::internal_masked("服务器内部错误", e.to_string())
    }
}

/// IO 错误 → 500 脱敏（同上）
impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError::internal_masked("服务器内部错误", e.to_string())
    }
}

/// 反向通道：`ApiError` → 框架错误，供「handler 之外的流程」（启动、插件、定时任务）
/// 复用同一份文案。`?` 只做一次转换，所以 `alun::Result<T>` 收不下 `DbError`——
/// 那些场景请用 `DbResult<T>` 或 [`DbError`] 显式处理。
impl From<ApiError> for CoreError {
    fn from(e: ApiError) -> Self {
        CoreError::Msg(e.msg)
    }
}

// ──── IntoResponse 实现 ─────────────────────────────

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

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        if let Some(ref detail) = self.internal_detail {
            let status =
                StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            tracing::error!(status = status.as_u16(), code = self.code, detail = %detail,
                "请求处理异常");
        }
        let body = Res::<()>::fail(self.code, self.msg);
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let mut resp = (status, Json(body)).into_response();
        resp.headers_mut().insert(
            axum::http::HeaderName::from_static("content-type"),
            axum::http::HeaderValue::from_static("application/json; charset=utf-8"),
        );
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 业务主动构造的错误原样透出，不被二次脱敏。
    #[test]
    fn business_error_passes_through() {
        let api = ApiError::bad_request("参数不对");
        assert_eq!(api.status, 400);
        assert_eq!(api.msg, "参数不对");
    }

    /// 框架错误对外一律脱敏成「服务器内部错误」，原文只进 internal_detail。
    #[test]
    fn framework_error_is_masked() {
        let api: ApiError = CoreError::Config("config.toml 缺 database.url".into()).into();
        assert_eq!(api.status, 500);
        assert_eq!(api.msg, "服务器内部错误");
        assert!(
            api.internal_detail
                .as_deref()
                .is_some_and(|d| d.contains("database.url")),
            "排障详情必须留在日志侧"
        );
    }

    /// IO 错误同样走脱敏通道。
    #[test]
    fn io_error_is_masked() {
        let api: ApiError = std::io::Error::other("disk on fire").into();
        assert_eq!(api.status, 500);
        assert_eq!(api.msg, "服务器内部错误");
        assert!(api.internal_detail.is_some());
    }

    /// `ApiError` 可以反向回到框架错误（启动 / 插件 / 定时任务流程复用文案）。
    #[test]
    fn api_error_roundtrips_to_framework_error() {
        let core: CoreError = ApiError::not_found("用户不存在").into();
        assert_eq!(core.to_string(), "用户不存在");
    }
}

#[cfg(all(test, feature = "db"))]
mod db_tests {
    use super::*;

    fn unique_violation() -> DbError {
        DbError::UniqueViolation {
            column: Some("email".into()),
            constraint: None,
            source: None,
        }
    }

    /// `?` 从「返回 `DbResult` 的函数」直通「返回 `ResResult` 的 handler」。
    #[test]
    fn question_mark_works_across_error_types() {
        fn layer_db() -> std::result::Result<(), DbError> {
            Err(unique_violation())
        }
        fn handler() -> ResResult<()> {
            layer_db()?;
            Ok(Res::ok(()))
        }
        let err = handler().unwrap_err();
        assert_eq!(err.status, 409, "{err:?}");
    }

    /// 语义映射：可枚举的约束给业务状态码，且对外不含驱动原文。
    #[test]
    fn db_error_maps_to_semantic_status() {
        let api = ApiError::from(unique_violation());
        assert_eq!(api.status, 409);
        assert!(
            !api.msg.contains("unique"),
            "对外消息不得泄露驱动原文：{}",
            api.msg
        );
        assert!(
            api.internal_detail
                .as_deref()
                .is_some_and(|d| d.contains("unique")),
            "驱动原文必须进日志"
        );

        let missing = ApiError::from(DbError::NotFound);
        assert_eq!(missing.status, 404);
        assert_eq!(missing.code, codes::NOT_FOUND);

        let busy = ApiError::from(DbError::Deadlock {
            source: Box::new(std::io::Error::other("deadlock detected")),
        });
        assert_eq!(busy.status, 500);
        assert_eq!(busy.msg, "服务繁忙，请重试");
    }

    /// SQL 文本绝不能出现在对外消息里，但可以进日志。
    #[test]
    fn sql_never_leaks_to_client() {
        let api = ApiError::from(DbError::Query {
            sql: "SELECT secret FROM users".into(),
            source: Box::new(std::io::Error::other("boom")),
        });
        assert_eq!(api.status, 500);
        assert_eq!(api.msg, "服务器内部错误");
        assert!(!api.msg.contains("secret"), "SQL 绝不能出现在对外消息里");
        assert!(
            api.internal_detail
                .as_deref()
                .is_some_and(|d| d.contains("secret")),
            "SQL 可以进日志（排障需要），但只能进日志"
        );
    }
}

//! 请求提取器：ValidatedJson —— 对标 aifei 的 Argument + Validate
//!
//! 设计要点：
//! - 自动校验 JSON 请求体的字段合法性
//! - 校验失败返回 422 Unprocessable Entity
//! - 若目标类型实现 Validate trait，则自动调用 validate()
//! - 也可使用 alun_utils::valid::Valid 独立校验

use axum::{
    extract::{FromRequest, Json},
    response::{IntoResponse, Response},
};
use alun_core::api::ApiError;
use serde::de::DeserializeOwned;
use validator::ValidationError;

/// 带自动校验的 JSON 提取器
///
/// # 示例
///
/// ```ignore
/// #[derive(Debug, Deserialize, Validate)]
/// pub struct CreateUserReq {
///     #[validate(length(min = 2, max = 50))]
///     pub username: String,
///     #[validate(email)]
///     pub email: String,
///     #[validate(custom(function = "validate_uuid"))]
///     pub parent_id: String,
/// }
///
/// async fn create_user(ValidatedJson(req): ValidatedJson<CreateUserReq>) -> Res<String> {
///     Res::ok(req.username)
/// }
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct ValidatedJson<T>(pub T);

/// 实现 axum 的 `FromRequest` 提取器，将 JSON 请求体解析为 `T` 类型。
///
/// 解析失败时返回 `ValidatedJsonRejection`，该 rejection 会转换为 HTTP 400 响应。
impl<T, S> FromRequest<S> for ValidatedJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ValidatedJsonRejection;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(ValidatedJson(value)),
            Err(rejection) => {
                let msg = format!("请求体格式错误: {}", rejection.body_text());
                Err(ValidatedJsonRejection(ApiError::bad_request(msg)))
            }
        }
    }
}

/// ValidatedJson 的校验便捷方法 —— 对实现了 `validator::Validate` 的类型调用 `validate()`
impl<T: validator::Validate> ValidatedJson<T> {
    /// 执行字段级校验
    ///
    /// 校验失败返回 `ApiError::unprocessable_entity`，包含格式化的校验错误列表。
    pub fn validate(self) -> Result<Self, ApiError> {
        self.0.validate().map_err(|e| {
            ApiError::unprocessable_entity(
                alun_utils::valid::Valid::format_validation_errors(&e)
            )
        })?;
        Ok(self)
    }
}

impl<T> std::ops::Deref for ValidatedJson<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for ValidatedJson<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// ValidatedJson 提取失败时的错误类型
#[derive(Debug)]
pub struct ValidatedJsonRejection(pub ApiError);

impl IntoResponse for ValidatedJsonRejection {
    fn into_response(self) -> Response {
        self.0.into_response()
    }
}

impl From<ValidatedJsonRejection> for ApiError {
    fn from(rejection: ValidatedJsonRejection) -> Self {
        rejection.0
    }
}

// ---- 自定义 validator 校验函数 ----
//
// 以下函数可直接用于 #[validate(custom(function = "..."))] 属性宏
// 字段为空时自动跳过校验（validate_password_strength 除外）

/// validator 自定义校验：UUID 格式
///
/// 字段为空时自动跳过校验；有值时校验 UUID 格式（v1~v7 均支持）。
///
/// # 使用示例
///
/// ```ignore
/// #[derive(Debug, Deserialize, Validate)]
/// pub struct Req {
///     #[validate(custom(function = "validate_uuid"))]
///     pub parent_id: String,
/// }
/// ```
pub fn validate_uuid(value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Ok(());
    }
    if alun_utils::valid::Valid::is_uuid(value) {
        Ok(())
    } else {
        Err(ValidationError::new("invalid_uuid")
            .with_message("必须是有效的 UUID 格式".into()))
    }
}

/// validator 自定义校验：手机号/固话（中国大陆）
///
/// 字段为空时自动跳过校验；有值时校验手机号或固话格式。
///
/// ```ignore
/// #[validate(custom(function = "validate_mobile"))]
/// pub mobile: String,
/// ```
pub fn validate_mobile(value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Ok(());
    }
    if alun_utils::valid::Valid::is_mobile(value) {
        Ok(())
    } else {
        Err(ValidationError::new("invalid_mobile")
            .with_message("必须是有效的手机号".into()))
    }
}

/// validator 自定义校验：密码强度
///
/// 始终校验，为空时也会报错（密码必须满足强度要求）。
///
/// ```ignore
/// #[validate(custom(function = "validate_password_strength"))]
/// pub password: String,
/// ```
pub fn validate_password_strength(value: &str) -> Result<(), ValidationError> {
    if alun_utils::valid::Valid::is_strong_password(value) {
        Ok(())
    } else {
        Err(ValidationError::new("weak_password")
            .with_message("密码必须至少 8 位，包含大小写字母、数字和特殊字符".into()))
    }
}

/// validator 自定义校验：身份证号
///
/// 字段为空时自动跳过校验；有值时校验中国居民身份证号（含校验位）。
///
/// ```ignore
/// #[validate(custom(function = "validate_id_card"))]
/// pub id_card: String,
/// ```
pub fn validate_id_card(value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Ok(());
    }
    if alun_utils::valid::Valid::is_id_card(value) {
        Ok(())
    } else {
        Err(ValidationError::new("invalid_id_card")
            .with_message("必须是有效的身份证号".into()))
    }
}

/// validator 自定义校验：日期格式（YYYY-MM-DD）
///
/// 字段为空时自动跳过校验；有值时校验日期格式。
///
/// ```ignore
/// #[validate(custom(function = "validate_date"))]
/// pub date: String,
/// ```
pub fn validate_date(value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Ok(());
    }
    if alun_utils::valid::Valid::is_date(value) {
        Ok(())
    } else {
        Err(ValidationError::new("invalid_date")
            .with_message("日期格式必须为 YYYY-MM-DD".into()))
    }
}

/// validator 自定义校验：邮箱
///
/// 字段为空时自动跳过校验；有值时校验邮箱格式。
///
/// ```ignore
/// #[validate(custom(function = "validate_email"))]
/// pub email: String,
/// ```
pub fn validate_email(value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Ok(());
    }
    if alun_utils::valid::Valid::is_email(value) {
        Ok(())
    } else {
        Err(ValidationError::new("invalid_email")
            .with_message("必须是有效的邮箱格式".into()))
    }
}

/// validator 自定义校验：URL
///
/// 字段为空时自动跳过校验；有值时校验 URL 格式。
///
/// ```ignore
/// #[validate(custom(function = "validate_url"))]
/// pub url: String,
/// ```
pub fn validate_url(value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Ok(());
    }
    if alun_utils::valid::Valid::is_url(value) {
        Ok(())
    } else {
        Err(ValidationError::new("invalid_url")
            .with_message("必须是有效的 URL 格式".into()))
    }
}

/// validator 自定义校验：日期时间格式（ISO 8601 / RFC 3339）
///
/// 字段为空时自动跳过校验；有值时校验日期时间格式。
///
/// ```ignore
/// #[validate(custom(function = "validate_datetime"))]
/// pub publish_time: String,
/// ```
pub fn validate_datetime(value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Ok(());
    }
    if alun_utils::valid::Valid::is_datetime(value) {
        Ok(())
    } else {
        Err(ValidationError::new("invalid_datetime")
            .with_message("日期时间格式必须为 ISO 8601/RFC 3339".into()))
    }
}

/// 验证日期（YYYY-MM-DD）或日期时间格式（ISO 8601 / RFC 3339）
///
/// 用于允许纯日期（如 "2025-04-01"）和完整时间戳（如 "2025-04-01T00:00:00Z"）混合的字段校验。
/// 使用方式：
///
/// ```ignore
/// #[validate(custom(function = "validate_date_or_datetime"))]
/// pub release_date: Option<String>,
/// ```
pub fn validate_date_or_datetime(value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Ok(());
    }
    if alun_utils::valid::Valid::is_datetime(value)
        || alun_utils::valid::Valid::is_date(value)
    {
        Ok(())
    } else {
        Err(ValidationError::new("invalid_date_or_datetime")
            .with_message("日期格式必须为 YYYY-MM-DD 或 ISO 8601/RFC 3339".into()))
    }
}

/// 为实现了 `Validate` 的类型提供便捷的校验方法
///
/// 在 handler 中通过 `req.validate_or_reject()?;` 即可完成校验。
pub trait ValidateExt {
    fn validate_or_reject(&self) -> Result<(), ApiError>;
}

impl<T: validator::Validate> ValidateExt for T {
    fn validate_or_reject(&self) -> Result<(), ApiError> {
        self.validate().map_err(|e| {
            ApiError::unprocessable_entity(
                alun_utils::valid::Valid::format_validation_errors(&e)
            )
        })
    }
}
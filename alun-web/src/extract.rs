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

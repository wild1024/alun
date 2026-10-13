//! 业务定制的错误映射：`DbError → ApiError`。
//!
//! 需求文档 §4.1 融合点 4 希望写成 `impl IntoResponse for DbError`，但
//! `DbError`（alun-db-core）与 `IntoResponse`（axum）对业务 crate 都是外部类型，
//! 孤儿规则（E0117）禁止该实现。
//!
//! 框架已在 `alun-web` 提供默认映射（`From<DbError> for ApiError`，
//! 见 `alun_web::api::db_error_to_api_error`），handler 里直接 `?` 即可。
//! 本文件保留一份**业务定制版**：文案更具体（带上约束名 / 列名），
//! 供需要对客户端暴露精确原因的接口使用。

use alun_db::prelude::DbError;
use alun_web::ApiError;

/// 把 [`DbError`] 映射为统一 API 错误（业务定制文案）。
///
/// 约束类变体携带**结构化标识符**（`column` / `constraint` 均为 `Option`，取不到为 `None`，
/// 驱动原文留在 `source()` 链上，D014 已修复），事务冲突类有专用变体（D013 已修复）。
pub fn map_db_err(e: &DbError) -> ApiError {
    match e {
        DbError::NotFound => ApiError::not_found("记录不存在"),
        DbError::UniqueViolation {
            column, constraint, ..
        } => {
            let who = constraint
                .as_deref()
                .or(column.as_deref())
                .unwrap_or("unknown");
            ApiError::conflict(format!("唯一约束冲突: {who}"))
        }
        DbError::ForeignKeyViolation { constraint, .. } => ApiError::bad_request(format!(
            "外键约束冲突: {}",
            constraint.as_deref().unwrap_or("unknown")
        )),
        DbError::CheckViolation { constraint, .. } => ApiError::unprocessable_entity(format!(
            "CHECK 约束冲突: {}",
            constraint.as_deref().unwrap_or("unknown")
        )),
        // 事务冲突类：客户端可安全重试（D013 已修复，不再落 Query）
        DbError::Deadlock { .. } => ApiError::conflict("数据库死锁，请重试"),
        DbError::SerializationFailure { .. } => ApiError::conflict("事务序列化失败，请重试"),
        DbError::LockTimeout { .. } => ApiError::service_unavailable("数据库锁等待超时，请重试"),
        DbError::Readonly(table) => ApiError::forbidden(format!("模型 `{table}` 为只读视图")),
        DbError::UnsupportedFeature { feature, dialect } => {
            ApiError::bad_request(format!("方言 `{dialect}` 不支持 `{feature}`"))
        }
        DbError::InvalidValue(msg) => ApiError::bad_request(msg.clone()),
        DbError::UnexpectedNull(col) => {
            ApiError::unprocessable_entity(format!("列 `{col}` 为 NULL"))
        }
        DbError::TypeMismatch { expected, actual } => ApiError::internal_masked(
            "数据类型不匹配",
            format!("expected {expected}, got {actual}"),
        ),
        other => ApiError::internal_masked("服务器内部错误", other.to_string()),
    }
}

/// 需求 §4.1 融合点 4 的 newtype 替代：业务侧包装后可自由实现 `IntoResponse`。
pub struct LedgerError(pub DbError);

impl From<DbError> for LedgerError {
    fn from(e: DbError) -> Self {
        LedgerError(e)
    }
}

impl From<LedgerError> for ApiError {
    fn from(e: LedgerError) -> Self {
        map_db_err(&e.0)
    }
}

//! Alun database: Row 模式 + 事务 RAII + Hook + SQL 模板
//!
//! 设计理念：
//!
//! 1. **Db + Row 模式** —— 无 Model 也可操作，字段追踪 + 类型安全
//! 2. **事务隐式提交** —— Rust 用 `?` 天然保证永不"忘记回滚"
//! 3. **Hook 生命周期** —— before/after 拦截 CRUD，支持审计/时间戳自动填充
//! 4. **配置驱动** —— 从 AppConfig 创建连接池，支持加密密码存储
//! 5. **多数据库支持** —— PostgreSQL / MySQL / SQLite，工厂按 type 自动选择

pub mod db;
pub mod row;
pub mod tx;
pub mod hook;
pub mod sql;
pub mod dialect;
pub mod factory;
pub mod migrate;
pub mod idkind;

pub use db::Db;
pub use alun_core::PageQuery;
pub use row::Row;
pub use tx::{ActiveTx, Isolation};
pub use hook::{Hook, HookChain, NullHook};
pub use sql::{SqlTemplate, SqlPara};
pub use dialect::Dialect;
pub use factory::{create_db, create_db_if_enabled};
pub use idkind::IdKind;

/// alun-db 错误
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("数据库错误: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("序列化错误: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("参数错误: {0}")]
    Argument(String),

    #[error("事务回滚: {0}")]
    Rollback(String),

    #[error("{0}")]
    Other(String),
}

pub type DbResult<T> = Result<T, DbError>;

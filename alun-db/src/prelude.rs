//! 常用类型重导出。

// ==================== 核心类型 ====================
pub use alun_db_core::prelude::*;
pub use alun_db_core::traits::Params;

// ==================== Derive 宏 ====================
pub use alun_db_macro::{Model, Row, TypeMapper, migrate, sql};

// ==================== facade 类型 ====================
pub use crate::builder::{CountBuilder, FindBuilder, PageBuilder};
pub use crate::changeset::ChangeSet;
pub use crate::config::{DatabaseConfig, PoolConfig};
pub use crate::crud::CrudExt;
pub use crate::db::Db;
pub use crate::migrate::{
    Divergent, DryRunReport, FreshReport, Migration, MigrationKind, MigrationState,
    MigrationStatus, Migrator, MigratorOptions, RepairReport, ResetReport, Version,
    split_statements,
};
pub use crate::model_query::ModelQuery;
pub use crate::persist::PersistExt;
pub use crate::query::{SqlExecutor, SqlQueryExt};
pub use crate::rowops::RowBatchExt;
pub use crate::tenant::{
    TenantContext, current_tenant, current_tenant_key, is_tenant_enabled, is_tenant_schema_enabled,
    is_tenant_write_enabled, set_tenant_enabled, set_tenant_schema_enabled,
    set_tenant_schema_prefix, set_tenant_write_enabled, tenant_schema_prefix, tenant_scope,
    tenant_value_key,
};
pub use crate::tx::Transaction;

// ==================== 全局函数 ====================
pub use crate::{db, init, init_with, try_db};

// ==================== 生态类型 ====================
#[cfg(feature = "bigdecimal")]
pub use bigdecimal;
#[cfg(feature = "chrono")]
pub use chrono;
#[cfg(feature = "ipnetwork")]
pub use ipnetwork;
#[cfg(feature = "jiff")]
pub use jiff;
#[cfg(feature = "rust_decimal")]
pub use rust_decimal;
#[cfg(feature = "json")]
pub use serde_json;
#[cfg(feature = "time")]
pub use time;
#[cfg(feature = "ulid")]
pub use ulid;
#[cfg(feature = "uuid")]
pub use uuid;

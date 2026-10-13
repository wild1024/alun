//! SQLite 方言。

use alun_db_core::error::DbError;
use alun_db_core::expr::Expr;
use alun_db_core::statement::Statement;
use alun_db_core::stmt::Type;
use alun_db_core::traits::dialect::{
    Dialect, DialectCapability, Placeholder, RawDbError, RenderCtx, RenderedSql,
};

/// SQLite 方言。
///
/// ## 能力
///
/// - 无原生 `BOOLEAN`，使用 `INTEGER`（0/1）
/// - 无原生 `UUID`，使用 `TEXT`
/// - 无原生 `JSON`，使用 `TEXT` + `json_*` 函数
/// - 支持 `RETURNING`（3.35.0+）
/// - 支持 `ON CONFLICT` UPSERT
/// - 不支持 `ILIKE`，降级为 `LOWER(...) LIKE LOWER(...)`
/// - 占位符为 `?`
pub struct SqliteDialect;

impl Dialect for SqliteDialect {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    fn capability(&self) -> &DialectCapability {
        &DialectCapability {
            native_bool: false,
            native_uuid: false,
            native_json: false,
            native_enum: false,
            returning_clause: true,
            ilike: false,
            array_type: false,
            on_conflict: true,
            nulls_order: true,
            placeholder: Placeholder::Question,
        }
    }

    fn render_type(&self, ty: &Type) -> String {
        match ty.inner() {
            Type::Bool => "INTEGER".into(),
            Type::I8 | Type::I16 | Type::I32 | Type::I64 => "INTEGER".into(),
            Type::U8 | Type::U16 | Type::U32 | Type::U64 => "INTEGER".into(),
            Type::F32 | Type::F64 => "REAL".into(),
            Type::Text => "TEXT".into(),
            Type::Bytes => "BLOB".into(),
            Type::Date => "TEXT".into(),
            Type::Time => "TEXT".into(),
            Type::Timestamp => "TEXT".into(),
            Type::TimestampTz => "TEXT".into(),
            Type::Uuid => "TEXT".into(),
            Type::Ulid => "TEXT".into(),
            Type::Decimal => "TEXT".into(),
            Type::Json => "TEXT".into(),
            Type::Enum(_) => "TEXT".into(),
            Type::Inet => "TEXT".into(),
            Type::Cidr => "TEXT".into(),
            Type::Array(_) => "TEXT".into(),
            Type::Unknown => "BLOB".into(),
            Type::Nullable(_) => unreachable!(),
        }
    }

    fn render_expr(&self, expr: &Expr, ctx: &mut RenderCtx<'_>) -> String {
        super::super::render::expr::render_expr(expr, ctx)
    }

    fn render_statement(&self, stmt: &Statement) -> RenderedSql {
        super::super::render::statement::render_statement(stmt, self)
    }

    fn render_now(&self) -> String {
        "datetime('now')".into()
    }

    fn quote_identifier(&self, name: &str) -> String {
        format!("\"{name}\"")
    }

    fn map_error(&self, raw: RawDbError) -> DbError {
        // 跨方言错误码分支集中在 core 的统一入口（含约束标识符解析）
        DbError::from_raw_driver_error(raw)
    }
}

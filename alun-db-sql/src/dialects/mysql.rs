//! MySQL 方言。

use alun_db_core::error::DbError;
use alun_db_core::expr::Expr;
use alun_db_core::statement::Statement;
use alun_db_core::stmt::Type;
use alun_db_core::traits::dialect::{
    Dialect, DialectCapability, Placeholder, RawDbError, RenderCtx, RenderedSql,
};

/// MySQL 方言。
///
/// ## 能力
///
/// - 无原生 `BOOLEAN`，使用 `TINYINT(1)`
/// - 无原生 `UUID`，使用 `CHAR(36)`
/// - 原生 `JSON`
/// - 不支持 `RETURNING`，使用 `LAST_INSERT_ID()`
/// - 支持 `ON DUPLICATE KEY UPDATE`（非 `ON CONFLICT`）
/// - 不支持 `ILIKE`，降级为 `LOWER(...) LIKE LOWER(...)`
/// - 占位符为 `?`
/// - 标识符使用反引号
pub struct MysqlDialect;

impl Dialect for MysqlDialect {
    fn name(&self) -> &'static str {
        "mysql"
    }

    fn capability(&self) -> &DialectCapability {
        &DialectCapability {
            native_bool: false,
            native_uuid: false,
            native_json: true,
            native_enum: true,
            returning_clause: false,
            ilike: false,
            array_type: false,
            on_conflict: false,
            nulls_order: false,
            placeholder: Placeholder::Question,
        }
    }

    fn render_type(&self, ty: &Type) -> String {
        match ty.inner() {
            Type::Bool => "TINYINT(1)".into(),
            Type::I8 | Type::I16 => "SMALLINT".into(),
            Type::I32 => "INT".into(),
            Type::I64 => "BIGINT".into(),
            Type::U8 | Type::U16 | Type::U32 | Type::U64 => "BIGINT".into(),
            Type::F32 => "FLOAT".into(),
            Type::F64 => "DOUBLE".into(),
            Type::Text => "VARCHAR(255)".into(),
            Type::Bytes => "BLOB".into(),
            Type::Date => "DATE".into(),
            Type::Time => "TIME".into(),
            Type::Timestamp => "DATETIME".into(),
            Type::TimestampTz => "DATETIME(6)".into(),
            Type::Uuid => "CHAR(36)".into(),
            Type::Ulid => "CHAR(26)".into(),
            Type::Decimal => "DECIMAL(38, 10)".into(),
            Type::Json => "JSON".into(),
            Type::Enum(_) => "VARCHAR(255)".into(),
            Type::Inet => "VARCHAR(45)".into(),
            Type::Cidr => "VARCHAR(45)".into(),
            Type::Array(_) => "JSON".into(),
            Type::Unknown => "TEXT".into(),
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
        "NOW()".into()
    }

    fn quote_identifier(&self, name: &str) -> String {
        format!("`{name}`")
    }

    fn map_error(&self, raw: RawDbError) -> DbError {
        // 跨方言错误码分支集中在 core 的统一入口（含约束标识符解析）
        DbError::from_raw_driver_error(raw)
    }
}

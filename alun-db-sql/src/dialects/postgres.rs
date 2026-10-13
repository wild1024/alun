//! PostgreSQL 方言。

use alun_db_core::error::DbError;
use alun_db_core::expr::Expr;
use alun_db_core::statement::Statement;
use alun_db_core::stmt::Type;
use alun_db_core::traits::dialect::{
    Dialect, DialectCapability, Placeholder, RawDbError, RenderCtx, RenderedSql,
};

/// PostgreSQL 方言。
///
/// ## 能力
///
/// - 原生 `BOOLEAN`、`UUID`、`JSONB`、`ENUM`、`INET`、`CIDR`
/// - 原生数组类型
/// - 支持 `RETURNING`
/// - 支持 `ON CONFLICT`
/// - 支持 `ILIKE`
/// - 支持 `NULLS FIRST` / `NULLS LAST`
/// - 占位符为 `$1, $2, ...`
pub struct PostgresDialect;

impl Dialect for PostgresDialect {
    fn name(&self) -> &'static str {
        "postgres"
    }

    fn capability(&self) -> &DialectCapability {
        &DialectCapability {
            native_bool: true,
            native_uuid: true,
            native_json: true,
            native_enum: true,
            returning_clause: true,
            ilike: true,
            array_type: true,
            on_conflict: true,
            nulls_order: true,
            placeholder: Placeholder::Dollar,
        }
    }

    fn render_type(&self, ty: &Type) -> String {
        match ty.inner() {
            Type::Bool => "BOOLEAN".into(),
            Type::I8 | Type::I16 => "SMALLINT".into(),
            Type::I32 => "INTEGER".into(),
            Type::I64 => "BIGINT".into(),
            Type::U8 | Type::U16 | Type::U32 | Type::U64 => "BIGINT".into(),
            Type::F32 => "REAL".into(),
            Type::F64 => "DOUBLE PRECISION".into(),
            Type::Text => "TEXT".into(),
            Type::Bytes => "BYTEA".into(),
            Type::Date => "DATE".into(),
            Type::Time => "TIME".into(),
            Type::Timestamp => "TIMESTAMP".into(),
            Type::TimestampTz => "TIMESTAMPTZ".into(),
            Type::Uuid => "UUID".into(),
            Type::Ulid => "UUID".into(),
            Type::Decimal => "NUMERIC".into(),
            Type::Json => "JSONB".into(),
            Type::Enum(name) => (*name).to_string(),
            Type::Inet => "INET".into(),
            Type::Cidr => "CIDR".into(),
            Type::Array(inner) => format!("{}[]", self.render_type(inner)),
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
        format!("\"{name}\"")
    }

    fn map_error(&self, raw: RawDbError) -> DbError {
        // 跨方言错误码分支集中在 core 的统一入口（含约束标识符解析）
        DbError::from_raw_driver_error(raw)
    }
}

//! JSON 操作符的方言适配。
//!
//! 不同数据库的 JSON 路径访问语法差异很大：
//!
//! | 操作 | PostgreSQL | MySQL | SQLite |
//! |---|---|---|---|
//! | 提取 JSON 对象字段 | `col -> 'key'` | `JSON_EXTRACT(col, '$.key')` | `json_extract(col, '$.key')` |
//! | 提取文本值 | `col ->> 'key'` | `JSON_UNQUOTE(JSON_EXTRACT(col, '$.key'))` | `json_extract(col, '$.key')` |
//! | 多级路径 | `col #> '{a,b}'` | `JSON_EXTRACT(col, '$.a.b')` | `json_extract(col, '$.a.b')` |

use alun_db_core::expr::Expr;
use alun_db_core::traits::dialect::RenderCtx;

/// 渲染 JSON 路径访问。
///
/// `Expr::JsonGet` 的语义是“提取 JSON 字段的文本值”，因此各方言都使用
/// 返回文本的操作符（PostgreSQL 的 `->>`、MySQL 的 `JSON_UNQUOTE`）。
pub fn render_json_get(expr: &Expr, path: &str, ctx: &mut RenderCtx<'_>) -> String {
    let expr_sql = super::expr::render_expr(expr, ctx);
    let dialect_name = ctx.dialect.name();

    match dialect_name {
        "postgres" => render_postgres_json(&expr_sql, path),
        "mysql" => render_mysql_json(&expr_sql, path),
        "sqlite" => render_sqlite_json(&expr_sql, path),
        _ => render_generic_json(&expr_sql, path),
    }
}

/// PostgreSQL：`col ->> 'key'` 或 `col #>> '{a,b}'`
fn render_postgres_json(expr_sql: &str, path: &str) -> String {
    if path.contains('.') {
        let path_array = path.split('.').collect::<Vec<_>>().join(",");
        format!("{expr_sql} #>> '{{{path_array}}}'")
    } else {
        format!("{expr_sql} ->> '{path}'")
    }
}

/// MySQL：`JSON_UNQUOTE(JSON_EXTRACT(col, '$.path'))`
fn render_mysql_json(expr_sql: &str, path: &str) -> String {
    let json_path = format!("$.{path}");
    format!("JSON_UNQUOTE(JSON_EXTRACT({expr_sql}, '{json_path}'))")
}

/// SQLite：`json_extract(col, '$.path')`
///
/// SQLite 的 `json_extract` 对标量值直接返回文本，无需额外解包。
fn render_sqlite_json(expr_sql: &str, path: &str) -> String {
    let json_path = format!("$.{path}");
    format!("json_extract({expr_sql}, '{json_path}')")
}

/// 通用降级：`JSON_EXTRACT` 风格。
fn render_generic_json(expr_sql: &str, path: &str) -> String {
    let json_path = format!("$.{path}");
    format!("JSON_EXTRACT({expr_sql}, '{json_path}')")
}

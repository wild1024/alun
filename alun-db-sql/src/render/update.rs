//! UPDATE 语句渲染。

use alun_db_core::statement::UpdateStmt;
use alun_db_core::traits::dialect::RenderCtx;

/// 渲染 UPDATE 语句。
pub fn render_update(stmt: &UpdateStmt, ctx: &mut RenderCtx<'_>) -> String {
    let table = ctx.dialect.quote_identifier(stmt.table);
    let sets: Vec<String> = stmt
        .sets
        .iter()
        .map(|(col, expr)| {
            let c = ctx.dialect.quote_identifier(col);
            let v = super::expr::render_expr(expr, ctx);
            format!("{c} = {v}")
        })
        .collect();

    let mut sql = format!("UPDATE {table} SET {}", sets.join(", "));

    // WHERE：先渲染用户条件再渲染租户条件；两者都有时给用户条件加括号，
    // 避免其中的 OR 短路租户条件。UPDATE 为单表语句，租户列不加表限定
    // （SQLite 不容忍 UPDATE 的 WHERE 出现表限定列引用）。
    let filter_sql = stmt
        .filter
        .as_ref()
        .map(|filter| super::expr::render_expr(filter, ctx));
    let scope_sql = super::scope::render_scope_mode(&stmt.scope, false, ctx);
    match (filter_sql, scope_sql) {
        (Some(filter), Some(scope)) => sql.push_str(&format!(" WHERE ({filter}) AND {scope}")),
        (Some(filter), None) => sql.push_str(&format!(" WHERE {filter}")),
        (None, Some(scope)) => sql.push_str(&format!(" WHERE {scope}")),
        (None, None) => {}
    }

    if let Some(returning) = &stmt.returning
        && ctx.dialect.capability().returning_clause
    {
        let cols = returning
            .iter()
            .map(|c| ctx.dialect.quote_identifier(c))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" RETURNING {cols}"));
    }

    sql
}

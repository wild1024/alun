//! INSERT 语句渲染。

use alun_db_core::statement::{ConflictAction, InsertStmt};
use alun_db_core::traits::dialect::RenderCtx;

/// 渲染 INSERT 语句。
pub fn render_insert(stmt: &InsertStmt, ctx: &mut RenderCtx<'_>) -> String {
    let table = ctx.dialect.quote_identifier(stmt.table);
    let columns = stmt
        .columns
        .iter()
        .map(|c| ctx.dialect.quote_identifier(c))
        .collect::<Vec<_>>()
        .join(", ");

    // VALUES
    let rows: Vec<String> = stmt
        .values
        .iter()
        .map(|row| {
            let vals: Vec<String> = row
                .iter()
                .map(|e| super::expr::render_expr(e, ctx))
                .collect();
            format!("({})", vals.join(", "))
        })
        .collect();

    let mut sql = format!("INSERT INTO {table} ({columns}) VALUES {}", rows.join(", "));

    // ON CONFLICT
    if let Some(conflict) = &stmt.on_conflict
        && ctx.dialect.capability().on_conflict
    {
        sql.push_str(&render_on_conflict(conflict, ctx));
    }

    // RETURNING
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

/// 渲染 ON CONFLICT 子句。
///
/// PostgreSQL 和 SQLite 使用 `ON CONFLICT ... DO ...` 语法。
/// MySQL 使用 `ON DUPLICATE KEY UPDATE`，需要不同的渲染路径。
fn render_on_conflict(
    conflict: &alun_db_core::statement::OnConflict,
    ctx: &mut RenderCtx<'_>,
) -> String {
    let target = conflict
        .target
        .iter()
        .map(|c| ctx.dialect.quote_identifier(c))
        .collect::<Vec<_>>()
        .join(", ");

    match &conflict.action {
        ConflictAction::DoNothing => {
            format!(" ON CONFLICT ({target}) DO NOTHING")
        }
        ConflictAction::DoUpdate(sets) => {
            let set_clauses: Vec<String> = sets
                .iter()
                .map(|(col, expr)| {
                    let c = ctx.dialect.quote_identifier(col);
                    let v = super::expr::render_expr(expr, ctx);
                    format!("{c} = {v}")
                })
                .collect();
            format!(
                " ON CONFLICT ({target}) DO UPDATE SET {}",
                set_clauses.join(", ")
            )
        }
    }
}

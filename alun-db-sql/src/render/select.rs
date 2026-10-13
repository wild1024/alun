//! SELECT 语句渲染。

use alun_db_core::statement::{ColumnRef, NullsOrder, Order, OrderBy, SelectStmt};
use alun_db_core::traits::dialect::RenderCtx;

/// 渲染 SELECT 语句。
pub fn render_select(stmt: &SelectStmt, ctx: &mut RenderCtx<'_>) -> String {
    let mut parts = Vec::new();

    // SELECT 列
    let columns = if stmt.columns.is_empty() {
        "*".to_string()
    } else {
        stmt.columns
            .iter()
            .map(|c| render_column(c, ctx))
            .collect::<Vec<_>>()
            .join(", ")
    };
    parts.push(format!("SELECT {columns}"));

    // FROM
    let table = ctx.dialect.quote_identifier(stmt.table);
    parts.push(format!("FROM {table}"));

    // JOIN：从表租户条件追加到 ON，防止跨租户联查
    for join in &stmt.joins {
        let kind = match join.kind {
            alun_db_core::statement::JoinKind::Inner => "INNER JOIN",
            alun_db_core::statement::JoinKind::Left => "LEFT JOIN",
            alun_db_core::statement::JoinKind::Right => "RIGHT JOIN",
            alun_db_core::statement::JoinKind::Full => "FULL JOIN",
        };
        let join_table = ctx.dialect.quote_identifier(join.table);
        let on = super::expr::render_expr(&join.on, ctx);
        let on = match &join.scope {
            Some(filter) => {
                let scope = super::scope::render_scope_filter(filter, true, ctx);
                format!("{on} AND {scope}")
            }
            None => on,
        };
        parts.push(format!("{kind} {join_table} ON {on}"));
    }

    // WHERE：先渲染用户条件再渲染租户条件（保证参数绑定顺序与 SQL 文本一致）。
    // 两者都有时把用户条件加括号，避免其中的 OR 短路租户条件。
    let filter_sql = stmt
        .filter
        .as_ref()
        .map(|filter| super::expr::render_expr(filter, ctx));
    let scope_sql = super::scope::render_scope_mode(&stmt.scope, true, ctx);
    match (filter_sql, scope_sql) {
        (Some(filter), Some(scope)) => parts.push(format!("WHERE ({filter}) AND {scope}")),
        (Some(filter), None) => parts.push(format!("WHERE {filter}")),
        (None, Some(scope)) => parts.push(format!("WHERE {scope}")),
        (None, None) => {}
    }

    // GROUP BY
    if !stmt.group_by.is_empty() {
        let cols = stmt
            .group_by
            .iter()
            .map(|c| render_column(c, ctx))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("GROUP BY {cols}"));
    }

    // HAVING
    if let Some(having) = &stmt.having {
        let having_sql = super::expr::render_expr(having, ctx);
        parts.push(format!("HAVING {having_sql}"));
    }

    // ORDER BY
    if !stmt.order_by.is_empty() {
        let orders = stmt
            .order_by
            .iter()
            .map(|o| render_order_by(o, ctx))
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("ORDER BY {orders}"));
    }

    // LIMIT / OFFSET：直接追加到已拼好的 SQL 尾部（渲染器自带前导空格）
    let mut sql = parts.join(" ");
    ctx.dialect
        .push_limit_offset(stmt.limit, stmt.offset, &mut sql);
    sql
}

/// 渲染列引用。
fn render_column(col: &ColumnRef, ctx: &mut RenderCtx<'_>) -> String {
    match col {
        ColumnRef::Column(name) => ctx.dialect.quote_identifier(name),
        ColumnRef::Aliased { name, alias } => {
            format!(
                "{} AS {}",
                ctx.dialect.quote_identifier(name),
                ctx.dialect.quote_identifier(alias)
            )
        }
        ColumnRef::Star(Some(table)) => {
            format!("{}.*", ctx.dialect.quote_identifier(table))
        }
        ColumnRef::Star(None) => "*".to_string(),
        ColumnRef::Expr { expr, alias } => {
            let expr_sql = super::expr::render_expr(expr, ctx);
            format!("{expr_sql} AS {}", ctx.dialect.quote_identifier(alias))
        }
    }
}

/// 渲染 ORDER BY 子句。
fn render_order_by(order: &OrderBy, ctx: &mut RenderCtx<'_>) -> String {
    let col = render_column(&order.expr, ctx);
    let dir = match order.direction {
        Order::Asc => "ASC",
        Order::Desc => "DESC",
    };
    let mut result = format!("{col} {dir}");
    if ctx.dialect.capability().nulls_order
        && let Some(nulls) = order.nulls
    {
        let n = match nulls {
            NullsOrder::First => "NULLS FIRST",
            NullsOrder::Last => "NULLS LAST",
        };
        result.push_str(&format!(" {n}"));
    }
    result
}

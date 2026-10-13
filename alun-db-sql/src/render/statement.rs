//! 语句渲染总入口。

use alun_db_core::statement::Statement;
use alun_db_core::traits::dialect::{Dialect, RenderCtx, RenderedSql};

/// 将语句渲染为 SQL 与绑定参数。
pub fn render_statement(stmt: &Statement, dialect: &dyn Dialect) -> RenderedSql {
    // Raw 语句不渲染，直接返回
    if let Statement::Raw(raw) = stmt {
        return RenderedSql {
            sql: raw.sql.clone(),
            params: raw.params.clone(),
        };
    }
    let mut ctx = RenderCtx::new(dialect);
    let sql = match stmt {
        Statement::Select(s) => super::select::render_select(s, &mut ctx),
        Statement::Insert(s) => super::insert::render_insert(s, &mut ctx),
        Statement::Update(s) => super::update::render_update(s, &mut ctx),
        Statement::Delete(s) => super::delete::render_delete(s, &mut ctx),
        Statement::Raw(_) => unreachable!(),
    };
    RenderedSql {
        sql,
        params: ctx.params,
    }
}

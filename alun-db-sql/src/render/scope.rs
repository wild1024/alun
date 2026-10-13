//! 租户作用域渲染。
//!
//! 把 [`ScopeMode`] / [`ScopeFilter`] 渲染为 SQL 谓词并绑定租户值。租户值一律
//! 走参数绑定（`?` / `$n`）而非内联：租户值每个请求都不同，内联会让 SQLite
//! 的语句缓存按租户值炸开（与 `LIMIT` 内联的取舍相反，见
//! [`Dialect::push_limit_offset`](alun_db_core::traits::dialect::Dialect::push_limit_offset)）。

use alun_db_core::statement::{ScopeFilter, ScopeMode};
use alun_db_core::traits::dialect::RenderCtx;

/// 渲染语句级租户作用域。
///
/// - [`ScopeMode::Scoped`] → 渲染为 `"列" = ?`（或带表限定），并绑定租户值
/// - [`ScopeMode::Unset`] / [`ScopeMode::Cross`] → `None`（不产生条件）
///
/// `qualify` 为 `true` 时输出 `"表"."列" = ?`，用于 SELECT / JOIN 等可能出现
/// 同名列的场景；为 `false` 时输出 `"列" = ?`，用于单表 UPDATE / DELETE
/// （SQLite 不容忍 UPDATE / DELETE 的 WHERE 中出现表限定列引用）。
pub fn render_scope_mode(
    scope: &ScopeMode,
    qualify: bool,
    ctx: &mut RenderCtx<'_>,
) -> Option<String> {
    match scope {
        ScopeMode::Scoped(filter) => Some(render_scope_filter(filter, qualify, ctx)),
        ScopeMode::Unset | ScopeMode::Cross => None,
    }
}

/// 渲染单个租户过滤条件。
///
/// `qualify` 语义同 [`render_scope_mode`]。租户列名来自宏生成的静态字符串，
/// 仍走方言引用；租户值经 [`RenderCtx::bind`] 参数化绑定。
pub fn render_scope_filter(filter: &ScopeFilter, qualify: bool, ctx: &mut RenderCtx<'_>) -> String {
    let column = ctx.dialect.quote_identifier(filter.column);
    let lhs = if qualify {
        format!("{}.{column}", ctx.dialect.quote_identifier(filter.table))
    } else {
        column
    };
    let placeholder = ctx.bind(filter.value.clone());
    format!("{lhs} = {placeholder}")
}

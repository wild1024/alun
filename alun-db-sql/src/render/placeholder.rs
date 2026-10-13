//! 占位符生成。

use alun_db_core::stmt::Value;
use alun_db_core::traits::dialect::{Placeholder, RenderCtx};

/// 根据方言能力生成占位符，并将值绑定到上下文。
pub fn bind_value(value: Value, ctx: &mut RenderCtx<'_>) -> String {
    let idx = ctx.params.len() + 1;
    ctx.params.push(value);
    match ctx.dialect.capability().placeholder {
        Placeholder::Question => "?".to_string(),
        Placeholder::Dollar => format!("${idx}"),
    }
}

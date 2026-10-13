//! 表达式渲染。

use alun_db_core::expr::{BinaryOp, Expr, Function, UnaryOp};
use alun_db_core::traits::dialect::RenderCtx;

/// 渲染表达式为 SQL 片段。
///
/// 渲染过程中，参数值通过 `ctx.bind(value)` 绑定，返回对应的占位符字符串。
pub fn render_expr(expr: &Expr, ctx: &mut RenderCtx<'_>) -> String {
    match expr {
        // ==================== 基础 ====================
        Expr::Column { table, column } => {
            // 空 table 表示无表前缀（Builder 单表查询场景）
            if table.is_empty() {
                ctx.dialect.quote_identifier(column)
            } else {
                let t = ctx.dialect.quote_identifier(table);
                let c = ctx.dialect.quote_identifier(column);
                format!("{t}.{c}")
            }
        }

        Expr::Value(v) => ctx.bind(v.clone()),

        Expr::AlwaysTrue => "1=1".to_string(),

        Expr::Raw(raw) => {
            // 逐字符扫描，跳过单引号字符串字面量内的 `?`；每个引用占位符
            // `?` 依次消费一个 bind（在当前位置渲染并绑定其参数，保证与
            // 文本顺序一致）。bind 多于 `?` 时多余部分不参与绑定，避免
            // 虚参数污染 params。
            let sql = raw.sql();
            let binds = raw.binds();
            let mut result = String::with_capacity(sql.len() * 2);
            let mut in_quote = false;
            let mut bind_iter = binds.iter();
            for c in sql.chars() {
                match c {
                    '\'' => {
                        in_quote = !in_quote;
                        result.push(c);
                    }
                    '?' if !in_quote => match bind_iter.next() {
                        Some(bind) => result.push_str(&render_expr(bind, ctx)),
                        None => result.push('?'),
                    },
                    _ => result.push(c),
                }
            }
            result
        }

        // ==================== 组合 ====================
        Expr::Binary { left, op, right } => {
            let l = render_expr(left, ctx);
            match op {
                BinaryOp::IsNull => format!("{l} IS NULL"),
                BinaryOp::IsNotNull => format!("{l} IS NOT NULL"),
                BinaryOp::In | BinaryOp::NotIn => {
                    let r = render_in_operand(right, ctx);
                    let op_str = if *op == BinaryOp::In { "IN" } else { "NOT IN" };
                    format!("{l} {op_str} {r}")
                }
                BinaryOp::ILike => {
                    if ctx.dialect.capability().ilike {
                        let r = render_expr(right, ctx);
                        format!("{l} ILIKE {r}")
                    } else {
                        // 降级：LOWER(left) LIKE LOWER(right)
                        let r = render_expr(right, ctx);
                        format!("LOWER({l}) LIKE LOWER({r})")
                    }
                }
                _ => {
                    let r = render_expr(right, ctx);
                    let op_str = render_binary_op(*op);
                    format!("{l} {op_str} {r}")
                }
            }
        }

        Expr::Unary { op, expr } => {
            let e = render_expr(expr, ctx);
            match op {
                UnaryOp::Not => format!("NOT ({e})"),
                UnaryOp::Neg => format!("-({e})"),
            }
        }

        Expr::Call { func, args } => render_function(*func, args, ctx),

        Expr::JsonGet { expr, path } => super::json::render_json_get(expr, path, ctx),

        Expr::And(exprs) => {
            let parts: Vec<String> = exprs.iter().map(|e| render_expr(e, ctx)).collect();
            format!("({})", parts.join(" AND "))
        }

        Expr::Or(exprs) => {
            let parts: Vec<String> = exprs.iter().map(|e| render_expr(e, ctx)).collect();
            format!("({})", parts.join(" OR "))
        }
    }
}

/// 渲染 IN / NOT IN 的右操作数。
///
/// `Expr::Value(Array)` 渲染为 `(?, ?, ...)`；
/// 其他表达式按普通表达式渲染。
fn render_in_operand(right: &Expr, ctx: &mut RenderCtx<'_>) -> String {
    match right {
        Expr::Value(alun_db_core::stmt::Value::Array(items)) => {
            let placeholders: Vec<String> = items.iter().map(|v| ctx.bind(v.clone())).collect();
            format!("({})", placeholders.join(", "))
        }
        _ => {
            let r = render_expr(right, ctx);
            format!("({r})")
        }
    }
}

/// 渲染二元运算符。
fn render_binary_op(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Eq => "=",
        BinaryOp::Ne => "<>",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::Like => "LIKE",
        BinaryOp::NotLike => "NOT LIKE",
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::Concat => "||",
        // 以下变体在 render_expr 中单独处理
        BinaryOp::ILike
        | BinaryOp::In
        | BinaryOp::NotIn
        | BinaryOp::IsNull
        | BinaryOp::IsNotNull => unreachable!(),
    }
}

/// 渲染函数调用。
fn render_function(func: Function, args: &[Expr], ctx: &mut RenderCtx<'_>) -> String {
    let arg_sql: Vec<String> = args.iter().map(|a| render_expr(a, ctx)).collect();
    let joined = arg_sql.join(", ");
    match func {
        Function::Count => format!("COUNT({joined})"),
        Function::Sum => format!("SUM({joined})"),
        Function::Avg => format!("AVG({joined})"),
        Function::Min => format!("MIN({joined})"),
        Function::Max => format!("MAX({joined})"),
        Function::Coalesce => format!("COALESCE({joined})"),
        Function::Lower => format!("LOWER({joined})"),
        Function::Upper => format!("UPPER({joined})"),
        Function::Length => format!("LENGTH({joined})"),
        Function::Trim => format!("TRIM({joined})"),
        Function::Now => ctx.dialect.render_now(),
        Function::Custom(name) => format!("{name}({joined})"),
    }
}

//! `sql!` 宏实现。
//!
//! 语法：
//!
//! ```ignore
//! sql!(
//!     "SELECT * FROM users WHERE 1=1",
//!     if let Some(n) = &name_filter { " AND name LIKE " { n } },
//!     #orderBy(created_at desc),
//!     " LIMIT ", { limit },
//! )
//! .find(db).await?
//! ```
//!
//! ## 安全模型
//!
//! - **插值 `{ expr }`**：一律转为占位符 `?` + 绑定参数（`TypeMapper`），
//!   值永不内联进 SQL 文本
//! - **裸表达式**：仅接受 `SqlFragment`（如嵌套的 `sql!` 结果），
//!   拼接时 SQL 文本与参数成对流转移，无注入面
//! - **`#` 指令**：静态字面量内解析，字段名 / 操作符 / 变量名 / 整数
//!   在编译期白名单校验，非法输入直接编译失败
//!
//! 所有片段共享同一个 `__alun_sql` / `__alun_params` 构建序列，
//! 条件插值的参数 push 发生在 `if` 作用域内，占位符与参数天然对齐。

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, LitStr, Token};

use crate::template::{TemplateDirective, TemplatePiece, parse_template};

/// `sql!` 宏的输入。
struct SqlInput {
    /// 片段列表，按出现顺序拼接。
    segments: Vec<Segment>,
}

/// 一个 SQL 片段。
enum Segment {
    /// 静态模板字面量（内含 `#` 指令，编译期解析展开）。
    Template(LitStr),
    /// 插值 `{ expr }`：参数绑定，`expr` 须实现 `TypeMapper`。
    Interpolation(Expr),
    /// 裸表达式：`SqlFragment`（如嵌套 `sql!` 结果），成对拼接。
    Fragment(Expr),
    /// 条件 `if cond { ... }`：条件成立时拼接 body。
    Conditional {
        /// 条件 token（支持 `let Some(x) = opt` 与 bool 表达式）。
        condition: proc_macro2::TokenStream,
        /// 条件为真时的片段。
        body: Vec<Segment>,
    },
}

impl Parse for SqlInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut segments = Vec::new();
        while !input.is_empty() {
            if input.peek(LitStr) {
                segments.push(Segment::Template(input.parse()?));
            } else if input.peek(Token![if]) {
                segments.push(parse_conditional(input)?);
            } else if input.peek(syn::token::Brace) {
                let content;
                syn::braced!(content in input);
                let expr: Expr = content.parse()?;
                segments.push(Segment::Interpolation(expr));
            } else {
                segments.push(Segment::Fragment(input.parse()?));
            }

            // 逗号分隔，可省略
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(SqlInput { segments })
    }
}

/// 解析条件片段：`if cond { 片段... }`，body 内允许嵌套 `if`。
///
/// 条件 token 手工收集到 `{` 为止——syn 的通用 `Expr` 解析器会把
/// `if cond { ... }` 连块一起消费，导致块内的 sql! 片段语法（字面量
/// 后跟插值）解析失败。原始 token 流直接嵌回生成代码，
/// `let Some(x) = opt` 与 bool 表达式均可使用。
fn parse_conditional(input: ParseStream) -> syn::Result<Segment> {
    input.parse::<Token![if]>()?;
    let mut condition = proc_macro2::TokenStream::new();
    while !input.is_empty() && !input.peek(syn::token::Brace) {
        let tt: proc_macro2::TokenTree = input.parse()?;
        condition.extend([tt]);
    }
    if condition.is_empty() {
        return Err(input.error("missing condition before `{`"));
    }
    let content;
    syn::braced!(content in input);
    let mut body = Vec::new();
    while !content.is_empty() {
        if content.peek(LitStr) {
            body.push(Segment::Template(content.parse()?));
        } else if content.peek(Token![if]) {
            body.push(parse_conditional(&content)?);
        } else if content.peek(syn::token::Brace) {
            let inner;
            syn::braced!(inner in content);
            let expr: Expr = inner.parse()?;
            body.push(Segment::Interpolation(expr));
        } else {
            body.push(Segment::Fragment(content.parse()?));
        }

        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
    }
    Ok(Segment::Conditional { condition, body })
}

/// 展开 `sql!` 宏，生成构造 `SqlQuery` 的表达式。
pub fn expand(input: TokenStream) -> syn::Result<TokenStream2> {
    let parsed = syn::parse::<SqlInput>(input)?;
    let statements = expand_segments(&parsed.segments)?;

    Ok(quote! {
        {
            let mut __alun_sql = ::std::string::String::new();
            let mut __alun_params = ::std::vec::Vec::new();
            #( #statements )*
            ::alun_db_core::query::SqlQuery::new(__alun_sql, __alun_params)
        }
    })
}

/// 将片段序列展开为运行时构建语句。
fn expand_segments(segments: &[Segment]) -> syn::Result<Vec<TokenStream2>> {
    let mut out = Vec::new();
    for segment in segments {
        match segment {
            Segment::Template(lit) => {
                // 编译期解析字面量中的静态文本与 `#` 指令
                let pieces =
                    parse_template(&lit.value()).map_err(|msg| syn::Error::new(lit.span(), msg))?;
                out.extend(expand_pieces(&pieces, lit.span())?);
            }
            Segment::Interpolation(expr) => {
                // 插值一律参数化：追加 `?` 并绑定值（IntoValue 额外覆盖 &str）
                out.push(quote! {
                    __alun_sql.push('?');
                    __alun_params.push(
                        ::alun_db_core::query::IntoValue::into_value(#expr)
                    );
                });
            }
            Segment::Fragment(expr) => {
                // 裸表达式仅限 SqlFragment：SQL 文本与参数成对流转移
                out.push(quote! {
                    ::alun_db_core::query::SqlFragment::push_into(
                        #expr, &mut __alun_sql, &mut __alun_params
                    );
                });
            }
            Segment::Conditional { condition, body } => {
                // 条件成立时执行 body 构建语句，插值参数在作用域内对齐
                let body_stmts = expand_segments(body)?;
                out.push(quote! {
                    if #condition {
                        #( #body_stmts )*
                    }
                });
            }
        }
    }
    Ok(out)
}

/// 展开模板片段序列（文本 / 指令 / `#if` 条件块，条件块递归）。
fn expand_pieces(
    pieces: &[TemplatePiece],
    span: proc_macro2::Span,
) -> syn::Result<Vec<TokenStream2>> {
    let mut out = Vec::new();
    for piece in pieces {
        match piece {
            TemplatePiece::Text(text) => out.push(quote! { __alun_sql.push_str(#text); }),
            TemplatePiece::Directive(directive) => {
                out.push(expand_directive(directive.clone())?);
            }
            TemplatePiece::If {
                cond,
                then,
                otherwise,
            } => {
                // 条件源码在编译期解析为 Rust 表达式（作用域即调用方作用域）
                let cond_expr: Expr = syn::parse_str(cond).map_err(|e| {
                    syn::Error::new(span, format!("invalid `#if` condition `{cond}`: {e}"))
                })?;
                let then_stmts = expand_pieces(then, span)?;
                let else_stmts = expand_pieces(otherwise, span)?;
                out.push(quote! {
                    if #cond_expr {
                        #( #then_stmts )*
                    } else {
                        #( #else_stmts )*
                    }
                });
            }
        }
    }
    Ok(out)
}

/// 将单条指令展开为运行时构建语句。
fn expand_directive(directive: TemplateDirective) -> syn::Result<TokenStream2> {
    match directive {
        TemplateDirective::Where { field, op, param } => {
            condition_stmt("WHERE", &field, &op, &param)
        }
        TemplateDirective::And { field, op, param } => condition_stmt("AND", &field, &op, &param),
        TemplateDirective::Or { field, op, param } => condition_stmt("OR", &field, &op, &param),
        TemplateDirective::Para { name } => {
            let ident: syn::Ident = syn::parse_str(&name).map_err(|_| {
                syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("invalid param variable `{name}`"),
                )
            })?;
            Ok(quote! {
                __alun_sql.push('?');
                __alun_params.push(
                    ::alun_db_core::query::IntoValue::into_value(#ident)
                );
            })
        }
        TemplateDirective::OrderBy { clauses } => {
            let rendered = clauses
                .iter()
                .map(|(field, dir)| format!("{} {}", field, dir.as_keyword()))
                .collect::<Vec<_>>()
                .join(", ");
            let text = format!(" ORDER BY {rendered}");
            Ok(quote! { __alun_sql.push_str(#text); })
        }
        TemplateDirective::GroupBy { fields } => {
            let text = format!(" GROUP BY {}", fields.join(", "));
            Ok(quote! { __alun_sql.push_str(#text); })
        }
        TemplateDirective::Limit { n } => {
            let text = format!(" LIMIT {n}");
            Ok(quote! { __alun_sql.push_str(#text); })
        }
    }
}

/// 展开条件指令（`#where` / `#and` / `#or`）。
///
/// 生成 `if let Some(ref v) = param { 拼接条件 + 绑定参数 }`：
/// `param` 必须是 `Option<T: TypeMapper>`，`None` 时整个条件省略。
fn condition_stmt(keyword: &str, field: &str, op: &str, param: &str) -> syn::Result<TokenStream2> {
    // template 层已校验标识符格式，这里防御性转 Ident
    let ident: syn::Ident = syn::parse_str(param).map_err(|_| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("invalid param variable `{param}`"),
        )
    })?;
    let text = format!(" {keyword} {field} {op} ?");
    Ok(quote! {
        if let ::core::option::Option::Some(ref v) = #ident {
            __alun_sql.push_str(#text);
            __alun_params.push(::alun_db_core::traits::TypeMapper::to_value(v));
        }
    })
}

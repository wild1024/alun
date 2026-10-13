//! `#` 指令模板解析（编译期）。
//!
//! 解析 `sql!` 静态字面量中的指令，输出片段序列，供 [`crate::sql_macro`]
//! 展开为运行时拼接代码。所有指令参数（字段名、操作符、变量名、整数）在本层
//! 完成编译期校验。
//!
//! ## 指令一览
//!
//! - `#where(field, op, param)` / `#and(...)` / `#or(...)` —— 条件比较，
//!   `param` 为 `Option<T>` 变量，`None` 时整个条件省略
//! - `#orderBy(field [asc|desc], ...)` —— 排序
//! - `#groupBy(field, ...)` —— 分组
//! - `#limit(n)` —— 行数上限
//! - `#if(cond)` / `#else` / `#end` —— 条件块，可嵌套；
//!   `cond` 是 Rust 布尔表达式，在编译期解析、调用方作用域求值
//! - `#para(name)` —— 把作用域内变量绑定为占位参数（字面量内的插值入口）

/// 排序方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    /// 升序 `ASC`。
    Asc,
    /// 降序 `DESC`。
    Desc,
}

impl SortDirection {
    /// 输出 SQL 关键字。
    pub fn as_keyword(&self) -> &'static str {
        match self {
            SortDirection::Asc => "ASC",
            SortDirection::Desc => "DESC",
        }
    }
}

/// 模板片段：静态文本、`#` 指令或 `#if` 条件块。
#[derive(Debug, Clone)]
pub enum TemplatePiece {
    /// 静态 SQL 文本（原样保留，含空白）。
    Text(String),
    /// `#` 指令。
    Directive(TemplateDirective),
    /// `#if(cond) ... #else ... #end` 条件块。
    If {
        /// 条件表达式（Rust 表达式源码，编译期解析为 `syn::Expr`）。
        cond: String,
        /// 条件成立时拼接的片段。
        then: Vec<TemplatePiece>,
        /// `#else` 分支（可选）。
        otherwise: Vec<TemplatePiece>,
    },
}

/// 模板指令。
#[derive(Debug, Clone)]
pub enum TemplateDirective {
    /// `#where(field, op, param)` —— 首个条件；param 为 `None` 时整个条件省略。
    Where {
        /// 列名（静态标识符）。
        field: String,
        /// 比较操作符（白名单校验后归一为大写）。
        op: String,
        /// 调用方作用域中的 `Option<T>` 变量名。
        param: String,
    },
    /// `#and(field, op, param)` —— 追加 AND 条件。
    And {
        /// 列名。
        field: String,
        /// 比较操作符。
        op: String,
        /// `Option<T>` 变量名。
        param: String,
    },
    /// `#or(field, op, param)` —— 追加 OR 条件。
    Or {
        /// 列名。
        field: String,
        /// 比较操作符。
        op: String,
        /// `Option<T>` 变量名。
        param: String,
    },
    /// `#orderBy(field [asc|desc], ...)` —— 排序，方向缺省为 ASC。
    OrderBy {
        /// 列名与方向的列表。
        clauses: Vec<(String, SortDirection)>,
    },
    /// `#groupBy(field, ...)` —— 分组。
    GroupBy {
        /// 列名列表。
        fields: Vec<String>,
    },
    /// `#limit(n)` —— 限制行数。
    Limit {
        /// 行数上限。
        n: u64,
    },
    /// `#para(name)` —— 把作用域内变量绑定为占位参数。
    ///
    /// 字面量内无法使用 `{ expr }` 插值，`#para` 就是字符串内的插值入口，
    /// 典型用法是与 `#if(...)` 配合按条件拼接参数化条件。
    Para {
        /// 调用方作用域中的变量名。
        name: String,
    },
}

/// 条件指令允许的比较操作符（大写归一后匹配）。
const ALLOWED_OPS: [&str; 10] = [
    "=", "!=", "<>", ">", ">=", "<", "<=", "LIKE", "NOT LIKE", "ILIKE",
];

/// `#if` 块解析栈帧。
struct Frame {
    /// 条件表达式源码；根帧为 `None`。
    cond: Option<String>,
    /// 条件成立时拼接的片段。
    then: Vec<TemplatePiece>,
    /// `#else` 分支片段。
    otherwise: Option<Vec<TemplatePiece>>,
}

impl Frame {
    /// 根帧：承载整段模板的片段列表。
    fn root() -> Self {
        Frame {
            cond: None,
            then: Vec::new(),
            otherwise: None,
        }
    }

    /// 分支帧：`#if(cond)` 开始。
    fn branch(cond: String) -> Self {
        Frame {
            cond: Some(cond),
            then: Vec::new(),
            otherwise: None,
        }
    }
}

/// 当前接收片段的列表：处于 `#else` 分支则写 `otherwise`，否则写 `then`。
fn target(stack: &mut [Frame]) -> &mut Vec<TemplatePiece> {
    let frame = stack.last_mut().expect("stack always holds the root frame");
    match frame.otherwise.as_mut() {
        Some(branch) => branch,
        None => &mut frame.then,
    }
}

/// 把当前累积的静态文本冲刷进片段列表。
fn flush_text(text: &mut String, stack: &mut [Frame]) {
    if !text.is_empty() {
        target(stack).push(TemplatePiece::Text(std::mem::take(text)));
    }
}

/// 处理 `#else`：进入当前 `#if` 的 else 分支。
fn open_else(stack: &mut [Frame]) -> Result<(), String> {
    if stack.len() < 2 {
        return Err("`#else` outside of `#if`".into());
    }
    let frame = stack.last_mut().expect("checked len >= 2");
    if frame.otherwise.is_some() {
        return Err("duplicate `#else` in `#if`".into());
    }
    frame.otherwise = Some(Vec::new());
    Ok(())
}

/// 处理 `#end`：闭合当前 `#if`，把条件块挂到父帧。
fn close_if(stack: &mut Vec<Frame>) -> Result<(), String> {
    if stack.len() < 2 {
        return Err("`#end` outside of `#if`".into());
    }
    let frame = stack.pop().expect("checked len >= 2");
    let cond = frame.cond.expect("non-root frame carries a condition");
    target(stack).push(TemplatePiece::If {
        cond,
        then: frame.then,
        otherwise: frame.otherwise.unwrap_or_default(),
    });
    Ok(())
}

/// 解析模板字符串，返回文本 / 指令 / 条件块序列。
///
/// 单引号字符串字面量内的 `#` 不解析；`#if(...)` / `#else` / `#end` 可嵌套，
/// 未闭合的 `#if` 是编译期错误。
pub fn parse_template(template: &str) -> Result<Vec<TemplatePiece>, String> {
    let chars = template.chars().collect::<Vec<_>>();
    let mut stack = vec![Frame::root()];
    let mut text = String::new();
    // 是否处于单引号字符串字面量内（'' 转义单引号）
    let mut in_quote = false;
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            in_quote = !in_quote;
            text.push(c);
            i += 1;
            continue;
        }
        // 引号外遇到 # 视为指令起点
        if !in_quote && c == '#' {
            // 读指令名
            let mut name = String::new();
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                name.push(chars[j]);
                j += 1;
            }
            // 块边界指令不带括号
            if name == "end" || name == "else" {
                flush_text(&mut text, &mut stack);
                if name == "else" {
                    open_else(&mut stack)?;
                } else {
                    close_if(&mut stack)?;
                }
                i = j;
                continue;
            }
            if j >= chars.len() || chars[j] != '(' {
                return Err(format!(
                    "unknown template directive: `#{name}` (expected `#name(...)`)"
                ));
            }
            // 读取括号内容（处理嵌套括号）
            let mut depth = 1;
            let mut content = String::new();
            j += 1;
            while j < chars.len() {
                match chars[j] {
                    '(' => {
                        depth += 1;
                        content.push('(');
                    }
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        content.push(')');
                    }
                    ch => content.push(ch),
                }
                j += 1;
            }
            if depth != 0 {
                return Err(format!("unbalanced parenthesis in `#{name}` directive"));
            }

            // 先冲刷已积累的静态文本
            flush_text(&mut text, &mut stack);
            if name == "if" {
                let cond = content.trim().to_string();
                if cond.is_empty() {
                    return Err("`#if` expects a condition, e.g. `#if(name)`".into());
                }
                stack.push(Frame::branch(cond));
            } else {
                let directive = parse_directive(&name, &content)?;
                target(&mut stack).push(TemplatePiece::Directive(directive));
            }
            i = j + 1;
        } else {
            text.push(c);
            i += 1;
        }
    }
    if in_quote {
        return Err("unterminated string literal in template".into());
    }
    flush_text(&mut text, &mut stack);
    if stack.len() != 1 {
        return Err("unclosed `#if` (missing `#end`)".into());
    }
    Ok(stack.pop().expect("root frame").then)
}

/// 解析单个指令的参数。
fn parse_directive(name: &str, content: &str) -> Result<TemplateDirective, String> {
    match name {
        "where" | "and" | "or" => {
            let parts: Vec<&str> = content.split(',').map(|s| s.trim()).collect();
            if parts.len() != 3 {
                return Err(format!("`#{name}` expects 3 arguments: field, op, param"));
            }
            let field = parts[0].to_string();
            let op_raw = strip_quotes(parts[1]);
            let param = parts[2].to_string();
            validate_identifier(&field)?;
            validate_identifier(&param)?;
            let op_upper = op_raw.to_ascii_uppercase();
            if !ALLOWED_OPS.contains(&op_upper.as_str()) {
                return Err(format!(
                    "`#{name}` unsupported operator `{op_raw}`; allowed: {}",
                    ALLOWED_OPS.join(", ")
                ));
            }
            let directive = match name {
                "where" => TemplateDirective::Where {
                    field,
                    op: op_upper,
                    param,
                },
                "and" => TemplateDirective::And {
                    field,
                    op: op_upper,
                    param,
                },
                _ => TemplateDirective::Or {
                    field,
                    op: op_upper,
                    param,
                },
            };
            Ok(directive)
        }
        "orderBy" => {
            let mut clauses = Vec::new();
            for clause in content.split(',') {
                let tokens: Vec<&str> = clause.split_whitespace().collect();
                match tokens.as_slice() {
                    [field] => {
                        validate_identifier(field)?;
                        clauses.push((field.to_string(), SortDirection::Asc));
                    }
                    [field, dir] => {
                        validate_identifier(field)?;
                        let direction = match dir.to_ascii_lowercase().as_str() {
                            "asc" => SortDirection::Asc,
                            "desc" => SortDirection::Desc,
                            other => {
                                return Err(format!(
                                    "`#orderBy` invalid direction `{other}`; expected asc/desc"
                                ));
                            }
                        };
                        clauses.push((field.to_string(), direction));
                    }
                    _ => {
                        return Err("`#orderBy` expects `field [asc|desc]` clauses".into());
                    }
                }
            }
            if clauses.is_empty() {
                return Err("`#orderBy` expects at least one clause".into());
            }
            Ok(TemplateDirective::OrderBy { clauses })
        }
        "groupBy" => {
            let mut fields = Vec::new();
            for field in content.split(',') {
                let field = field.trim();
                validate_identifier(field)?;
                fields.push(field.to_string());
            }
            if fields.is_empty() {
                return Err("`#groupBy` expects at least one field".into());
            }
            Ok(TemplateDirective::GroupBy { fields })
        }
        "limit" => {
            let n = content
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("`#limit` expects a non-negative integer, got `{content}`"))?;
            Ok(TemplateDirective::Limit { n })
        }
        "para" => {
            let name = content.trim().to_string();
            validate_identifier(&name)?;
            Ok(TemplateDirective::Para { name })
        }
        _ => Err(format!("unknown template directive: `#{name}`")),
    }
}

/// 去除操作符参数两侧的成对单/双引号（如 `'='` → `=`）。
fn strip_quotes(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        let first = bytes[0];
        let last = bytes[s.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

/// 校验静态标识符（列名 / 变量名）：`[a-zA-Z_][a-zA-Z0-9_]*`。
fn validate_identifier(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return Err(format!("invalid identifier `{name}`")),
    }
    if chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(format!("invalid identifier `{name}`"))
    }
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_text() {
        let pieces = parse_template("SELECT * FROM users").unwrap();
        assert_eq!(pieces.len(), 1);
        assert!(matches!(&pieces[0], TemplatePiece::Text(t) if t == "SELECT * FROM users"));
    }

    #[test]
    fn parses_where_directive() {
        let pieces = parse_template("#where(status, '=', status)").unwrap();
        assert_eq!(pieces.len(), 1);
        match &pieces[0] {
            TemplatePiece::Directive(TemplateDirective::Where { field, op, param }) => {
                assert_eq!(field, "status");
                assert_eq!(op, "=");
                assert_eq!(param, "status");
            }
            other => panic!("unexpected piece: {other:?}"),
        }
    }

    #[test]
    fn mixes_text_and_directives() {
        let pieces =
            parse_template("SELECT * FROM users #where(a, '>', x) #orderBy(id desc) #limit(10)")
                .unwrap();
        // 指令间空格保留为独立 Text：Text, Directive, Text, Directive, Text, Directive
        assert_eq!(pieces.len(), 6);
        assert!(matches!(&pieces[0], TemplatePiece::Text(t) if t == "SELECT * FROM users "));
        assert!(matches!(&pieces[3], TemplatePiece::Directive(
            TemplateDirective::OrderBy { clauses }
        ) if clauses[0] == ("id".to_string(), SortDirection::Desc)));
    }

    #[test]
    fn ignores_hash_inside_string_literal() {
        let pieces = parse_template("WHERE name = '#literal'").unwrap();
        assert_eq!(pieces.len(), 1);
        assert!(matches!(&pieces[0], TemplatePiece::Text(t) if t.contains("#literal")));
    }

    #[test]
    fn rejects_unknown_directive() {
        assert!(parse_template("#foo(1)").is_err());
        assert!(parse_template("#where(a)").is_err());
    }

    #[test]
    fn rejects_invalid_identifier() {
        assert!(parse_template("#where(a; DROP TABLE x, '=', v)").is_err());
        assert!(parse_template("#orderBy(id; --)").is_err());
    }

    #[test]
    fn rejects_invalid_op_and_limit() {
        assert!(parse_template("#where(a, 'LIKE IN', v)").is_err());
        assert!(parse_template("#limit(-1)").is_err());
        assert!(parse_template("#limit(abc)").is_err());
    }

    #[test]
    fn parses_if_block() {
        let pieces = parse_template("SELECT 1 #if(a) AND b = 1 #end").unwrap();
        assert_eq!(pieces.len(), 2);
        match &pieces[1] {
            TemplatePiece::If {
                cond,
                then,
                otherwise,
            } => {
                assert_eq!(cond, "a");
                assert!(otherwise.is_empty());
                assert!(matches!(&then[0], TemplatePiece::Text(t) if t == " AND b = 1 "));
            }
            other => panic!("unexpected piece: {other:?}"),
        }
    }

    #[test]
    fn parses_nested_if_else() {
        let pieces = parse_template("#if(a) 1 #else #if(b) 2 #end #end").unwrap();
        assert_eq!(pieces.len(), 1);
        let TemplatePiece::If { otherwise, .. } = &pieces[0] else {
            panic!("expected if block");
        };
        assert!(
            otherwise
                .iter()
                .any(|p| matches!(p, TemplatePiece::If { .. }))
        );
    }

    #[test]
    fn rejects_unbalanced_if() {
        assert!(parse_template("#if(a) x").is_err());
        assert!(parse_template("#end").is_err());
        assert!(parse_template("#else").is_err());
        assert!(parse_template("#if(a) 1 #else 2 #else 3 #end").is_err());
        assert!(parse_template("#if() x #end").is_err());
    }

    #[test]
    fn parses_para_directive() {
        let pieces = parse_template("#if(s) AND status = #para(s) #end").unwrap();
        let TemplatePiece::If { then, .. } = &pieces[0] else {
            panic!("expected if block");
        };
        assert!(then.iter().any(|p| matches!(
            p,
            TemplatePiece::Directive(TemplateDirective::Para { name }) if name == "s"
        )));
        assert!(parse_template("#para(1bad)").is_err());
    }
}

//! SQL 渲染核心。

pub mod delete;
pub mod expr;
pub mod insert;
pub mod json;
pub mod placeholder;
pub mod scope;
pub mod select;
pub mod statement;
pub mod update;

use alun_db_core::error::DbError;

/// 渲染标识符（表名、列名）。
///
/// 各数据库对标识符的引用方式不同：
/// - PostgreSQL / SQLite：双引号 `"name"`
/// - MySQL：反引号 `` `name` ``
///
/// 渲染器在输出标识符前会校验其格式，仅允许
/// `[a-zA-Z_][a-zA-Z0-9_]*`，防止 SQL 注入。
pub fn render_identifier(name: &str, quote: QuoteStyle) -> Result<String, DbError> {
    if !is_valid_identifier(name) {
        return Err(DbError::InvalidValue(format!(
            "invalid identifier: `{name}`"
        )));
    }
    Ok(match quote {
        QuoteStyle::Double => format!("\"{name}\""),
        QuoteStyle::Backtick => format!("`{name}`"),
        QuoteStyle::None => name.to_string(),
    })
}

/// 标识符引用风格。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum QuoteStyle {
    /// 双引号。PostgreSQL、SQLite。
    Double,
    /// 反引号。MySQL。
    Backtick,
    /// 不引用。
    None,
}

/// 校验标识符是否合法。
///
/// Builder 等接受运行时表名 / 列名的入口必须在拼接前调用本校验，
/// 防止通过标识符注入 SQL。AST 路径的标识符来自宏生成的静态字符串，
/// 天然满足本规则。
pub fn validate_identifier(name: &str) -> Result<(), DbError> {
    if is_valid_identifier(name) {
        Ok(())
    } else {
        Err(DbError::InvalidValue(format!(
            "invalid identifier: `{name}`"
        )))
    }
}

/// 校验标识符是否合法。
fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

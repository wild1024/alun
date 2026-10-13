//! 占位符改写工具。
//!
//! `stmt::Value` 体系的语句统一使用 `?` 占位符书写；PostgreSQL 等方言
//! 要求 `$1, $2, ...` 风格。本工具在执行前按方言完成改写，供 facade
//! 与各驱动共用，保证 `sql!` 宏路径与手写路径行为一致。

use alun_db_core::traits::dialect::{Dialect, Placeholder};

/// 按方言改写占位符：`?` 风格原样返回，`$n` 风格重写为 `$1`、`$2`…
///
/// 单引号字符串字面量内的 `?` 不参与改写（`''` 转义按两次进出处理）。
///
/// # 参数
///
/// - `sql`：使用 `?` 占位符的 SQL 文本
/// - `dialect`：目标方言
///
/// # 返回值
///
/// 改写后的 SQL。本函数无失败路径。
pub fn rewrite_placeholders(sql: &str, dialect: &dyn Dialect) -> String {
    if dialect.capability().placeholder == Placeholder::Question {
        return sql.to_string();
    }
    let mut out = String::with_capacity(sql.len() + 8);
    let mut index = 0usize;
    let mut in_quote = false;
    for c in sql.chars() {
        match c {
            '\'' => {
                in_quote = !in_quote;
                out.push(c);
            }
            '?' if !in_quote => {
                index += 1;
                out.push_str(&format!("${index}"));
            }
            _ => out.push(c),
        }
    }
    out
}

/// 按方言改写占位符（SQL 已拥有所有权，能复用则不再拷贝）。
///
/// SQLite 等直接使用 `?` 占位符的方言原样返回入参，省掉整串拷贝；
/// 需要改写的方言转交 [`rewrite_placeholders`]。
///
/// # 参数
///
/// - `sql`：使用 `?` 占位符的 SQL 文本
/// - `dialect`：目标方言
///
/// # 返回值
///
/// 改写后的 SQL。本函数无失败路径。
pub fn rewrite_placeholders_owned(sql: String, dialect: &dyn Dialect) -> String {
    if dialect.capability().placeholder == Placeholder::Question {
        sql
    } else {
        rewrite_placeholders(&sql, dialect)
    }
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialects::postgres::PostgresDialect;
    use crate::dialects::sqlite::SqliteDialect;

    #[test]
    fn rewrites_question_to_dollar() {
        let sql = rewrite_placeholders("WHERE a = ? AND b IN (?, ?)", &PostgresDialect);
        assert_eq!(sql, "WHERE a = $1 AND b IN ($2, $3)");
    }

    #[test]
    fn keeps_question_in_string_literal() {
        let sql = rewrite_placeholders("WHERE a = 'a?b' AND c = ?", &PostgresDialect);
        assert_eq!(sql, "WHERE a = 'a?b' AND c = $1");
    }

    #[test]
    fn keeps_question_style_as_is() {
        let sql = rewrite_placeholders("WHERE a = ? AND b = ?", &SqliteDialect);
        assert_eq!(sql, "WHERE a = ? AND b = ?");
    }
}

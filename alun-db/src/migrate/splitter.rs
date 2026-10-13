//! 方言感知的语句拆分器。
//!
//! 迁移文件普遍是**多语句脚本**（真实项目实测：jinhu 的 `01_base_tables_init.sql`
//! 单文件 856 条语句，含 plpgsql 函数体）。执行层要把脚本拆成语句序列逐条执行，
//! 因此拆分必须正确识别各种「分号不是语句边界」的上下文，否则会把 SQL 切坏。
//!
//! 覆盖的上下文：
//! - 单引号字符串（`''` 双写转义；MySQL 额外支持 `\` 转义）
//! - 双引号标识符、反引号标识符（MySQL）
//! - `--` 行注释；MySQL 额外支持 `#` 行注释
//! - `/* */` 块注释（支持嵌套，PostgreSQL 语义）
//! - PostgreSQL `$tag$…$tag$` 美元引用（plpgsql 函数体实测必需）
//! - `;` 语句分隔符，可被 `-- +delimiter` 指令覆盖
//!
//! 拆分是纯函数，不依赖数据库连接，可独立复用于数据导入等场景。

use alun_db_core::error::{DbError, Result};

use super::kind::DialectKind;

/// 脚本中的指令注释。
///
/// 指令写在行注释里、紧跟 `+`，就近生效（通常是文件头）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Directives {
    /// 是否禁用事务包装（`-- +no-transaction`）。
    ///
    /// 适用于 PostgreSQL `CREATE INDEX CONCURRENTLY`、MySQL DDL（隐式提交）
    /// 这类无法进事务的语句。
    pub is_no_transaction: bool,
    /// 自定义语句分隔符（`-- +delimiter //`），未指定时为 `None`。
    pub delimiter: Option<String>,
}

/// 单文件双段式（`-- +migrate Up` / `-- +migrate Down`）的分段结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sections {
    /// 正向脚本；无指令注释时即整个文件内容。
    pub up: String,
    /// 回退脚本；未声明 `-- +migrate Down` 时为空串。
    pub down: String,
}

/// 把 SQL 脚本拆分为语句序列。
///
/// # 参数
///
/// - `sql`：待拆分的脚本（单个迁移文件的全部内容）。
/// - `dialect`：方言类别，决定转义与注释规则。
///
/// # 返回
///
/// 语句序列（已去除首尾空白）；仅含注释与空白的片段被丢弃。
///
/// # 错误
///
/// `-- +delimiter` 指令给出空分隔符时返回 [`DbError::Migration`]。
pub fn split_statements(sql: &str, dialect: DialectKind) -> Result<Vec<String>> {
    let directives = parse_directives(sql);
    let delimiter: Vec<char> = match &directives.delimiter {
        Some(custom) => custom.chars().collect(),
        None => vec![';'],
    };
    if delimiter.is_empty() {
        return Err(DbError::Migration(
            "migration directive `-- +delimiter` must specify a non-empty delimiter".into(),
        ));
    }

    let chars: Vec<char> = sql.chars().collect();
    let mut statements = Vec::new();
    let mut current = String::new();
    // 当前片段是否含有真正的 SQL 代码（注释与空白不算）
    let mut has_code = false;
    let mut index = 0_usize;

    while index < chars.len() {
        let ch = chars[index];

        // `--` 行注释：整段原样保留（MySQL 的 `/*!...*/` 可执行注释另有处理）
        if ch == '-' && chars.get(index + 1) == Some(&'-') {
            index = copy_line_comment(&chars, index, &mut current);
            continue;
        }
        // `#` 行注释：仅 MySQL
        if ch == '#' && dialect == DialectKind::Mysql {
            index = copy_line_comment(&chars, index, &mut current);
            continue;
        }
        // `/* */` 块注释（支持嵌套）
        if ch == '/' && chars.get(index + 1) == Some(&'*') {
            index = copy_block_comment(&chars, index, &mut current);
            continue;
        }
        // 引号包裹片段：单引号字符串 / 双引号与反引号标识符
        if let Some(quote) = match ch {
            '\'' => Some('\''),
            '"' => Some('"'),
            '`' if dialect == DialectKind::Mysql => Some('`'),
            _ => None,
        } {
            index = copy_quoted(
                &chars,
                index,
                &mut current,
                quote,
                dialect == DialectKind::Mysql,
            );
            has_code = true;
            continue;
        }
        // PostgreSQL 美元引用：`$tag$ ... $tag$` 整体是一个词法单元
        if ch == '$'
            && dialect == DialectKind::Postgres
            && let Some(end) = dollar_quote_end(&chars, index)
        {
            while index < end {
                current.push(chars[index]);
                index += 1;
            }
            has_code = true;
            continue;
        }
        // 语句分隔符（顶层）
        if starts_with(&chars, index, &delimiter) {
            if has_code {
                statements.push(current.trim().to_string());
            }
            current.clear();
            has_code = false;
            index += delimiter.len();
            continue;
        }

        if !ch.is_whitespace() {
            has_code = true;
        }
        current.push(ch);
        index += 1;
    }

    // 结尾没有分隔符的最后一条语句
    if has_code {
        statements.push(current.trim().to_string());
    }
    Ok(statements)
}

/// 解析脚本中的指令注释。
///
/// # 参数
///
/// - `sql`：脚本内容。
///
/// # 返回
///
/// 提取到的指令；未声明任何指令时返回默认值（不禁事务、默认 `;` 分隔）。
pub fn parse_directives(sql: &str) -> Directives {
    let mut directives = Directives::default();
    for line in sql.lines() {
        let Some(comment) = line.trim_start().strip_prefix("--") else {
            continue;
        };
        let Some(directive) = comment.trim_start().strip_prefix('+') else {
            continue;
        };
        let mut parts = directive.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("").trim().to_ascii_lowercase();
        let value = parts.next().unwrap_or("").trim();
        match name.as_str() {
            "no-transaction" => directives.is_no_transaction = true,
            "delimiter" if !value.is_empty() => directives.delimiter = Some(value.to_string()),
            _ => {}
        }
    }
    directives
}

/// 按 `-- +migrate Up` / `-- +migrate Down` 指令把单文件切成上下两段。
///
/// 无任何 `+migrate` 指令时，整段内容视为 up（缺省单文件即单向迁移）。
///
/// # 参数
///
/// - `sql`：单文件内容。
///
/// # 返回
///
/// 分段结果；指令注释行本身不进入任何一段。
pub fn split_sections(sql: &str) -> Sections {
    let mut up = String::new();
    let mut down = String::new();
    let mut in_down = false;

    for line in sql.lines() {
        if let Some(section) = migrate_marker(line) {
            in_down = section == SectionMarker::Down;
            continue;
        }
        let target = if in_down { &mut down } else { &mut up };
        target.push_str(line);
        target.push('\n');
    }

    Sections { up, down }
}

/// `-- +migrate` 指令指向的段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SectionMarker {
    /// `-- +migrate Up`
    Up,
    /// `-- +migrate Down`
    Down,
}

/// 判断一行是否为 `-- +migrate Up/Down` 指令（大小写不敏感）。
fn migrate_marker(line: &str) -> Option<SectionMarker> {
    let comment = line.trim_start().strip_prefix("--")?;
    let rest = comment.trim_start().strip_prefix('+')?;
    let mut parts = rest.split_whitespace();
    let name = parts.next()?;
    if !name.eq_ignore_ascii_case("migrate") {
        return None;
    }
    let direction = parts.next()?;
    if direction.eq_ignore_ascii_case("up") {
        Some(SectionMarker::Up)
    } else if direction.eq_ignore_ascii_case("down") {
        Some(SectionMarker::Down)
    } else {
        None
    }
}

/// 复制一整行注释（到行尾，不含换行符本身），返回新的扫描下标。
fn copy_line_comment(chars: &[char], start: usize, out: &mut String) -> usize {
    let mut index = start;
    while index < chars.len() && chars[index] != '\n' {
        out.push(chars[index]);
        index += 1;
    }
    index
}

/// 复制一个块注释（支持嵌套），返回新的扫描下标。
///
/// 未闭合时复制到脚本末尾——宁可把后续内容并入本片段，也不越界 panic。
fn copy_block_comment(chars: &[char], start: usize, out: &mut String) -> usize {
    let mut index = start;
    let mut depth = 0_usize;
    while index < chars.len() {
        if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
            depth += 1;
            out.push('/');
            out.push('*');
            index += 2;
            continue;
        }
        if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
            out.push('*');
            out.push('/');
            index += 2;
            depth -= 1;
            if depth == 0 {
                break;
            }
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    index
}

/// 复制一个引号包裹的片段（含起止引号），返回新的扫描下标。
///
/// `backslash_escapes` 为真时按 MySQL 规则处理 `\` 转义；所有方言都支持
/// 引号双写（`''` / `""` / ` `` `）转义。
fn copy_quoted(
    chars: &[char],
    start: usize,
    out: &mut String,
    quote: char,
    backslash_escapes: bool,
) -> usize {
    let mut index = start;
    out.push(chars[index]);
    index += 1;

    while index < chars.len() {
        let ch = chars[index];
        if backslash_escapes && ch == '\\' {
            out.push(ch);
            index += 1;
            if index < chars.len() {
                out.push(chars[index]);
                index += 1;
            }
            continue;
        }
        out.push(ch);
        index += 1;
        if ch == quote {
            // 引号双写表示字面引号，仍留在字符串内
            if chars.get(index) == Some(&quote) {
                out.push(chars[index]);
                index += 1;
                continue;
            }
            break;
        }
    }
    index
}

/// 若 `chars[start]` 处是 PostgreSQL 美元引用开头（`$tag$`），返回引用结束后的下标。
///
/// 判定规则：`$` 后跟「不以数字开头」的标签（可空）再跟 `$`。因此 `$1`（占位符）
/// 不会被误判为美元引用。
///
/// # 返回
///
/// `Some(end)`：`end` 为整段引用结束后的下标；未闭合时返回脚本末尾。
/// `None`：此处不是美元引用开头。
fn dollar_quote_end(chars: &[char], start: usize) -> Option<usize> {
    let mut cursor = start + 1;
    if chars.get(cursor).is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    while cursor < chars.len() && (chars[cursor].is_ascii_alphanumeric() || chars[cursor] == '_') {
        cursor += 1;
    }
    if chars.get(cursor) != Some(&'$') {
        return None;
    }

    let tag = &chars[start..=cursor];
    let mut probe = cursor + 1;
    while probe + tag.len() <= chars.len() {
        if &chars[probe..probe + tag.len()] == tag {
            return Some(probe + tag.len());
        }
        probe += 1;
    }
    Some(chars.len())
}

/// 判断 `chars[at..]` 是否以 `pattern` 开头。
fn starts_with(chars: &[char], at: usize, pattern: &[char]) -> bool {
    chars.len() >= at + pattern.len() && &chars[at..at + pattern.len()] == pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 基本拆分：多条语句按 `;` 切开，空片段丢弃。
    #[test]
    fn splits_simple_statements() {
        let sql = "CREATE TABLE a (id INT);\n\nINSERT INTO a VALUES (1);\n";
        let out = split_statements(sql, DialectKind::Sqlite).expect("split");
        assert_eq!(
            out,
            vec!["CREATE TABLE a (id INT)", "INSERT INTO a VALUES (1)"]
        );
    }

    /// 分号出现在字符串 / 标识符 / 注释里时不得作为语句边界。
    #[test]
    fn ignores_delimiters_inside_literals_and_comments() {
        let sql = "\
INSERT INTO t VALUES ('a;b');\n\
INSERT INTO t VALUES (\"col;name\");\n\
-- 这是注释; 不是语句\n\
SELECT 1;\n\
/* 块注释; 也不是语句 */\n\
SELECT 2;";
        let out = split_statements(sql, DialectKind::Postgres).expect("split");
        assert_eq!(out.len(), 4, "got {out:?}");
        assert!(out[0].contains("'a;b'"));
        assert!(out[1].contains("\"col;name\""));
        assert!(out[2].contains("SELECT 1"));
        assert!(out[3].contains("SELECT 2"));
    }

    /// PostgreSQL 美元引用：`$$ … $$` 内的分号绝不能被当作语句边界。
    ///
    /// 这是 jinhu `01_base_tables_init.sql` 能加载的前提。
    #[test]
    fn keeps_dollar_quoted_function_body_intact() {
        let sql = "\
CREATE FUNCTION bump() RETURNS void AS $$\n\
BEGIN\n\
    INSERT INTO t VALUES (1);\n\
    INSERT INTO t VALUES (2);\n\
END;\n\
$$ LANGUAGE plpgsql;\n\
SELECT 1;";
        let out = split_statements(sql, DialectKind::Postgres).expect("split");
        assert_eq!(out.len(), 2, "函数体必须整体成为一条语句: {out:?}");
        assert!(out[0].starts_with("CREATE FUNCTION bump()"));
        assert!(out[0].contains("INSERT INTO t VALUES (2);"));
        assert!(out[0].ends_with("$$ LANGUAGE plpgsql"));
        assert_eq!(out[1], "SELECT 1");
    }

    /// 带标签的美元引用同样整体保留。
    #[test]
    fn keeps_tagged_dollar_quote_intact() {
        let sql =
            "CREATE FUNCTION f() RETURNS int AS $body$ SELECT 1; $body$ LANGUAGE sql;\nSELECT 2;";
        let out = split_statements(sql, DialectKind::Postgres).expect("split");
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out[0].contains("SELECT 1;"));
    }

    /// `$1` 占位符不得被误判为美元引用。
    #[test]
    fn does_not_treat_placeholder_as_dollar_quote() {
        let sql = "SELECT * FROM t WHERE id = $1;\nSELECT $2;";
        let out = split_statements(sql, DialectKind::Postgres).expect("split");
        assert_eq!(out, vec!["SELECT * FROM t WHERE id = $1", "SELECT $2"]);
    }

    /// MySQL：`#` 行注释、反引号标识符与反斜杠转义。
    #[test]
    fn handles_mysql_specifics() {
        let sql = "\
# 井号注释; 不是语句边界\n\
INSERT INTO `weird;table` VALUES ('a\\';b');\n\
SELECT 1;";
        let out = split_statements(sql, DialectKind::Mysql).expect("split");
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out[0].contains("`weird;table`"));
        assert!(out[1] == "SELECT 1");
    }

    /// `#` 在 PostgreSQL 中是运算符而非注释，不得据此切分 / 吞掉内容。
    #[test]
    fn treats_hash_as_operator_on_postgres() {
        let sql = "SELECT 1 # 2;\nSELECT 3;";
        let out = split_statements(sql, DialectKind::Postgres).expect("split");
        assert_eq!(out, vec!["SELECT 1 # 2", "SELECT 3"]);
    }

    /// `-- +delimiter` 覆盖默认分隔符（MySQL 存储过程体）。
    #[test]
    fn honors_custom_delimiter_directive() {
        let sql = "\
-- +delimiter //\n\
CREATE PROCEDURE p() BEGIN SELECT 1; SELECT 2; END//\n\
SELECT 3//";
        let out = split_statements(sql, DialectKind::Mysql).expect("split");
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out[0].contains("SELECT 1; SELECT 2; END"));
        assert_eq!(out[1], "SELECT 3");
    }

    /// 指令解析：`+no-transaction` 与 `+delimiter`。
    #[test]
    fn parses_directives() {
        let directives = parse_directives("-- +no-transaction\n-- +delimiter //\nSELECT 1;");
        assert!(directives.is_no_transaction);
        assert_eq!(directives.delimiter.as_deref(), Some("//"));

        let none = parse_directives("-- 普通注释\nSELECT 1;");
        assert!(!none.is_no_transaction);
        assert!(none.delimiter.is_none());
    }

    /// 单文件双段式：按 `-- +migrate Up/Down` 切分。
    #[test]
    fn splits_up_down_sections() {
        let sql = "\
-- +migrate Up\n\
CREATE TABLE t (id INT);\n\
\n\
-- +migrate Down\n\
DROP TABLE t;\n";
        let sections = split_sections(sql);
        assert!(sections.up.contains("CREATE TABLE t"));
        assert!(!sections.up.contains("DROP TABLE t"));
        assert!(sections.down.contains("DROP TABLE t"));
        assert!(!sections.down.contains("CREATE TABLE t"));
    }

    /// 无 `+migrate` 指令时整段即 up。
    #[test]
    fn whole_file_is_up_without_marker() {
        let sections = split_sections("CREATE TABLE t (id INT);\n");
        assert!(sections.up.contains("CREATE TABLE t"));
        assert!(sections.down.is_empty());
    }

    /// 纯注释 / 纯空白片段被丢弃，不产生空语句。
    #[test]
    fn drops_comment_only_fragments() {
        let sql = "-- 只有注释;\n;\n/* 只有块注释 */;\nSELECT 1;";
        let out = split_statements(sql, DialectKind::Sqlite).expect("split");
        assert_eq!(out, vec!["SELECT 1"]);
    }
}

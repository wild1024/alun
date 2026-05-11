/// 数据库方言枚举
///
/// 封装不同数据库的 SQL 语法差异，如参数占位符风格和标识符引号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// PostgreSQL（参数：`$1`，引号：`""`）
    Postgres,
    /// MySQL（参数：`?`，引号：`` ` ` ``）
    Mysql,
    /// SQLite（参数：`?`，引号：`""`）
    Sqlite,
}

impl Dialect {
    /// 参数占位符风格
    pub fn placeholder(&self, index: usize) -> String {
        match self {
            Dialect::Postgres => format!("${}", index),
            Dialect::Mysql => "?".to_string(),
            Dialect::Sqlite => "?".to_string(),
        }
    }

    /// 引用标识符（表名/列名）
    pub fn quote(&self, ident: &str) -> String {
        match self {
            Dialect::Postgres => format!("\"{}\"", ident),
            Dialect::Mysql => format!("`{}`", ident),
            Dialect::Sqlite => format!("\"{}\"", ident),
        }
    }
}

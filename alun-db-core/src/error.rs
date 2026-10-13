//! `DbError` —— 统一错误类型。

use std::error::Error as StdError;

use thiserror::Error;

use crate::traits::dialect::RawDbError;

/// 底层错误对象（保留完整 `source()` 链）。
pub type BoxError = Box<dyn StdError + Send + Sync>;

/// `alun-db` 的统一错误类型。
///
/// ## 错误映射
///
/// 驱动的原始错误由 `Dialect::map_error` 映射为本类型的变体；跨方言的错误码
/// 分支集中在 [`DbError::from_raw_driver_error`]。
///
/// ## 信息保全
///
/// - 约束类变体（[`UniqueViolation`](Self::UniqueViolation) /
///   [`ForeignKeyViolation`](Self::ForeignKeyViolation) /
///   [`CheckViolation`](Self::CheckViolation)）的标识符字段是**解析出的**
///   列名 / 约束名，驱动未提供时为 `None`——绝不把驱动原文塞进这类字段；
///   驱动原文一律经 `source()` 保留。
/// - [`Query`](Self::Query) 的 `Display` 直接带上驱动原因，`source()` 仍可遍历。
#[derive(Debug, Error)]
pub enum DbError {
    // ==================== 连接 ====================
    /// 连接失败。
    #[error("connection failed: {0}")]
    Connection(#[source] BoxError),

    // ==================== 查询 ====================
    /// 查询执行失败。保留 SQL 文本与底层错误。
    ///
    /// `Display` 形如 `query failed: <sql>: <驱动原因>`：直接 `{}` 打印即可看到
    /// 真实原因，无需手工遍历 `source()` 链。
    #[error("query failed: {sql}: {source}")]
    Query {
        /// 失败的 SQL 文本。
        sql: String,
        /// 底层错误。
        #[source]
        source: BoxError,
    },

    // ==================== 记录 ====================
    /// 记录未找到。
    #[error("record not found")]
    NotFound,

    /// 模型为只读（如 `#[view]` 视图模型），禁止写操作。
    #[error("model `{0}` is read-only (view)")]
    Readonly(&'static str),

    // ==================== 约束 ====================
    /// 唯一约束冲突。
    #[error(
        "unique constraint violated{}",
        identifier_detail(.column.as_deref(), .constraint.as_deref(), .source.as_deref())
    )]
    UniqueViolation {
        /// 冲突的列名（驱动提供时，如 SQLite 消息里的 `users.email` → `email`）。
        column: Option<String>,
        /// 冲突的约束 / 索引名（驱动提供时，如 PostgreSQL `uq_users_email`）。
        constraint: Option<String>,
        /// 底层驱动错误。
        #[source]
        source: Option<BoxError>,
    },

    /// 外键约束冲突。
    #[error(
        "foreign key violated{}",
        identifier_detail(None, .constraint.as_deref(), .source.as_deref())
    )]
    ForeignKeyViolation {
        /// 约束名（驱动提供时；SQLite 不提供 → `None`）。
        constraint: Option<String>,
        /// 底层驱动错误。
        #[source]
        source: Option<BoxError>,
    },

    /// CHECK 约束冲突。
    #[error(
        "check constraint violated{}",
        identifier_detail(None, .constraint.as_deref(), .source.as_deref())
    )]
    CheckViolation {
        /// 约束名（驱动提供时；未命名 CHECK 可能退化为表名）。
        constraint: Option<String>,
        /// 底层驱动错误。
        #[source]
        source: Option<BoxError>,
    },

    // ==================== 并发冲突 ====================
    /// 数据库检测到死锁并回滚了当前事务。可安全重试。
    #[error("deadlock detected: {source}")]
    Deadlock {
        /// 底层驱动错误。
        #[source]
        source: BoxError,
    },

    /// 事务因串行化冲突被回滚（PostgreSQL `40001`）。可安全重试。
    #[error("serialization failure: {source}")]
    SerializationFailure {
        /// 底层驱动错误。
        #[source]
        source: BoxError,
    },

    /// 获取锁等待超时 / 锁不可用（MySQL `1205`、PostgreSQL `55P03`、
    /// SQLite `SQLITE_BUSY`）。可安全重试。
    #[error("lock wait timeout: {source}")]
    LockTimeout {
        /// 底层驱动错误。
        #[source]
        source: BoxError,
    },

    // ==================== 类型 ====================
    /// 行中不存在指定列。
    #[error("column `{0}` not found in row")]
    ColumnNotFound(String),

    /// 类型不匹配。
    #[error("type mismatch: expected `{expected}`, got `{actual}`")]
    TypeMismatch {
        /// 期望的类型名。
        expected: &'static str,
        /// 实际类型名。
        actual: &'static str,
    },

    /// 非空列遇到 `Null`。
    #[error("unexpected NULL at column `{0}`")]
    UnexpectedNull(&'static str),

    // ==================== 能力 ====================
    /// 方言不支持请求的功能。
    #[error("unsupported feature `{feature}` for dialect `{dialect}`")]
    UnsupportedFeature {
        /// 功能名。
        feature: &'static str,
        /// 方言名（运行时确定，如来自配置的数据库类型）。
        dialect: String,
    },

    // ==================== 事务 ====================
    /// 事务错误。
    #[error("transaction error: {0}")]
    Transaction(String),

    // ==================== 值 ====================
    /// 无效的值（如越界的日期、格式错误的 JSON）。
    #[error("invalid value: {0}")]
    InvalidValue(String),

    // ==================== 迁移 ====================
    /// 迁移错误。
    #[error("migration error: {0}")]
    Migration(String),

    /// 已应用迁移的内容指纹与当前文件不一致（文件被改写 / 篡改）。
    ///
    /// `applied` 是记录在 `schema_migrations` 中的指纹，`current` 是本次运行时
    /// 重新计算的指纹；两者不等说明**已应用**的迁移脚本被修改过，需要人工介入
    /// （修复文件或清理记录），框架不会静默跳过。
    #[error(
        "migration version {version} (`{name}`) was modified after it was applied: \
         recorded checksum `{applied}` != current `{current}`; \
         the applied migration has been tampered with — restore the file or fix `schema_migrations`"
    )]
    MigrationChecksum {
        /// 迁移版本号（通用版本串，如 `1`、`1.2`、`20250101_120000`）。
        version: String,
        /// 迁移名称。
        name: String,
        /// 库内记录的指纹。
        applied: String,
        /// 本次运行计算的指纹。
        current: String,
    },
}

impl DbError {
    /// 由驱动的原始错误构造统一错误——跨方言错误码映射的**唯一入口**。
    ///
    /// # 参数
    ///
    /// - `raw`：驱动提供的错误码（PostgreSQL SQLSTATE / MySQL errno /
    ///   SQLite 扩展码）、消息与原始错误对象。
    ///
    /// # 映射表
    ///
    /// | 语义 | PostgreSQL | MySQL | SQLite |
    /// |---|---|---|---|
    /// | 唯一约束 | `23505` | `1062` | `2067`、`1555` |
    /// | 外键约束 | `23503` | `1452`、`1451` | `787` |
    /// | CHECK 约束 | `23514` | `3819` | `275` |
    /// | 死锁 | `40P01` | `1213` | — |
    /// | 串行化冲突 | `40001` | — | — |
    /// | 锁等待 | `55P03` | `1205` | `5`、`261`、`517`、`773` |
    ///
    /// 未命中任何分支时落 [`DbError::Query`]（SQL 上下文由驱动调用方补全）。
    pub fn from_raw_driver_error(raw: RawDbError) -> Self {
        let RawDbError {
            code,
            message,
            source,
        } = raw;
        let code = code.as_deref().unwrap_or_default();
        let (column, constraint) = extract_identifiers(code, &message);
        // 原始错误对象优先；驱动未提供时用消息兜底，保证 `source()` 链非空
        let source = || source.unwrap_or_else(|| Box::new(std::io::Error::other(message.clone())));

        match code {
            // ---- 唯一约束 ----
            "23505" | "1062" | "2067" | "1555" => DbError::UniqueViolation {
                column,
                constraint,
                source: Some(source()),
            },
            // ---- 外键约束 ----
            "23503" | "1452" | "1451" | "787" => DbError::ForeignKeyViolation {
                constraint,
                source: Some(source()),
            },
            // ---- CHECK 约束 ----
            "23514" | "3819" | "275" => DbError::CheckViolation {
                constraint,
                source: Some(source()),
            },
            // ---- 并发冲突（均可安全重试）----
            "40P01" | "1213" => DbError::Deadlock { source: source() },
            "40001" => DbError::SerializationFailure { source: source() },
            // SQLite `SQLITE_BUSY` 家族：库被其他连接占用 / 无法取得锁
            "55P03" | "1205" | "5" | "261" | "517" | "773" => {
                DbError::LockTimeout { source: source() }
            }
            // ---- 未命中：保留 SQL 上下文由调用方补全 ----
            _ => DbError::Query {
                sql: String::new(),
                source: source(),
            },
        }
    }
}

/// `alun-db` 的 `Result` 别名。
pub type Result<T, E = DbError> = std::result::Result<T, E>;

/// 约束类错误的展示后缀。
///
/// 优先展示驱动提供的结构化标识符；一个都没有时退回驱动原文（仅用于展示，
/// 不写入标识符字段）。
fn identifier_detail(
    column: Option<&str>,
    constraint: Option<&str>,
    source: Option<&(dyn StdError + Send + Sync)>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(column) = column.filter(|c| !c.is_empty()) {
        parts.push(format!("column `{column}`"));
    }
    if let Some(constraint) = constraint.filter(|c| !c.is_empty()) {
        parts.push(format!("constraint `{constraint}`"));
    }
    if parts.is_empty() {
        return match source {
            Some(source) => format!(" (driver: {source})"),
            None => String::new(),
        };
    }
    format!(" ({})", parts.join(", "))
}

/// 从驱动错误码与消息中提取「列名 / 约束名」。
///
/// 各驱动暴露的结构化程度不同：PostgreSQL / MySQL / SQLite 都只在消息文本里
/// 内嵌标识符（`ALUN` 不依赖任何驱动的私有 API），故按错误码逐类解析。
/// 解析不出的字段保持 `None`，绝不回填驱动原文。
fn extract_identifiers(code: &str, message: &str) -> (Option<String>, Option<String>) {
    match code {
        // SQLite：`UNIQUE constraint failed: users.email[, users.tenant]`
        "2067" | "1555" => (sqlite_qualified_columns(message), None),
        // PostgreSQL：`duplicate key value violates unique constraint "uq_users_email"`
        // MySQL：`Duplicate entry 'x' for key 'users.uq_users_email'`
        "1062" => (None, qualified_key_of(message)),
        "23505" => (None, quoted_after(message, "unique constraint ")),
        // SQLite：`CHECK constraint failed: <table-or-constraint>`；未命名 CHECK 时
        // 该位置是**表达式**（如 `amount >= 0`）→ 只接受标识符形状，否则 None
        "275" => (
            None,
            identifier_like(text_after(message, "check constraint failed:")),
        ),
        // PostgreSQL：`... violates check constraint "ck_x"`
        // MySQL：`Check constraint 'ck_x' is violated.`
        "23514" | "3819" => (None, quoted_after_ci(message, "check constraint ")),
        // PostgreSQL：`... violates foreign key constraint "fk_x"`
        "23503" => (None, quoted_after(message, "foreign key constraint ")),
        // MySQL：`... CONSTRAINT `fk_x` FOREIGN KEY ...`（大写关键字后的反引号标识符）
        "1452" | "1451" => (None, quoted_after(message, "CONSTRAINT ")),
        // SQLite：`FOREIGN KEY constraint failed`（不提供约束名）
        _ => (None, None),
    }
}

/// 仅接受「标识符形状」的文本（`[A-Za-z_][A-Za-z0-9_$.]*`，可带引号/反引号）。
///
/// 用于区分「约束名」（如 `err_probe`、`ck_amount`）与 SQLite 在未命名 CHECK 时
/// 放进消息里的**表达式**（如 `amount >= 0`）——后者不是标识符，应保持 `None`。
fn identifier_like(text: Option<String>) -> Option<String> {
    let raw = text?;
    let trimmed = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '`' || c == '\'')
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut chars = trimmed.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.') {
        return None;
    }
    Some(trimmed.to_string())
}

/// 取 MySQL 1062 消息里 `` for key 'table.index' `` 的索引名（去掉表限定符）。
fn qualified_key_of(message: &str) -> Option<String> {
    quoted_after_ci(message, "for key ").map(|key| {
        // `table.index` → `index`；无 `.` 时原样返回
        key.rsplit('.').next().unwrap_or(&key).to_string()
    })
}

/// 取 SQLite 唯一约束消息 `... failed: a.b, c.d` 中的列名列表。
fn sqlite_qualified_columns(message: &str) -> Option<String> {
    let tail = text_after(message, "unique constraint failed:")?;
    let columns: Vec<&str> = tail
        .split(',')
        // `users.email` → `email`：去掉表限定符
        .map(|item| item.trim().rsplit('.').next().unwrap_or("").trim())
        .filter(|name| !name.is_empty())
        .collect();
    if columns.is_empty() {
        None
    } else {
        Some(columns.join(", "))
    }
}

/// 取关键字之后紧跟的、非空的裸文本（去掉首尾空白）。
fn text_after(message: &str, keyword: &str) -> Option<String> {
    let start = find_ci(message, keyword)? + keyword.len();
    let token = message[start..].trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// 取关键字之后出现的第一个引号包裹的标识符（`"x"` / `'x'` / `` `x` ``）。
///
/// 关键字匹配区分大小写：MySQL 的大写 `CONSTRAINT` 与同句中的小写 `constraint`
/// 需区分（后者出现在 `foreign key constraint fails` 中，其后的引号是库名）。
fn quoted_after(message: &str, keyword: &str) -> Option<String> {
    quoted_after_at(message, message.find(keyword)? + keyword.len())
}

/// [`quoted_after`] 的大小写不敏感版本。
fn quoted_after_ci(message: &str, keyword: &str) -> Option<String> {
    quoted_after_at(message, find_ci(message, keyword)? + keyword.len())
}

/// 从 `rest_start` 起取第一个引号包裹的标识符。
fn quoted_after_at(message: &str, rest_start: usize) -> Option<String> {
    let rest = message.get(rest_start..)?;
    for quote in ['"', '\'', '`'] {
        if let Some(open) = rest.find(quote) {
            let after = &rest[open + quote.len_utf8()..];
            if let Some(close) = after.find(quote) {
                let token = after[..close].trim();
                if !token.is_empty() {
                    return Some(token.to_string());
                }
            }
        }
    }
    None
}

/// 大小写不敏感的子串查找，返回字节下标。
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(code: &str, message: &str) -> RawDbError {
        RawDbError {
            code: Some(code.to_string()),
            message: message.to_string(),
            source: None,
        }
    }

    /// 迁移冲突 / 锁等待类错误码必须映射为可识别变体，而非一律 `Query`。
    #[test]
    fn concurrency_codes_map_to_retryable_variants() {
        let cases: &[(&str, &str)] = &[
            ("40P01", "postgres"),
            ("40001", "postgres"),
            ("1213", "mysql"),
            ("1205", "mysql"),
            ("5", "sqlite"),
        ];
        for (code, driver) in cases {
            let err = DbError::from_raw_driver_error(raw(code, "conflict"));
            let kind = match &err {
                DbError::Deadlock { .. } => "deadlock",
                DbError::SerializationFailure { .. } => "serialization",
                DbError::LockTimeout { .. } => "lock timeout",
                other => panic!("{driver} 错误码 {code} 未映射：{other:?}"),
            };
            assert!(!kind.is_empty());
            // Display 与 source() 都必须保留驱动原文
            assert!(err.to_string().contains("conflict"), "{code}: {err}");
            assert!(err.source().is_some(), "{code} 丢了 source");
        }
    }

    /// 约束类错误的标识符字段取解析出的名字；SQLite 无约束名时为 `None`。
    #[test]
    fn constraint_identifiers_are_parsed_not_raw_messages() {
        match DbError::from_raw_driver_error(raw("2067", "UNIQUE constraint failed: users.email")) {
            DbError::UniqueViolation {
                column, constraint, ..
            } => {
                assert_eq!(column.as_deref(), Some("email"));
                assert_eq!(constraint, None);
            }
            other => panic!("{other:?}"),
        }

        match DbError::from_raw_driver_error(raw(
            "23505",
            "duplicate key value violates unique constraint \"uq_users_email\"",
        )) {
            DbError::UniqueViolation {
                column, constraint, ..
            } => {
                assert_eq!(column, None);
                assert_eq!(constraint.as_deref(), Some("uq_users_email"));
            }
            other => panic!("{other:?}"),
        }

        match DbError::from_raw_driver_error(raw(
            "1062",
            "Duplicate entry 'a@b.c' for key 'users.uq_users_email'",
        )) {
            DbError::UniqueViolation { constraint, .. } => {
                assert_eq!(constraint.as_deref(), Some("uq_users_email"));
            }
            other => panic!("{other:?}"),
        }

        // SQLite 外键不提供约束名 → `None`（不得回填原始消息）
        match DbError::from_raw_driver_error(raw("787", "FOREIGN KEY constraint failed")) {
            DbError::ForeignKeyViolation { constraint, .. } => assert_eq!(constraint, None),
            other => panic!("{other:?}"),
        }

        // MySQL 外键：`foreign key constraint fails` 后的库名不是约束名
        match DbError::from_raw_driver_error(raw(
            "1452",
            "Cannot add or update a child row: a foreign key constraint fails \
             (`alun`.`items` CONSTRAINT `fk_items_invoice` FOREIGN KEY (`invoice_id`) \
             REFERENCES `invoices` (`id`))",
        )) {
            DbError::ForeignKeyViolation { constraint, .. } => {
                assert_eq!(constraint.as_deref(), Some("fk_items_invoice"));
            }
            other => panic!("{other:?}"),
        }

        match DbError::from_raw_driver_error(raw(
            "3819",
            "Check constraint 'ck_err_probe_amount' is violated.",
        )) {
            DbError::CheckViolation { constraint, .. } => {
                assert_eq!(constraint.as_deref(), Some("ck_err_probe_amount"));
            }
            other => panic!("{other:?}"),
        }

        match DbError::from_raw_driver_error(raw("275", "CHECK constraint failed: err_probe")) {
            DbError::CheckViolation { constraint, .. } => {
                assert_eq!(constraint.as_deref(), Some("err_probe"));
            }
            other => panic!("{other:?}"),
        }
    }

    /// `Query` 的 Display 必须带上驱动原因（D019）。
    #[test]
    fn query_display_includes_driver_reason() {
        let err = DbError::from_raw_driver_error(raw("42P01", "relation \"t\" does not exist"));
        match &err {
            DbError::Query { sql, .. } => assert!(sql.is_empty(), "SQL 由调用方补全"),
            other => panic!("{other:?}"),
        }
        let text = err.to_string();
        assert!(text.contains("query failed"), "{text}");
        assert!(
            text.contains("does not exist"),
            "Display 丢了驱动原因：{text}"
        );
    }
}

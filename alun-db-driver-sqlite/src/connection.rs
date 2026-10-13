//! SQLite 连接。

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use alun_db_core::error::{DbError, Result};
use alun_db_core::row::Row;
use alun_db_core::statement::Statement;
use alun_db_core::traits::dialect::{Dialect, RawDbError};
use alun_db_core::traits::executor::{ExecuteResult, Rows};
use alun_db_sql::dialects::sqlite::SqliteDialect;

use crate::convert;

/// 列元数据缓存的容量上限（超过则整体清空）。
///
/// 动态拼接的 SQL 会让缓存键无限增长，故设上限；上限内是「热点 SQL 常驻」，
/// 命中率足够高，清空策略的抖动可接受。
const COLUMN_META_CACHE_CAPACITY: usize = 64;

/// 一条 SQL 的列元数据（列名 + 声明类型）。
///
/// 同一 SQL 文本的列名与声明类型恒定，因此可按 SQL 复用：点查这类高频短查询
/// 若每次重建这两组字符串，元数据分配会成为该路径固定开销的主体。
///
/// 列名以 `Arc<str>` 持有，逐行结果直接 `Arc::clone` 共享同一份名字——
/// 分页 / 全表扫描时行数远大于列数，这份共享把「每行 N 次列名字符串分配」
/// 摊成「每 SQL 一次」。
#[derive(Debug)]
struct ColumnMeta {
    /// 列名，按 SELECT 顺序。
    names: Vec<Arc<str>>,
    /// 列声明类型（`sqlite3_column_decltype`）；表达式列为 `None`。
    decltypes: Vec<Option<String>>,
}

/// SQLite 连接。内部持有 rusqlite 连接。
///
/// ## 线程安全
///
/// `rusqlite::Connection` 不是 `Sync` 的。`SqliteConnection` 通过
/// 池的 `Mutex` 串行化访问以保证 `Send + Sync` 约束。对于 SQLite
/// 的典型使用场景（单写多读），这是可接受的。
pub struct SqliteConnection {
    /// rusqlite 底层连接。
    inner: rusqlite::Connection,
    /// SQLite 方言。用于语句渲染与错误映射。
    dialect: SqliteDialect,
    /// SQL → 列元数据缓存。
    ///
    /// `RefCell` 而非 `Mutex`：`SqliteConnection` 只要求 `Send`（池以 `Mutex`
    /// 串行化借出），单连接不会被跨线程并发访问，无锁访问更贴合实际使用。
    column_meta: RefCell<HashMap<String, Arc<ColumnMeta>>>,
}

impl SqliteConnection {
    /// 创建连接。
    ///
    /// # 参数
    ///
    /// - `url`：`sqlite://path`、`sqlite::memory:` 或文件路径
    ///
    /// # 错误
    ///
    /// 打开数据库或启用外键约束失败时返回 [`DbError::Connection`]。
    pub fn open(url: &str) -> Result<Self> {
        let path = url.strip_prefix("sqlite://").unwrap_or(url);
        let inner = if path == ":memory:" || url == "sqlite::memory:" {
            rusqlite::Connection::open_in_memory()
        } else {
            rusqlite::Connection::open(path)
        }
        .map_err(|e| DbError::Connection(Box::new(e)))?;

        // 启用外键约束
        inner
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| DbError::Connection(Box::new(e)))?;

        Ok(SqliteConnection {
            inner,
            dialect: SqliteDialect,
            column_meta: RefCell::new(HashMap::new()),
        })
    }

    /// 返回方言引用。
    pub fn dialect(&self) -> &SqliteDialect {
        &self.dialect
    }

    /// 执行 INSERT / UPDATE / DELETE。
    ///
    /// # 错误
    ///
    /// 语句准备或执行失败时返回 [`DbError`]，底层错误码经方言
    /// `map_error` 映射（如唯一约束冲突映射为 `UniqueViolation`）。
    pub fn execute_sync(&self, stmt: &Statement) -> Result<ExecuteResult> {
        // Raw 语句直接使用传入的 SQL 和参数
        if let Statement::Raw(raw) = stmt {
            let mut prepared = self
                .inner
                .prepare_cached(&raw.sql)
                .map_err(|e| self.map_error(e, &raw.sql))?;
            bind_params(&mut prepared, &raw.params)?;
            let affected = prepared
                .raw_execute()
                .map_err(|e| self.map_error(e, &raw.sql))?;
            return Ok(ExecuteResult {
                rows_affected: affected as u64,
                last_insert_id: Some(self.inner.last_insert_rowid()),
            });
        }

        let rendered = self.dialect.render_statement(stmt);
        let mut prepared = self
            .inner
            .prepare_cached(&rendered.sql)
            .map_err(|e| self.map_error(e, &rendered.sql))?;
        bind_params(&mut prepared, &rendered.params)?;
        let rows_affected = prepared
            .raw_execute()
            .map_err(|e| self.map_error(e, &rendered.sql))?;

        // INSERT 语句附带自增主键
        let last_insert_id = match stmt {
            Statement::Insert(_) => Some(self.inner.last_insert_rowid()),
            _ => None,
        };
        Ok(ExecuteResult {
            rows_affected: rows_affected as u64,
            last_insert_id,
        })
    }

    /// 执行 SELECT。
    ///
    /// # 错误
    ///
    /// 语句准备、绑定或行读取失败时返回 [`DbError`]。
    pub fn query_sync(&self, stmt: &Statement) -> Result<Rows> {
        match stmt {
            // Raw 分支直接借用传入的 SQL 与参数，不做任何克隆（点查等热路径）
            Statement::Raw(raw) => self.query_borrowed(&raw.sql, &raw.params),
            _ => {
                let rendered = self.dialect.render_statement(stmt);
                self.query_borrowed(&rendered.sql, &rendered.params)
            }
        }
    }

    /// 按 SQL 文本与参数执行查询（语句与列元数据均按 SQL 复用）。
    ///
    /// # 错误
    ///
    /// 语句准备、绑定或行读取失败时返回 [`DbError`]。
    fn query_borrowed(&self, sql: &str, params: &[alun_db_core::stmt::Value]) -> Result<Rows> {
        let mut prepared = self
            .inner
            .prepare_cached(sql)
            .map_err(|e| self.map_error(e, sql))?;
        bind_params(&mut prepared, params)?;

        // 列名与声明类型按 SQL 缓存：重复查询不再重建这两组字符串
        let meta = self.column_meta_for(sql, &prepared);

        let mut rows = prepared.raw_query();
        // 行容量按列数一次到位；列名逐列 `Arc::clone` 共享缓存里的那份，
        // 追加走 `push_shared`（列序即结果集列序，无需重名检查）
        let mut result = Vec::new();
        while let Some(row) = rows.next().map_err(|e| self.map_error(e, sql))? {
            let mut r = Row::with_capacity(meta.names.len());
            for (i, name) in meta.names.iter().enumerate() {
                let value_ref = row.get_ref(i).map_err(|e| self.map_error(e, sql))?;
                let value = convert::from_rusqlite(value_ref, meta.decltypes[i].as_deref())?;
                r.push_shared(Arc::clone(name), value);
            }
            result.push(r);
        }

        Ok(Rows(result))
    }

    /// 取（或写入）该 SQL 的列元数据。
    ///
    /// 容量超过 [`COLUMN_META_CACHE_CAPACITY`] 时整体清空，避免动态 SQL 让缓存无限增长。
    fn column_meta_for(&self, sql: &str, prepared: &rusqlite::Statement<'_>) -> Arc<ColumnMeta> {
        if let Some(meta) = self.column_meta.borrow().get(sql) {
            return Arc::clone(meta);
        }

        let meta = Arc::new(ColumnMeta {
            names: prepared
                .column_names()
                .iter()
                .map(|s| Arc::from(*s))
                .collect(),
            decltypes: prepared
                .columns()
                .iter()
                .map(|c| c.decl_type().map(|s| s.to_string()))
                .collect(),
        });

        let mut cache = self.column_meta.borrow_mut();
        if cache.len() >= COLUMN_META_CACHE_CAPACITY {
            cache.clear();
        }
        cache.insert(sql.to_string(), Arc::clone(&meta));
        meta
    }

    /// 执行多语句 SQL 脚本（`;` 分隔的完整脚本，不支持参数绑定）。
    ///
    /// 底层使用 rusqlite 的 `execute_batch`。任一语句失败时整体报错，
    /// 已执行语句不会自动回滚（SQLite 无嵌套事务语义，调用方自行取舍）。
    ///
    /// # 错误
    ///
    /// 任一语句执行失败时返回 [`DbError`]。
    pub fn execute_script_sync(&self, sql: &str) -> Result<()> {
        self.inner
            .execute_batch(sql)
            .map_err(|e| self.map_error(e, sql))?;
        Ok(())
    }

    /// 开始事务。
    ///
    /// # 错误
    ///
    /// `BEGIN` 执行失败时返回 [`DbError::Transaction`]。
    pub fn begin_transaction(&self) -> Result<()> {
        self.inner
            .execute_batch("BEGIN")
            .map_err(|e| DbError::Transaction(e.to_string()))
    }

    /// 提交事务。
    ///
    /// # 错误
    ///
    /// `COMMIT` 执行失败时返回 [`DbError::Transaction`]。
    pub fn commit(&self) -> Result<()> {
        self.inner
            .execute_batch("COMMIT")
            .map_err(|e| DbError::Transaction(e.to_string()))
    }

    /// 回滚事务。
    ///
    /// # 错误
    ///
    /// `ROLLBACK` 执行失败时返回 [`DbError::Transaction`]。
    pub fn rollback(&self) -> Result<()> {
        self.inner
            .execute_batch("ROLLBACK")
            .map_err(|e| DbError::Transaction(e.to_string()))
    }

    /// 将 rusqlite 错误映射为统一的 `DbError`。
    ///
    /// SQLite 原生错误码（如 `2067` 唯一约束、`787` 外键约束）经方言
    /// `map_error` 转换；映射产生的 `Query` 变体补全 SQL 上下文。
    fn map_error(&self, e: rusqlite::Error, sql: &str) -> DbError {
        // 先借用提取错误码与消息，再消耗 e 作为底层错误源
        let sqlite_failure = match &e {
            rusqlite::Error::SqliteFailure(ffi_err, msg) => Some((
                ffi_err.extended_code.to_string(),
                msg.clone().unwrap_or_else(|| format!("{:?}", ffi_err.code)),
            )),
            _ => None,
        };
        let mapped = match sqlite_failure {
            Some((code, message)) => self.dialect.map_error(RawDbError {
                code: Some(code),
                message,
                source: Some(Box::new(e)),
            }),
            None => DbError::Query {
                sql: sql.to_string(),
                source: Box::new(e),
            },
        };
        // 方言 map_error 生成的 Query 变体不携带 SQL 上下文，补全
        match mapped {
            DbError::Query { sql: empty, source } if empty.is_empty() => DbError::Query {
                sql: sql.to_string(),
                source,
            },
            other => other,
        }
    }
}

/// 将 `stmt::Value` 列表绑定到 prepared statement。
///
/// # 错误
///
/// 值转换或绑定失败时返回 [`DbError`]。
fn bind_params(
    stmt: &mut rusqlite::Statement<'_>,
    params: &[alun_db_core::stmt::Value],
) -> Result<()> {
    for (i, value) in params.iter().enumerate() {
        let rvalue = convert::to_rusqlite(value)?;
        stmt.raw_bind_parameter(i + 1, rvalue)
            .map_err(|e| DbError::Query {
                sql: String::new(),
                source: Box::new(e),
            })?;
    }
    Ok(())
}

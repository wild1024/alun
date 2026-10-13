//! `SqlQuery` 的执行层：方言占位符改写 + 链式执行 API。
//!
//! [`Db`] 与 [`Transaction`] 都实现 [`SqlExecutor`]，`sql!` 宏产物
//! 可直接链式执行：
//!
//! ```ignore
//! sql!("SELECT * FROM users WHERE email = ", { email })
//!     .find(db).await?;
//!
//! db.transaction(|tx| async move {
//!     sql!("UPDATE users SET status = ", { 9i32 }, " WHERE id = ", { id })
//!         .execute(&tx).await?;
//!     Ok(())
//! }).await?;
//! ```

use std::future::Future;
use std::pin::Pin;

use alun_db_core::error::{DbError, Result};
use alun_db_core::query::SqlQuery;
use alun_db_core::row::Row;
use alun_db_core::stmt::Value;
use alun_db_core::traits::TypeMapper;

use alun_db_core::page::Page;
use alun_db_core::traits::dialect::Dialect;

use crate::db::Db;
use crate::tx::Transaction;

/// SQL 执行器抽象：[`Db`] 与 [`Transaction`] 都能执行 [`SqlQuery`]。
///
/// `Send + Sync` 约束保证 `sql!` 产物的执行 future 在多线程运行时可 spawn。
pub trait SqlExecutor: Send + Sync {
    /// 执行器所用方言，用于占位符改写。
    fn sql_dialect(&self) -> &'static dyn Dialect;

    /// 「提交后任务队列」—— 供异步 Hook 绑定事务边界。
    ///
    /// - `None`（默认，[`Db`] 自动提交）：语句执行即已提交，副作用应立即执行；
    /// - `Some(queue)`（[`Transaction`]）：副作用应排队，`commit()` 成功后统一执行，
    ///   `rollback()` 时丢弃。
    ///
    /// 见 [`AsyncHook`](alun_db_core::traits::AsyncHook) 与
    /// [`hook_async`](crate::hook_async)。
    fn post_commit_queue(&self) -> Option<&crate::hook_async::PostCommitQueue> {
        None
    }

    /// 执行写语句（INSERT / UPDATE / DELETE），返回受影响行数。
    fn exec_sql(&self, sql: String, params: Vec<Value>)
    -> impl Future<Output = Result<u64>> + Send;

    /// 执行查询，返回所有行。
    fn query_sql(
        &self,
        sql: String,
        params: Vec<Value>,
    ) -> impl Future<Output = Result<Vec<Row>>> + Send;

    /// 查询首行；无行返回 `Ok(None)`。
    fn query_first_sql(
        &self,
        sql: String,
        params: Vec<Value>,
    ) -> impl Future<Output = Result<Option<Row>>> + Send {
        async move { Ok(self.query_sql(sql, params).await?.into_iter().next()) }
    }

    /// 取单值（首行首列）。
    fn scalar_sql<T: TypeMapper>(
        &self,
        sql: String,
        params: Vec<Value>,
    ) -> impl Future<Output = Result<T>> + Send {
        async move {
            let row = self
                .query_first_sql(sql, params)
                .await?
                .ok_or(DbError::NotFound)?;
            let value = row
                .values()
                .next()
                .ok_or_else(|| DbError::ColumnNotFound("scalar".into()))?;
            T::from_value(value)
        }
    }

    /// 分页查询（`sql` 不含 LIMIT / OFFSET）。
    fn paginate_sql(
        &self,
        sql: String,
        params: Vec<Value>,
        page: u64,
        size: u64,
    ) -> impl Future<Output = Result<Page<Row>>> + Send {
        async move {
            let page = page.max(1);
            let size = size.max(1);
            let offset = (page - 1) * size;

            let count_sql = format!("SELECT COUNT(*) FROM ({sql}) AS _count");
            let total: i64 = self.scalar_sql(count_sql, params.clone()).await?;

            // LIMIT / OFFSET 内联为字面量（见 `Dialect::push_limit_offset`：
            // 参数化 LIMIT 会让 SQLite 每次查询多花约 2.7 µs）
            let mut data_sql = String::with_capacity(sql.len() + 24);
            data_sql.push_str(&sql);
            self.sql_dialect()
                .push_limit_offset(Some(size), Some(offset), &mut data_sql);
            let list = self.query_sql(data_sql, params).await?;

            Ok(Page {
                list,
                total: total as u64,
                page,
                size,
                pages: Page::<Row>::calc_pages(total as u64, size),
            })
        }
    }

    /// INSERT 并取回自增主键。
    ///
    /// 支持 `RETURNING` 的方言走查询路径；其余方言执行后读连接级
    /// `LAST_INSERT_ID()`（事务内同样有效，因为它绑定连接）。
    fn insert_returning_id_sql(
        &self,
        sql: String,
        params: Vec<Value>,
    ) -> impl Future<Output = Result<i64>> + Send {
        async move {
            if self.sql_dialect().capability().returning_clause {
                let row =
                    self.query_first_sql(sql, params)
                        .await?
                        .ok_or_else(|| DbError::Query {
                            sql: "INSERT ... RETURNING".into(),
                            source: "INSERT ... RETURNING returned no row".into(),
                        })?;
                let value = row
                    .values()
                    .next()
                    .ok_or_else(|| DbError::ColumnNotFound("returning id".into()))?;
                i64::from_value(value)
            } else {
                self.exec_sql(sql, params).await?;
                self.scalar_sql("SELECT LAST_INSERT_ID()".to_string(), vec![])
                    .await
            }
        }
    }

    /// 在**事务作用域**内执行闭包。
    ///
    /// `Db` 会开启新事务（`Ok` 提交 / `Err` 回滚）；`Transaction` 直接内联执行，
    /// 因为框架不支持嵌套事务（缺口 G4）——在事务内调用本方法不会另开事务，
    /// 而是复用外层事务的原子性。
    /// 闭包返回**装箱 future**，使其可以借用事务句柄跨 `await`
    /// （`for<'x>` 高阶约束：借用生命周期由事务句柄决定，而非 `&self`）。
    fn in_tx<'a, T, F>(&'a self, f: F) -> impl Future<Output = Result<T>> + Send + 'a
    where
        Self: Sized,
        F: for<'x> FnOnce(&'x Transaction) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'x>>
            + Send
            + 'a,
        T: Send + 'a;
}

impl SqlExecutor for Db {
    fn sql_dialect(&self) -> &'static dyn Dialect {
        self.dialect()
    }

    /// `Db`：优先用驱动回传的 `last_insert_id`（比再查一次 `LAST_INSERT_ID()` 更稳）。
    async fn insert_returning_id_sql(&self, sql: String, params: Vec<Value>) -> Result<i64> {
        self.insert_returning_id(&sql, params).await
    }

    /// `Db`：开启真正的事务；闭包收到事务句柄（`E = Transaction`），按结果提交 / 回滚。
    async fn in_tx<'a, T, F>(&'a self, f: F) -> Result<T>
    where
        F: for<'x> FnOnce(&'x Transaction) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'x>>
            + Send
            + 'a,
        T: Send + 'a,
    {
        // 直接驱动事务（不经 `Db::transaction`）：闭包返回的 future 借用事务句柄，
        // 句柄必须活在本函数栈上才能跨 await。语义与 `transaction()` 一致：
        // Ok 提交 / Err 回滚（回滚失败不覆盖业务错误，RAII 兜底）。
        let tx = self.begin().await?;
        match f(&tx).await {
            Ok(value) => {
                tx.commit().await?;
                Ok(value)
            }
            Err(e) => {
                let _ = tx.rollback().await;
                Err(e)
            }
        }
    }

    async fn exec_sql(&self, sql: String, params: Vec<Value>) -> Result<u64> {
        self.execute_owned(sql, params).await
    }

    async fn query_sql(&self, sql: String, params: Vec<Value>) -> Result<Vec<Row>> {
        self.find_owned(sql, params).await
    }

    /// `Db`：SQL 已拥有所有权，直接取首行，不再构造整段结果再丢弃。
    async fn query_first_sql(&self, sql: String, params: Vec<Value>) -> Result<Option<Row>> {
        self.find_first_owned(sql, params).await
    }
}

impl Db {}

impl SqlExecutor for Transaction {
    fn sql_dialect(&self) -> &'static dyn Dialect {
        self.dialect()
    }

    /// `Transaction`：异步 Hook 排队至本事务提交成功后执行（回滚则丢弃）。
    fn post_commit_queue(&self) -> Option<&crate::hook_async::PostCommitQueue> {
        Some(self.post_commit_queue_ref())
    }

    /// `Transaction`：已在事务中 → 内联执行（框架不支持嵌套事务，见缺口 G4）。
    async fn in_tx<'a, T, F>(&'a self, f: F) -> Result<T>
    where
        F: for<'x> FnOnce(&'x Transaction) -> Pin<Box<dyn Future<Output = Result<T>> + Send + 'x>>
            + Send
            + 'a,
        T: Send + 'a,
    {
        f(self).await
    }

    async fn exec_sql(&self, sql: String, params: Vec<Value>) -> Result<u64> {
        self.execute_owned(sql, params).await
    }

    async fn query_sql(&self, sql: String, params: Vec<Value>) -> Result<Vec<Row>> {
        self.find_owned(sql, params).await
    }
}

// ==================== 查询终态 ====================

/// 判断是否可安全追加 `LIMIT` / `OFFSET` 子句。
///
/// 仅当语句是 SELECT 类（以 `SELECT` / `WITH` / `(` 开头）且不含已有的
/// `LIMIT` / `OFFSET` 时才追加；INSERT ... RETURNING 等非查询语句或已分页的
/// 语句不追加，避免语法破坏。
pub(crate) fn can_append_limit(sql: &str) -> bool {
    let upper = sql.trim_start().to_ascii_uppercase();
    (upper.starts_with("SELECT") || upper.starts_with("WITH") || upper.starts_with('('))
        && !upper.contains(" LIMIT")
}

/// 同上，且不得已有 `OFFSET`（分页遍历追加的是 `LIMIT ? OFFSET ?`）。
fn can_append_limit_offset(sql: &str) -> bool {
    can_append_limit(sql) && !sql.to_ascii_uppercase().contains(" OFFSET")
}

/// 渲染 `SqlQuery` 为方言 SQL（占位符改写），返回可直接交给执行器的二元组。
pub(crate) fn prepare(q: SqlQuery, dialect: &'static dyn Dialect) -> (String, Vec<Value>) {
    let (sql, params) = q.into_parts();
    (
        alun_db_sql::rewrite_placeholders_owned(sql, dialect),
        params,
    )
}

/// 查询全部行。
pub(crate) async fn run_find<E: SqlExecutor>(q: SqlQuery, ex: &E) -> Result<Vec<Row>> {
    let (sql, params) = prepare(q, ex.sql_dialect());
    ex.query_sql(sql, params).await
}

/// 查询首行；追加 `LIMIT 1` 避免取全量结果。
pub(crate) async fn run_find_first<E: SqlExecutor>(q: SqlQuery, ex: &E) -> Result<Option<Row>> {
    let (mut sql, params) = q.into_parts();
    // 就地追加而非 `format!` 重建：SQL 已是拥有所有权的字符串，拼接无需再分配整串
    if can_append_limit(&sql) {
        sql.push_str(" LIMIT 1");
    }
    let sql = alun_db_sql::rewrite_placeholders_owned(sql, ex.sql_dialect());
    ex.query_first_sql(sql, params).await
}

/// 查询恰好一行（`LIMIT 2` 探测多行）。
///
/// `allow_none` 为 `true` 时 0 行返回 `None`，否则返回 [`DbError::NotFound`]；
/// 多于一行一律返回 [`DbError::InvalidValue`]。
pub(crate) async fn run_find_one<E: SqlExecutor>(
    q: SqlQuery,
    ex: &E,
    allow_none: bool,
) -> Result<Option<Row>> {
    let (mut sql, params) = q.into_parts();
    // LIMIT 2：多取一行即可判定「结果不唯一」，无需全量取回
    if can_append_limit(&sql) {
        sql.push_str(" LIMIT 2");
    }
    let sql = alun_db_sql::rewrite_placeholders_owned(sql, ex.sql_dialect());
    let mut rows = ex.query_sql(sql, params).await?;
    match rows.len() {
        0 if allow_none => Ok(None),
        0 => Err(DbError::NotFound),
        1 => Ok(Some(rows.remove(0))),
        _ => Err(DbError::InvalidValue(
            "find_one: query matched more than one row".into(),
        )),
    }
}

/// 是否存在匹配行（`LIMIT 1` 探测）。
pub(crate) async fn run_exists<E: SqlExecutor>(q: SqlQuery, ex: &E) -> Result<bool> {
    Ok(run_find_first(q, ex).await?.is_some())
}

/// 分页查询（条数与页码由调用方给出，`OFFSET` 分页）。
pub(crate) async fn run_paginate<E: SqlExecutor>(
    q: SqlQuery,
    ex: &E,
    page: u64,
    size: u64,
) -> Result<Page<Row>> {
    let (sql, params) = prepare(q, ex.sql_dialect());
    ex.paginate_sql(sql, params, page, size).await
}

/// 逐行遍历查询结果。
///
/// 回调返回 `Ok(false)` 提前结束。结果集**一次性取回**（协议无游标支持），
/// 超大结果集请改用 [`run_for_each_page`] 的分页遍历。
pub(crate) async fn run_for_each<E, F>(q: SqlQuery, ex: &E, mut f: F) -> Result<()>
where
    E: SqlExecutor,
    F: FnMut(Row) -> Result<bool> + Send,
{
    for row in run_find(q, ex).await? {
        if !f(row)? {
            break;
        }
    }
    Ok(())
}

/// 按页遍历查询结果，内存占用受 `size` 约束。
///
/// 回调返回 `Ok(false)` 提前结束。`sql` 不得自带 `LIMIT` / `OFFSET`
/// （分页子句由本方法追加），否则返回 [`DbError::InvalidValue`]。
pub(crate) async fn run_for_each_page<E, F>(q: SqlQuery, ex: &E, size: u64, mut f: F) -> Result<()>
where
    E: SqlExecutor,
    F: FnMut(Page<Row>) -> Result<bool> + Send,
{
    let (sql, params) = q.into_parts();
    if !can_append_limit_offset(&sql) {
        return Err(DbError::InvalidValue(
            "for_each_page: query must be a SELECT without LIMIT / OFFSET".into(),
        ));
    }
    let sql = alun_db_sql::rewrite_placeholders_owned(sql, ex.sql_dialect());
    let size = size.max(1);
    let mut page_num = 1_u64;
    loop {
        // LIMIT / OFFSET 内联为字面量（见 `Dialect::push_limit_offset`）
        let offset = (page_num - 1) * size;
        let mut paged_sql = String::with_capacity(sql.len() + 24);
        paged_sql.push_str(&sql);
        ex.sql_dialect()
            .push_limit_offset(Some(size), Some(offset), &mut paged_sql);
        let list = ex.query_sql(paged_sql, params.clone()).await?;
        let fetched = list.len() as u64;
        if fetched == 0 {
            return Ok(());
        }
        // keyset / 遍历分页不预读总数：total / pages 保持在 0，page 为当前页号
        let more = f(Page {
            list,
            total: 0,
            page: page_num,
            size,
            pages: 0,
        })?;
        if !more || fetched < size {
            return Ok(());
        }
        page_num += 1;
    }
}

/// 取单值（首行首列）。
pub(crate) async fn run_scalar<T: TypeMapper, E: SqlExecutor>(q: SqlQuery, ex: &E) -> Result<T> {
    let (sql, params) = prepare(q, ex.sql_dialect());
    ex.scalar_sql(sql, params).await
}

/// 执行写语句，返回受影响行数。
pub(crate) async fn run_execute<E: SqlExecutor>(q: SqlQuery, ex: &E) -> Result<u64> {
    let (sql, params) = prepare(q, ex.sql_dialect());
    ex.exec_sql(sql, params).await
}

/// [`SqlQuery`] 的链式执行扩展（`sql!` 的终态动词）。
///
/// 导入 [`crate::prelude`] 即可用：
///
/// ```ignore
/// sql!("SELECT * FROM users WHERE age > ", { 18 }).find(&db).await?;
/// sql!("SELECT COUNT(*) FROM users").scalar::<i64, _>(&db).await?;
/// ```
///
/// 终态动词与 aifei 的 `Dao` 保持一致：
/// `find` / `find_first` / `find_one` / `find_one_or_null` / `exists` /
/// `paginate` / `for_each` / `for_each_page` / `execute`。
pub trait SqlQueryExt: Sized {
    /// 查询全部行。
    fn find<E: SqlExecutor>(self, ex: &E) -> impl Future<Output = Result<Vec<Row>>> + Send;

    /// 查询首行；无行返回 `Ok(None)`（追加 `LIMIT 1`）。
    fn find_first<E: SqlExecutor>(self, ex: &E)
    -> impl Future<Output = Result<Option<Row>>> + Send;

    /// 查询恰好一行。无行返回 [`DbError::NotFound`]，多于一行返回
    /// [`DbError::InvalidValue`]。
    fn find_one<E: SqlExecutor>(self, ex: &E) -> impl Future<Output = Result<Row>> + Send;

    /// 查询 0 或 1 行。多于一行返回 [`DbError::InvalidValue`]。
    fn find_one_or_null<E: SqlExecutor>(
        self,
        ex: &E,
    ) -> impl Future<Output = Result<Option<Row>>> + Send;

    /// 是否存在匹配行。
    fn exists<E: SqlExecutor>(self, ex: &E) -> impl Future<Output = Result<bool>> + Send;

    /// 分页查询（页码从 1 开始）。
    fn paginate<E: SqlExecutor>(
        self,
        ex: &E,
        page: u64,
        size: u64,
    ) -> impl Future<Output = Result<Page<Row>>> + Send;

    /// 逐行遍历（结果一次性取回）。回调返回 `Ok(false)` 提前结束。
    fn for_each<E, F>(self, ex: &E, f: F) -> impl Future<Output = Result<()>> + Send
    where
        E: SqlExecutor,
        F: FnMut(Row) -> Result<bool> + Send;

    /// 按页遍历（内存占用受 `size` 约束）。回调返回 `Ok(false)` 提前结束。
    fn for_each_page<E, F>(
        self,
        ex: &E,
        size: u64,
        f: F,
    ) -> impl Future<Output = Result<()>> + Send
    where
        E: SqlExecutor,
        F: FnMut(Page<Row>) -> Result<bool> + Send;

    /// 取单值（首行首列）。
    fn scalar<T: TypeMapper, E: SqlExecutor>(
        self,
        ex: &E,
    ) -> impl Future<Output = Result<T>> + Send;

    /// 执行写语句，返回受影响行数。
    fn execute<E: SqlExecutor>(self, ex: &E) -> impl Future<Output = Result<u64>> + Send;
}

impl SqlQueryExt for SqlQuery {
    async fn find<E: SqlExecutor>(self, ex: &E) -> Result<Vec<Row>> {
        run_find(self, ex).await
    }

    async fn find_first<E: SqlExecutor>(self, ex: &E) -> Result<Option<Row>> {
        run_find_first(self, ex).await
    }

    async fn find_one<E: SqlExecutor>(self, ex: &E) -> Result<Row> {
        run_find_one(self, ex, false)
            .await?
            .ok_or(DbError::NotFound)
    }

    async fn find_one_or_null<E: SqlExecutor>(self, ex: &E) -> Result<Option<Row>> {
        run_find_one(self, ex, true).await
    }

    async fn exists<E: SqlExecutor>(self, ex: &E) -> Result<bool> {
        run_exists(self, ex).await
    }

    async fn paginate<E: SqlExecutor>(self, ex: &E, page: u64, size: u64) -> Result<Page<Row>> {
        run_paginate(self, ex, page, size).await
    }

    async fn for_each<E, F>(self, ex: &E, f: F) -> Result<()>
    where
        E: SqlExecutor,
        F: FnMut(Row) -> Result<bool> + Send,
    {
        run_for_each(self, ex, f).await
    }

    async fn for_each_page<E, F>(self, ex: &E, size: u64, f: F) -> Result<()>
    where
        E: SqlExecutor,
        F: FnMut(Page<Row>) -> Result<bool> + Send,
    {
        run_for_each_page(self, ex, size, f).await
    }

    async fn scalar<T: TypeMapper, E: SqlExecutor>(self, ex: &E) -> Result<T> {
        run_scalar(self, ex).await
    }

    async fn execute<E: SqlExecutor>(self, ex: &E) -> Result<u64> {
        run_execute(self, ex).await
    }
}

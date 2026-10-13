//! `CrudExt` —— 为所有 `Model` 提供的关联查询 / 删除函数。
//!
//! 命名与语义对齐 aifei 的 `Model` 静态糖：每个方法都是
//! [`ModelQuery`] 或 [`CountBuilder`] 的一行转发，唯一实现永远只有一份。
//!
//! ## `where_or_field` 双态签名
//!
//! `find_by` / `find_first_by` / `count_by` / `delete_by` 的第一个参数既是
//! **字段名**也是 **WHERE 片段**：
//!
//! ```ignore
//! User::find_by(&db, "age", (18,)).await?;                       // 字段名 + 值
//! User::find_by(&db, "age = ? ORDER BY id DESC", (18,)).await?;  // WHERE 片段
//! ```
//!
//! 只有一个参数且恰好是合法标识符时按「字段 = ?」处理，否则按 WHERE 片段
//! 处理（片段内自行书写占位符）。这是 aifei 刻意的取舍：**一个签名覆盖两种
//! 高频写法**，避免 `find_by("age > ?", 18)` 误用重载。
//!
//! 片段渲染在 WHERE 子句的**最后且不加括号**（经
//! [`FindBuilder::filter_fragment`](crate::builder::FindBuilder::filter_fragment)），
//! 因此片段可以自带 `ORDER BY` 等尾部子句：
//!
//! ```ignore
//! User::find_by(&db, "age > ? ORDER BY id DESC", (18,)).await?;
//! ```

use std::future::Future;

use alun_db_core::error::{DbError, Result};
use alun_db_core::expr::Expr;
use alun_db_core::field::FieldExpr;
use alun_db_core::query::SqlQuery;
use alun_db_core::stmt::Value;
use alun_db_core::traits::params::Params;
use alun_db_core::traits::{Model, TypeMapper};

use crate::builder::CountBuilder;
use crate::changeset::ChangeSet;
use crate::model_query::{ModelQuery, soft_filter};
use crate::query::SqlExecutor;
use crate::tenant::{scope_for, scoped_predicate};
use alun_db_core::hook;

/// 为所有 `Model` 提供关联查询与删除函数。
///
/// 通过 `prelude` 导出，用户只需 `use alun_db::prelude::*` 即可使用。
///
/// 方法返回 `impl Future + Send`（RPITIT）而非 `async fn`：
/// 保证返回的 Future 可跨线程传递，适配 tokio::spawn / axum handler。
pub trait CrudExt: Model {
    /// 构造变更集：只带主键的「引用对象」，用于免 `SELECT` 的部分更新 / 删除。
    ///
    /// ```ignore
    /// User::of(123).set(User::name(), "james").update(&db).await?;
    /// User::of(123).delete(&db).await?;
    /// ```
    fn of(pk: impl Into<Value> + Send) -> ChangeSet<Self>;

    /// 构造强类型查询：`SELECT * FROM <表>` + 软删除过滤 + 链式条件。
    ///
    /// ```ignore
    /// User::query()
    ///     .filter(User::age().gt(18))
    ///     .order_by(User::created_at().desc())
    ///     .paginate(&db, 1, 20).await?;
    /// ```
    fn query() -> ModelQuery<Self>;

    /// 构造原始 SQL 查询（`sql!` 产物），结果映射为 `Self`。
    ///
    /// 原始 SQL 不做软删除过滤，`WHERE` 由调用方书写；链接条件修饰
    /// （`filter` / `order_by` / `limit` / `offset`）在该形态下不可用。
    fn sql(sql: SqlQuery) -> ModelQuery<Self>;

    /// 按主键查询单条；不存在返回 [`DbError::NotFound`]。
    fn find(
        ex: &impl SqlExecutor,
        pk: impl Into<Value> + Send,
    ) -> impl Future<Output = Result<Self>> + Send;

    /// 按主键查询单条；不存在返回 `None`。
    fn find_or_none(
        ex: &impl SqlExecutor,
        pk: impl Into<Value> + Send,
    ) -> impl Future<Output = Result<Option<Self>>> + Send;

    /// `where_or_field` 查询，返回全部匹配行。
    fn find_by(
        ex: &impl SqlExecutor,
        where_or_field: &str,
        params: impl Params,
    ) -> impl Future<Output = Result<Vec<Self>>> + Send;

    /// `where_or_field` 查询，返回首条。
    fn find_first_by(
        ex: &impl SqlExecutor,
        where_or_field: &str,
        params: impl Params,
    ) -> impl Future<Output = Result<Option<Self>>> + Send;

    /// 单列 `IN` 查询。
    fn find_in<V, I>(
        ex: &impl SqlExecutor,
        field: FieldExpr<V>,
        values: I,
    ) -> impl Future<Output = Result<Vec<Self>>> + Send
    where
        V: TypeMapper + Send,
        I: IntoIterator<Item = V> + Send;

    /// 主键 `IN` 查询。
    fn find_in_ids<V, I>(
        ex: &impl SqlExecutor,
        ids: I,
    ) -> impl Future<Output = Result<Vec<Self>>> + Send
    where
        V: TypeMapper + Send,
        I: IntoIterator<Item = V> + Send;

    /// 全表计数（软删模型自动排除已删行）。
    fn count(ex: &impl SqlExecutor) -> impl Future<Output = Result<i64>> + Send;

    /// `where_or_field` 条件计数。
    fn count_by(
        ex: &impl SqlExecutor,
        where_or_field: &str,
        params: impl Params,
    ) -> impl Future<Output = Result<i64>> + Send;

    /// 按主键删除，返回受影响行数（软删模型改写为标记删除）。
    ///
    /// 命名对齐 aifei 的 `deleteById`：实例侧的 `PersistExt::delete(&self)`
    /// 已占用 `delete`，静态侧带 `_by_pk` 后缀以消歧。
    fn delete_by_pk(
        ex: &impl SqlExecutor,
        pk: impl Into<Value> + Send,
    ) -> impl Future<Output = Result<u64>> + Send;

    /// `where_or_field` 条件删除，返回受影响行数。
    fn delete_by(
        ex: &impl SqlExecutor,
        where_or_field: &str,
        params: impl Params,
    ) -> impl Future<Output = Result<u64>> + Send;

    /// 单列 `IN` 条件删除，返回受影响行数。
    fn delete_in<V, I>(
        ex: &impl SqlExecutor,
        field: FieldExpr<V>,
        values: I,
    ) -> impl Future<Output = Result<u64>> + Send
    where
        V: TypeMapper + Send,
        I: IntoIterator<Item = V> + Send;

    /// 主键 `IN` 删除，返回受影响行数。
    fn delete_in_ids<V, I>(
        ex: &impl SqlExecutor,
        ids: I,
    ) -> impl Future<Output = Result<u64>> + Send
    where
        V: TypeMapper + Send,
        I: IntoIterator<Item = V> + Send;
}

/// 解析 `where_or_field` 为 (WHERE 片段, 参数)：单参数 + 合法标识符 →
/// 「字段 = ?」；否则整段作为 WHERE 片段（参数按占位符顺序绑定）。
fn resolve_where_clause(where_or_field: &str, params: Vec<Value>) -> (String, Vec<Value>) {
    let sql =
        if params.len() == 1 && alun_db_sql::render::validate_identifier(where_or_field).is_ok() {
            format!("{where_or_field} = ?")
        } else {
            where_or_field.to_string()
        };
    (sql, params)
}

/// 同上，返回可直接并入 Builder 过滤链的表达式。
fn resolve_where(where_or_field: &str, params: Vec<Value>) -> Expr {
    let (sql, params) = resolve_where_clause(where_or_field, params);
    Expr::raw_trusted_bound(sql, params.into_iter().map(Expr::Value).collect())
}

/// 构造 `column IN (?, ?, ...)` 表达式；空集合降级为恒假 `1 = 0`。
fn in_expr(column: &'static str, values: Vec<Value>) -> Expr {
    if values.is_empty() {
        return Expr::raw_trusted("1 = 0");
    }
    let placeholders = vec!["?"; values.len()].join(", ");
    Expr::raw_trusted_bound(
        format!("{column} IN ({placeholders})"),
        values.into_iter().map(Expr::Value).collect(),
    )
}

/// 按主键删除（`ChangeSet::delete` 与 `CrudExt::delete_by_pk` 的唯一实现）。
///
/// 软删模型改写为 `UPDATE SET <标记列> = now WHERE <主键> = ?`，参数为
/// `[now, pk]`；触发 `before_delete` / `after_delete`。
pub(crate) async fn delete_by_pk_impl<T: Model, E: SqlExecutor>(ex: &E, pk: Value) -> Result<u64> {
    crate::persist::ensure_writable::<T>()?;
    hook::run_before_delete(T::hooks(), &pk)?;

    // 手写 SQL 无法走 Builder 注入，租户谓词在此显式追加（fail-closed，
    // 模型需隔离但无上下文时直接报错），防止跨租户删除
    let tenant = scoped_predicate::<T>(ex.sql_dialect())?;

    let affected = if let Some(col) = T::SOFT_DELETE {
        let mut sql = format!(
            "UPDATE {} SET {} = ? WHERE {} = ?",
            T::TABLE,
            col,
            T::primary_key_column()
        );
        let mut params = vec![hook::now_value()?, pk.clone()];
        if let Some((predicate, value)) = tenant {
            sql.push_str(&format!(" AND {predicate}"));
            params.push(value);
        }
        ex.exec_sql(sql, params).await?
    } else {
        let mut sql = format!(
            "DELETE FROM {} WHERE {} = ?",
            T::TABLE,
            T::primary_key_column()
        );
        let mut params = vec![pk.clone()];
        if let Some((predicate, value)) = tenant {
            sql.push_str(&format!(" AND {predicate}"));
            params.push(value);
        }
        ex.exec_sql(sql, params).await?
    };

    hook::run_after_delete(T::hooks(), &pk)?;
    // 异步 Hook：提交边界感知（自动提交立即执行；事务内排队至 commit 后）
    crate::hook_async::dispatch_after_delete::<T, _>(ex, &pk).await;
    Ok(affected)
}

impl<T: Model> CrudExt for T {
    fn of(pk: impl Into<Value> + Send) -> ChangeSet<Self> {
        ChangeSet::new(pk.into())
    }

    fn query() -> ModelQuery<Self> {
        ModelQuery::query()
    }

    fn sql(sql: SqlQuery) -> ModelQuery<Self> {
        ModelQuery::from_sql(sql)
    }

    async fn find(ex: &impl SqlExecutor, pk: impl Into<Value> + Send) -> Result<Self> {
        let pk = pk.into();
        // 租户作用域在构造期注入（fail-closed：需隔离但无上下文时直接报错）
        let mut builder = crate::builder::FindBuilder::new()
            .table(T::TABLE)
            .filter(pk_eq::<T>(pk.clone()))
            .scope(scope_for::<T>()?);
        if let Some(filter) = soft_filter::<T>() {
            builder = builder.filter(filter);
        }
        builder
            .first(ex)
            .await?
            .ok_or(DbError::NotFound)
            .and_then(Self::from_row)
    }

    async fn find_or_none(
        ex: &impl SqlExecutor,
        pk: impl Into<Value> + Send,
    ) -> Result<Option<Self>> {
        let pk = pk.into();
        // 租户作用域在构造期注入（fail-closed）
        let mut builder = crate::builder::FindBuilder::new()
            .table(T::TABLE)
            .filter(pk_eq::<T>(pk))
            .scope(scope_for::<T>()?);
        if let Some(filter) = soft_filter::<T>() {
            builder = builder.filter(filter);
        }
        match builder.first(ex).await? {
            Some(row) => Ok(Some(Self::from_row(row)?)),
            None => Ok(None),
        }
    }

    async fn find_by(
        ex: &impl SqlExecutor,
        where_or_field: &str,
        params: impl Params,
    ) -> Result<Vec<Self>> {
        Self::query()
            .filter_fragment(resolve_where(where_or_field, params.into_values()))
            .find(ex)
            .await
    }

    async fn find_first_by(
        ex: &impl SqlExecutor,
        where_or_field: &str,
        params: impl Params,
    ) -> Result<Option<Self>> {
        Self::query()
            .filter_fragment(resolve_where(where_or_field, params.into_values()))
            .find_first(ex)
            .await
    }

    async fn find_in<V, I>(
        ex: &impl SqlExecutor,
        field: FieldExpr<V>,
        values: I,
    ) -> Result<Vec<Self>>
    where
        V: TypeMapper + Send,
        I: IntoIterator<Item = V> + Send,
    {
        let values = values.into_iter().map(|v| v.to_value()).collect();
        Self::query()
            .filter(in_expr(field.column(), values))
            .find(ex)
            .await
    }

    async fn find_in_ids<V, I>(ex: &impl SqlExecutor, ids: I) -> Result<Vec<Self>>
    where
        V: TypeMapper + Send,
        I: IntoIterator<Item = V> + Send,
    {
        let field = FieldExpr::<V>::new(T::TABLE, T::primary_key_column());
        Self::find_in(ex, field, ids).await
    }

    async fn count(ex: &impl SqlExecutor) -> Result<i64> {
        // 租户作用域在构造期注入（fail-closed）
        let mut builder = CountBuilder::new().table(T::TABLE).scope(scope_for::<T>()?);
        if let Some(filter) = soft_filter::<T>() {
            builder = builder.filter(filter);
        }
        builder.exec(ex).await
    }

    async fn count_by(
        ex: &impl SqlExecutor,
        where_or_field: &str,
        params: impl Params,
    ) -> Result<i64> {
        // 租户作用域在构造期注入（fail-closed）
        let mut builder = CountBuilder::new()
            .table(T::TABLE)
            .filter_fragment(resolve_where(where_or_field, params.into_values()))
            .scope(scope_for::<T>()?);
        if let Some(filter) = soft_filter::<T>() {
            builder = builder.filter(filter);
        }
        builder.exec(ex).await
    }

    async fn delete_by_pk(ex: &impl SqlExecutor, pk: impl Into<Value> + Send) -> Result<u64> {
        delete_by_pk_impl::<T, _>(ex, pk.into()).await
    }

    async fn delete_by(
        ex: &impl SqlExecutor,
        where_or_field: &str,
        params: impl Params,
    ) -> Result<u64> {
        let (sql_where, params) = resolve_where_clause(where_or_field, params.into_values());
        delete_by_where::<T, _>(ex, &sql_where, params).await
    }

    async fn delete_in<V, I>(ex: &impl SqlExecutor, field: FieldExpr<V>, values: I) -> Result<u64>
    where
        V: TypeMapper + Send,
        I: IntoIterator<Item = V> + Send,
    {
        let values: Vec<Value> = values.into_iter().map(|v| v.to_value()).collect();
        if values.is_empty() {
            return Ok(0);
        }
        let placeholders = vec!["?"; values.len()].join(", ");
        let sql_where = format!("{} IN ({placeholders})", field.column());
        delete_by_where::<T, _>(ex, &sql_where, values).await
    }

    async fn delete_in_ids<V, I>(ex: &impl SqlExecutor, ids: I) -> Result<u64>
    where
        V: TypeMapper + Send,
        I: IntoIterator<Item = V> + Send,
    {
        let field = FieldExpr::<V>::new(T::TABLE, T::primary_key_column());
        Self::delete_in(ex, field, ids).await
    }
}

/// 按主键构造等值表达式。
fn pk_eq<T: Model>(pk: Value) -> Expr {
    Expr::raw_trusted_bound(
        format!("{} = ?", T::primary_key_column()),
        vec![Expr::Value(pk)],
    )
}

/// 条件删除的唯一实现（软删模型改写为标记删除）。
async fn delete_by_where<T: Model, E: SqlExecutor>(
    ex: &E,
    sql_where: &str,
    params: Vec<Value>,
) -> Result<u64> {
    crate::persist::ensure_writable::<T>()?;

    // 手写 SQL 无法走 Builder 注入，租户谓词在此显式追加（fail-closed）
    let tenant = scoped_predicate::<T>(ex.sql_dialect())?;

    if let Some(col) = T::SOFT_DELETE {
        let mut sql = format!("UPDATE {} SET {} = ? WHERE {}", T::TABLE, col, sql_where);
        let mut params = params;
        params.insert(0, hook::now_value()?);
        if let Some((predicate, value)) = tenant {
            sql.push_str(&format!(" AND {predicate}"));
            params.push(value);
        }
        return ex.exec_sql(sql, params).await;
    }
    let mut sql = format!("DELETE FROM {} WHERE {}", T::TABLE, sql_where);
    let mut params = params;
    if let Some((predicate, value)) = tenant {
        sql.push_str(&format!(" AND {predicate}"));
        params.push(value);
    }
    ex.exec_sql(sql, params).await
}

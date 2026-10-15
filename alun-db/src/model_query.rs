//! `ModelQuery<T>` —— 强类型查询脊柱。
//!
//! 设计对齐 aifei 的 `Dao`：**SQL 表达语义，终态动词控制执行方式**。
//! 两条构造入口产出同一种查询对象：
//!
//! - [`ModelQuery::query`]（`User::query()`）：从表名 + 链式条件构造，自动追加
//!   软删除过滤（模型声明了软删标记列时）
//! - [`ModelQuery::from_sql`]（`User::sql(sql!(...))`）：原始 SQL 直通，不做任何
//!   改写（原始 SQL 由调用方负责语义完整）
//!
//! 终态动词与 [`SqlQueryExt`](crate::query::SqlQueryExt) 完全一致，只是把
//! [`Row`] 映射为 `T`：`find` / `find_first` / `find_one` / `find_one_or_null`
//! / `exists` / `paginate` / `for_each` / `for_each_page` / `scalar` / `execute`。

use std::marker::PhantomData;

use alun_db_core::error::{DbError, Result};
use alun_db_core::expr::{BinaryOp, Expr};
use alun_db_core::page::{Cursor, Page};
use alun_db_core::query::SqlQuery;
use alun_db_core::row::Row;
use alun_db_core::statement::{ColumnRef, JoinKind, Order, OrderBy};
use alun_db_core::stmt::Value;
use alun_db_core::traits::dialect::Dialect;
use alun_db_core::traits::{Model, TypeMapper};

use crate::builder::FindBuilder;
use crate::query::{
    SqlExecutor, run_execute, run_exists, run_find, run_find_first, run_find_one, run_for_each,
    run_for_each_page, run_paginate, run_scalar,
};

/// 执行一段 ORM 语句并埋点（F10）。
///
/// 这里是**唯一能拿到模型表名**的执行层：`Db` / `Transaction` 只有 SQL 文本，
/// 只能按 `raw` 归类；ORM 能精确到 `table`，慢查询才定位得到具体模型。
///
/// 未编译 `metrics` feature 时退化为直接 `await`——同名同签名的空壳，
/// 让调用点保持单一形态，不必到处写 `#[cfg]`。
#[cfg(feature = "metrics")]
use crate::metrics::instrument as observed;

#[cfg(not(feature = "metrics"))]
async fn observed<T>(
    _op: &'static str,
    _table: &'static str,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    fut.await
}

/// 软删除过滤表达式：`<标记列> IS NULL`。
///
/// 由结构化表达式构造（列名经渲染器引用校验），未启用软删时返回 `None`。
pub(crate) fn soft_filter<T: Model>() -> Option<Expr> {
    T::SOFT_DELETE.map(|column| Expr::Binary {
        left: Box::new(Expr::Column {
            table: T::TABLE,
            column,
        }),
        op: BinaryOp::IsNull,
        right: Box::new(Expr::Value(Value::Null)),
    })
}

/// 强类型查询。
///
/// `T` 只用于把查询结果映射为模型；构造时不做任何隐式列投影。
pub struct ModelQuery<T> {
    /// 查询来源。
    source: Source,
    /// 链式修饰在原始 SQL 源上不可用，记录后由终态动词语句化报错。
    unsupported: Option<&'static str>,
    /// 构建期已发生但需推迟到执行期上报的错误（如 `join::<U>` 解析从表租户
    /// 作用域时无租户上下文）。`join` 的签名是 `-> Self`，无法即时返回 `Err`，
    /// 故在此暂存，由 [`into_sql`](Self::into_sql) 最先检查——fail-closed 不丢错。
    deferred_error: Option<String>,
    /// 原生 SQL 的**租户完整性声明**（见 [`ModelQuery::raw_tenant_annotated`]）。
    ///
    /// `false`（默认）时，租户模型 + `Scoped` 上下文下的原生 SQL 一律拒绝——
    /// 即便其中已含 `/*tenant*/` 占位。因为框架**无法从 SQL 文本推断是否漏标**，
    /// 而"漏标一张 JOIN 从表"就是一次静默跨租户泄漏，必须由调用方显式承诺。
    raw_tenant_annotated: bool,
    /// 标记 `T`，不占用运行时空间。
    _marker: PhantomData<fn() -> T>,
}

/// 查询来源。
enum Source {
    /// 原始 SQL（`sql!` 宏产物）。
    Sql(SqlQuery),
    /// 表名 + 链式条件（`FindBuilder` 渲染，标识符与值都受管控）。
    ///
    /// 用 `Box` 承载：`FindBuilder` 体积远大于 `SqlQuery`，装箱可缩小枚举整体尺寸。
    Builder(Box<FindBuilder>),
}

impl<T: Model> ModelQuery<T> {
    /// 从表名构造查询（`User::query()`）：自动追加软删除过滤。
    pub fn query() -> Self {
        let builder = FindBuilder::new().table(T::TABLE);
        let builder = match soft_filter::<T>() {
            Some(filter) => builder.filter(filter),
            None => builder,
        };
        ModelQuery {
            source: Source::Builder(Box::new(builder)),
            unsupported: None,
            deferred_error: None,
            raw_tenant_annotated: false,
            _marker: PhantomData,
        }
    }

    /// 从原始 SQL 构造查询（`User::sql(sql!(...))`）。
    ///
    /// 原始 SQL 不做软删除过滤，也不参与列投影——调用方自己写 `WHERE`。
    pub fn from_sql(sql: SqlQuery) -> Self {
        ModelQuery {
            source: Source::Sql(sql),
            unsupported: None,
            deferred_error: None,
            raw_tenant_annotated: false,
            _marker: PhantomData,
        }
    }

    /// **显式声明**：本原生 SQL 已在**每一张**参与查询的租户表上标注了
    /// `/*tenant:别名*/` 占位。
    ///
    /// # 为什么必须显式声明
    ///
    /// 框架**无法**从任意 SQL 文本推断"租户表是否都标注了"——它只能看到
    /// "是否注入过至少一次"。因此若只凭"出现了占位"就放行，下面这条 SQL 会
    /// **静默地跨租户泄漏**（`c` 的行完全不过滤，守卫却因 `o` 有占位而放行）：
    ///
    /// ```sql
    /// SELECT ... FROM biz_order o JOIN biz_customer c ON ...
    ///  WHERE o.status = ? AND /*tenant:o*/        -- 漏标 /*tenant:c*/ → 泄漏
    /// ```
    ///
    /// 本方法把"完整性"从**框架的隐式假设**提升为**调用点可见的承诺**：
    /// 未调用它的原生 SQL，在「租户模型 + `Scoped` 上下文」下**一律拒绝**，
    /// 无论是否含占位。调用它意味着调用方已逐一核对每张租户表——该调用可用
    /// `grep raw_tenant_annotated` 盘点，纳入代码评审。
    ///
    /// # 更安全的替代
    ///
    /// 能用结构化联查就不要用原生 SQL：`ModelQuery::join::<U>()` 由框架枚举
    /// 从表模型并自动注入租户条件，**不存在漏标风险**。原生 SQL 的占位通道
    /// 仅用于结构化表达不了的场景（`UNION`、CTE、窗口函数、方言特性）。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// let sql = sql!(
    ///     "SELECT o.id, c.name FROM biz_order o JOIN biz_customer c ON c.id = o.customer_id",
    ///     " WHERE o.status = ", { status },
    ///     " AND /*tenant:o*/ AND /*tenant:c*/"
    /// );
    /// let rows = Order::sql(sql).raw_tenant_annotated().find(db()).await?;
    /// ```
    pub fn raw_tenant_annotated(mut self) -> Self {
        self.raw_tenant_annotated = true;
        self
    }

    /// 记录一次「原始 SQL 源不支持的操作」，由终态动词统一报错。
    fn reject(mut self, op: &'static str) -> Self {
        if matches!(self.source, Source::Sql(_)) && self.unsupported.is_none() {
            self.unsupported = Some(op);
        }
        self
    }

    /// 追加 WHERE 条件（多次调用之间为 AND）。
    pub fn filter(mut self, expr: Expr) -> Self {
        self = self.reject("filter");
        if let Source::Builder(builder) = self.source {
            self.source = Source::Builder(Box::new(builder.filter(expr)));
        }
        self
    }

    /// 追加「末位 WHERE 片段」：渲染在 WHERE 子句最后且不加括号，
    /// 因此片段可自带 `ORDER BY` / `LIMIT` 等尾部子句（`whereOrField` 语义）。
    pub fn filter_fragment(mut self, expr: Expr) -> Self {
        self = self.reject("filter_fragment");
        if let Source::Builder(builder) = self.source {
            self.source = Source::Builder(Box::new(builder.filter_fragment(expr)));
        }
        self
    }

    /// 关联一张从表（投影仍以主表 `T` 为准；从表字段经
    /// `Expr::Column { table: U::TABLE, column }` 取用）。
    ///
    /// # 租户安全（**结构化注入** —— 推荐的多租户联查通道）
    ///
    /// 若 `U` 声明了 `#[tenant]`，框架会从**当前租户上下文**解析其租户条件，
    /// 并追加到该 JOIN 的 `ON` 之后。调用方**无需**、也**不应**手写租户条件。
    ///
    /// 相比原生 SQL 的 `/*tenant:别名*/` 占位，这是**完整**的隔离：
    /// - 从表模型由类型参数 `U` 枚举得到，**不存在"漏标一张表"导致的静默
    ///   跨租户泄漏**；
    /// - 占位方案的守卫只能看到"是否注入过至少一次"，无法知道 SQL 里有几张
    ///   租户表——漏标即泄漏。本方法从机制上消除该风险。
    ///
    /// # 与主表作用域的关系
    ///
    /// 主表 `T` 的作用域仍由 `scope_for::<T>()` 在渲染前注入（见 `into_sql`）；
    /// 本方法只负责从表。二者的租户值同源（同一个 `Scoped` 上下文）。
    ///
    /// # 失败语义（fail-closed）
    ///
    /// `U` 需要隔离但当前无租户上下文时，错误被推迟到执行期（`find` /
    /// `paginate` 等终态动词）上报，**不会**静默退化为"不过滤"。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// // 订单 JOIN 客户，二者都声明了 #[tenant]
    /// let rows = Order::query()
    ///     .join::<Customer>(
    ///         JoinKind::Inner,
    ///         Expr::Binary {
    ///             left: Box::new(Expr::Column { table: Customer::TABLE, column: "id" }),
    ///             op: BinaryOp::Eq,
    ///             right: Box::new(Expr::Column { table: Order::TABLE, column: "customer_id" }),
    ///         },
    ///     )
    ///     .find(db())
    ///     .await?;
    /// // 渲染为：... INNER JOIN "biz_customer" ON "biz_customer"."id" = "biz_order"."customer_id"
    /// //          AND ("biz_customer"."tenant_id" = ?)
    /// ```
    pub fn join<U: Model>(mut self, kind: JoinKind, on: Expr) -> Self {
        self = self.reject("join");
        if let Source::Builder(builder) = self.source {
            let (scope, err) = match crate::tenant::join_scope_for::<U>() {
                Ok(scope) => (scope, None),
                Err(e) => (None, Some(e.to_string())),
            };
            if let Some(msg) = err
                && self.deferred_error.is_none()
            {
                self.deferred_error = Some(msg);
            }
            self.source = Source::Builder(Box::new(builder.join_scoped(kind, U::TABLE, on, scope)));
        }
        self
    }

    /// 追加 ORDER BY。
    pub fn order_by(self, order: OrderBy) -> Self {
        let mut this = self.reject("order_by");
        let ColumnRef::Column(column) = order.expr else {
            // 表达式排序（别名 / 函数列）未经 Builder 抽象，交由原始 SQL 表达
            this.unsupported = Some("order_by(expression)");
            return this;
        };
        if let Source::Builder(builder) = this.source {
            this.source = Source::Builder(Box::new(builder.order_by(column, order.direction)));
        }
        this
    }

    /// 按**白名单**排序——`ORDER BY` 的安全入口。
    ///
    /// `ORDER BY` 的列名无法用绑定参数表达，因此是 SQL 注入的经典入口。
    /// 本方法把「用户输入 → 列名」的映射收敛为一次白名单查找：
    ///
    /// - `field` 为 `None` → 不排序，原样返回（列表接口未传排序参数）
    /// - `field` 命中 `allowed` → 按 `desc` 追加 ORDER BY
    /// - `field` **未命中** → 返回 [`DbError::InvalidValue`]（**不静默忽略**：
    ///   静默忽略会让调用方以为已排序，从而在分页场景下产生难以察觉的
    ///   重复/遗漏行）
    ///
    /// # 参数
    ///
    /// - `allowed`: `&'static str` 白名单。必须是编译期常量，从而保证最终
    ///   拼进 SQL 的列名不可能来自用户输入。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// /// 允许排序的列（唯一事实源，同时可作为 API 文档）
    /// const SORTABLE: &[&str] = &["id", "created_at", "name"];
    ///
    /// let sort = SortQuery { sort_by: Some("name".into()), order: SortOrder::Desc };
    /// let rows = User::query()
    ///     .order_by_whitelisted(sort.field(), sort.is_desc(), SORTABLE)?
    ///     .paginate(db(), 1, 20)
    ///     .await?;
    /// ```
    pub fn order_by_whitelisted(
        self,
        field: Option<&str>,
        desc: bool,
        allowed: &[&'static str],
    ) -> Result<Self> {
        let Some(field) = field else {
            return Ok(self);
        };
        let column = allowed
            .iter()
            .find(|c| **c == field)
            .copied()
            .ok_or_else(|| {
                DbError::InvalidValue(format!(
                    "`{field}` is not a sortable field; allowed: {allowed:?}"
                ))
            })?;
        Ok(self.order_by(OrderBy {
            expr: ColumnRef::Column(column),
            direction: if desc { Order::Desc } else { Order::Asc },
            nulls: None,
        }))
    }

    /// 设置 LIMIT。
    pub fn limit(mut self, n: u64) -> Self {
        self = self.reject("limit");
        if let Source::Builder(builder) = self.source {
            self.source = Source::Builder(Box::new(builder.limit(n)));
        }
        self
    }

    /// 设置 OFFSET。
    pub fn offset(mut self, n: u64) -> Self {
        self = self.reject("offset");
        if let Source::Builder(builder) = self.source {
            self.source = Source::Builder(Box::new(builder.offset(n)));
        }
        self
    }

    /// 设置 keyset 游标分页条件（避免深翻页性能退化）。
    pub fn page_after(
        mut self,
        sort_col: &'static str,
        pk_col: &'static str,
        direction: Order,
        after: Option<Cursor>,
    ) -> Self {
        self = self.reject("page_after");
        if let Source::Builder(builder) = self.source {
            self.source = Source::Builder(Box::new(
                builder.page_after(sort_col, pk_col, direction, after),
            ));
        }
        self
    }

    /// 渲染为带占位符的 `SqlQuery`（终态动词的统一入口）。
    fn into_sql(self, dialect: &'static dyn Dialect) -> Result<SqlQuery> {
        // 构建期暂存的错误（如 `join::<U>` 无租户上下文）优先上报：fail-closed，
        // 绝不允许"带着错误继续渲染出不过滤的 SQL"。
        if let Some(msg) = self.deferred_error {
            return Err(DbError::InvalidValue(msg));
        }
        if let Some(op) = self.unsupported {
            return Err(DbError::InvalidValue(format!(
                "ModelQuery::{op} is only supported on `{}::query()`; \
                 write the clause into `sql!` when using a raw-SQL query",
                T::TABLE
            )));
        }
        match self.source {
            Source::Sql(sql) => {
                // 原生 SQL 绕过 AST 注入，无法自动追加租户条件。两条路径：
                // ① 调用方用 `/*tenant:别名*/` 显式声明了每张租户表的隔离位置
                //    且以 `raw_tenant_annotated()` 承诺完整性 → 框架注入后放行；
                // ② 否则由守卫拒绝（fail-closed，不静默放行全量）。
                let (sql, tenant_bound) = crate::tenant::rewrite_raw_tenant::<T>(sql, dialect)?;
                crate::tenant::ensure_raw_allowed::<T>(tenant_bound, self.raw_tenant_annotated)?;
                Ok(sql)
            }
            Source::Builder(builder) => {
                // 租户作用域在渲染前注入：模型需隔离但无上下文时 `scope_for`
                // 返回错误（fail-closed），绝不静默放行全量数据
                let scope = crate::tenant::scope_for::<T>()?;
                let rendered = builder.scope(scope).render(dialect)?;
                Ok(SqlQuery::new(rendered.sql, rendered.params))
            }
        }
    }

    /// 查询全部，映射为 `Vec<T>`。
    pub async fn find<E: SqlExecutor>(self, ex: &E) -> Result<Vec<T>> {
        let sql = self.into_sql(ex.sql_dialect())?;
        observed("select", T::TABLE, run_find(sql, ex))
            .await?
            .into_iter()
            .map(T::from_row)
            .collect()
    }

    /// 查询首条；无匹配返回 `Ok(None)`。
    pub async fn find_first<E: SqlExecutor>(self, ex: &E) -> Result<Option<T>> {
        let sql = self.into_sql(ex.sql_dialect())?;
        match observed("select", T::TABLE, run_find_first(sql, ex)).await? {
            Some(row) => Ok(Some(T::from_row(row)?)),
            None => Ok(None),
        }
    }

    /// 查询恰好一条。无匹配返回 [`DbError::NotFound`]，多于一条返回
    /// [`DbError::InvalidValue`]。
    pub async fn find_one<E: SqlExecutor>(self, ex: &E) -> Result<T> {
        let sql = self.into_sql(ex.sql_dialect())?;
        let row = observed("select", T::TABLE, run_find_one(sql, ex, false))
            .await?
            .ok_or(DbError::NotFound)?;
        T::from_row(row)
    }

    /// 查询 0 或 1 条。多于一条返回 [`DbError::InvalidValue`]。
    pub async fn find_one_or_null<E: SqlExecutor>(self, ex: &E) -> Result<Option<T>> {
        let sql = self.into_sql(ex.sql_dialect())?;
        match observed("select", T::TABLE, run_find_one(sql, ex, true)).await? {
            Some(row) => Ok(Some(T::from_row(row)?)),
            None => Ok(None),
        }
    }

    /// 是否存在匹配行。
    pub async fn exists<E: SqlExecutor>(self, ex: &E) -> Result<bool> {
        let sql = self.into_sql(ex.sql_dialect())?;
        observed("select", T::TABLE, run_exists(sql, ex)).await
    }

    /// 分页查询（页码从 1 开始），返回完整 `total` / `pages`。
    ///
    /// 查询自身不得自带 `LIMIT` / `OFFSET`（分页子句由本方法追加）：链式
    /// [`limit`](Self::limit) / [`offset`](Self::offset) 请改为本方法的参数，
    /// 自带 `#limit(...)` 的原始 SQL 会因重复分页子句而报错。
    pub async fn paginate<E: SqlExecutor>(self, ex: &E, page: u64, size: u64) -> Result<Page<T>> {
        let sql = self.into_sql(ex.sql_dialect())?;
        let page_rows: Page<Row> =
            observed("select", T::TABLE, run_paginate(sql, ex, page, size)).await?;
        let total = page_rows.total;
        let page_num = page_rows.page;
        let size = page_rows.size;
        let pages = page_rows.pages;
        let list = page_rows
            .list
            .into_iter()
            .map(T::from_row)
            .collect::<Result<Vec<T>>>()?;
        Ok(Page {
            list,
            total,
            page: page_num,
            size,
            pages,
        })
    }

    /// 逐行遍历（结果一次性取回）。回调返回 `Ok(false)` 提前结束。
    pub async fn for_each<E, F>(self, ex: &E, mut f: F) -> Result<()>
    where
        E: SqlExecutor,
        F: FnMut(T) -> Result<bool> + Send,
    {
        let sql = self.into_sql(ex.sql_dialect())?;
        observed(
            "select",
            T::TABLE,
            run_for_each(sql, ex, |row| f(T::from_row(row)?)),
        )
        .await
    }

    /// 按页遍历（内存占用受 `size` 约束）。回调返回 `Ok(false)` 提前结束。
    ///
    /// 查询不得自带 `LIMIT` / `OFFSET`（分页子句由本方法追加），否则返回
    /// [`DbError::InvalidValue`]。
    pub async fn for_each_page<E, F>(self, ex: &E, size: u64, mut f: F) -> Result<()>
    where
        E: SqlExecutor,
        F: FnMut(Page<T>) -> Result<bool> + Send,
    {
        let sql = self.into_sql(ex.sql_dialect())?;
        observed(
            "select",
            T::TABLE,
            run_for_each_page(sql, ex, size, |page| {
                let total = page.total;
                let page_num = page.page;
                let size = page.size;
                let pages = page.pages;
                let list = page
                    .list
                    .into_iter()
                    .map(T::from_row)
                    .collect::<Result<Vec<T>>>()?;
                f(Page {
                    list,
                    total,
                    page: page_num,
                    size,
                    pages,
                })
            }),
        )
        .await
    }

    /// 把查询结果当单值读取（首行首列）。
    ///
    /// 适用于 `sql!("SELECT COUNT(*) ...")` 这类聚合查询；模型查询不带聚合时
    /// 读到的只是首行首列，请改用 `count` 系列。
    pub async fn scalar<V: TypeMapper, E: SqlExecutor>(self, ex: &E) -> Result<V> {
        let sql = self.into_sql(ex.sql_dialect())?;
        observed("select", T::TABLE, run_scalar(sql, ex)).await
    }

    /// 执行写语句，返回受影响行数。
    pub async fn execute<E: SqlExecutor>(self, ex: &E) -> Result<u64> {
        crate::persist::ensure_writable::<T>()?;
        let sql = self.into_sql(ex.sql_dialect())?;
        // 写路径：`persist` 的 insert/update/delete 有各自语义标签，这里按
        // 通用 `execute` 归类（`ModelQuery::execute` 是自定义 SQL 出口）。
        observed("execute", T::TABLE, run_execute(sql, ex)).await
    }
}

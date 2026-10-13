//! 查询 Builder。
//!
//! 链式组装查询条件，所有值经 `RenderCtx::bind` 参数化绑定，
//! 表名 / 列名经标识符校验后引用，杜绝 SQL 注入。

use alun_db_core::error::Result;
use alun_db_core::expr::Expr;
use alun_db_core::page::{Cursor, Page};
use alun_db_core::row::Row;
use alun_db_core::statement::{Join, JoinKind, Order, ScopeFilter, ScopeMode};
use alun_db_core::traits::dialect::{Dialect, RenderCtx, RenderedSql};

use crate::query::SqlExecutor;

/// 排序列。
#[derive(Debug, Clone)]
struct OrderColumn {
    /// 列名。可为简单 `id` 或带表前缀 `squads.id`。
    column: String,
    /// 排序方向。
    direction: Order,
}

/// Keyset 游标条件（供 `page_after` 使用）。
///
/// 承载排序列、主键列、排序方向以及「上一页最后一行」的游标值。
#[derive(Debug, Clone)]
struct KeysetClause {
    /// 排序列名。
    sort_col: &'static str,
    /// 主键列名。
    pk_col: &'static str,
    /// 排序方向，决定游标比较符是 `>` 还是 `<`。
    direction: Order,
    /// 上一页末尾的游标。为 `None` 时表示从第一页开始。
    cursor: Option<Cursor>,
}

/// 渲染并引用一个用户提供的标识符（表名 / 列名）。
///
/// # 错误
///
/// 标识符含非法字符（非 `[a-zA-Z_][a-zA-Z0-9_]*`）时返回
/// [`alun_db_core::error::DbError::InvalidValue`]。
fn quote_identifier(dialect: &dyn Dialect, name: &str) -> Result<String> {
    alun_db_sql::render::validate_identifier(name)?;
    Ok(dialect.quote_identifier(name))
}

/// 引用一个限定标识符：支持简单列名 `id` 或带表前缀 `table.id`。
///
/// 按 `.` 拆分后逐段校验并引用，供 JOIN 场景按 `table.column` 排序消歧。
///
/// # 错误
///
/// 任一段标识符含非法字符时返回 [`alun_db_core::error::DbError::InvalidValue`]。
fn quote_qualified_identifier(dialect: &dyn Dialect, name: &str) -> Result<String> {
    let parts: Vec<&str> = name.split('.').collect();
    let quoted = parts
        .into_iter()
        .map(|p| {
            alun_db_sql::render::validate_identifier(p)?;
            Ok(dialect.quote_identifier(p))
        })
        .collect::<Result<Vec<String>>>()?;
    Ok(quoted.join("."))
}

/// 渲染 keyset 游标比较条件 `(sort_col, pk) > (?, ?)` 或 `< (?, ?)`，并绑定游标值。
///
/// 排序正向（`Order::Asc`）为 `>`，反向为 `<`。游标为 `None` 时表示从第一页
/// 开始，返回 `None`（不产生 WHERE 片段）。
fn render_keyset_where(
    dialect: &dyn Dialect,
    ctx: &mut RenderCtx<'_>,
    keyset: &KeysetClause,
) -> Result<Option<String>> {
    let Some(cursor) = &keyset.cursor else {
        return Ok(None);
    };
    let sort_col = quote_identifier(dialect, keyset.sort_col)?;
    let pk_col = quote_identifier(dialect, keyset.pk_col)?;
    let op = if keyset.direction == Order::Asc {
        ">"
    } else {
        "<"
    };
    let key_ph = ctx.bind(cursor.key.clone());
    let pk_ph = ctx.bind(cursor.pk.clone());
    Ok(Some(format!(
        "({sort_col}, {pk_col}) {op} ({key_ph}, {pk_ph})"
    )))
}

/// 查询 Builder。
///
/// 链式组装查询条件，最后通过 `exec` 方法执行。
///
/// ## 安全性
///
/// - `filter` 中的值走参数绑定（`?` / `$n`），不内联进 SQL
/// - `table` / `order_by` 的标识符经白名单校验（`[a-zA-Z_][a-zA-Z0-9_]*`）
pub struct FindBuilder {
    /// 表名。
    table: Option<String>,
    /// WHERE 条件。多次调用 `filter` 之间为 AND。
    filter: Option<Expr>,
    /// 末位 WHERE 片段：渲染在 WHERE 子句最后、**不参与 AND 括号包裹**。
    ///
    /// 用于承载 aifei `whereOrField` 语义的原始片段——片段可以自带
    /// `ORDER BY` / `LIMIT` 等尾部子句，一旦被 `AND (...)` 包住即成语法错误。
    trailing: Option<Expr>,
    /// JOIN 子句列表。
    joins: Vec<Join>,
    /// Keyset 游标条件。
    keyset: Option<KeysetClause>,
    /// 租户作用域。由模型级入口（`scope_for::<T>()`）注入，渲染时拼入 WHERE 最外层。
    scope: ScopeMode,
    /// ORDER BY 列表。
    order_by: Vec<OrderColumn>,
    /// LIMIT。
    limit: Option<u64>,
    /// OFFSET。
    offset: Option<u64>,
}

impl FindBuilder {
    /// 创建空 Builder。
    pub fn new() -> Self {
        FindBuilder {
            table: None,
            filter: None,
            trailing: None,
            joins: Vec::new(),
            keyset: None,
            scope: ScopeMode::Unset,
            order_by: Vec::new(),
            limit: None,
            offset: None,
        }
    }

    /// 设置租户作用域。
    ///
    /// 由模型级入口注入（`Invoice::query()` / `Invoice::find(..)` 等）。
    /// 手写 Builder 操作租户表时需自行调用，否则该语句不隔离。
    pub fn scope(mut self, scope: ScopeMode) -> Self {
        self.scope = scope;
        self
    }

    /// 设置表名。
    pub fn table(mut self, name: impl Into<String>) -> Self {
        self.table = Some(name.into());
        self
    }

    /// 追加 WHERE 条件（与已有条件 AND 组合）。
    pub fn filter(mut self, expr: Expr) -> Self {
        self.filter = Some(match self.filter.take() {
            Some(existing) => Expr::and_all([existing, expr]),
            None => expr,
        });
        self
    }

    /// 设置「末位 WHERE 片段」（多次调用保留最后一次）。
    ///
    /// 与 [`filter`](Self::filter) 的区别：本片段渲染在 WHERE 子句的**最后**，
    /// 且不被 `AND (...)` 包裹，因此可以自带 `ORDER BY` / `LIMIT` 等尾部子句。
    /// 这是 `whereOrField` 双态签名（如 `find_by(&db, "age > ? ORDER BY id DESC", …)`）
    /// 所依赖的能力：片段本身即完整的 WHERE 尾部。
    pub fn filter_fragment(mut self, expr: Expr) -> Self {
        self.trailing = Some(expr);
        self
    }

    /// 添加 JOIN 子句（**不注入租户条件**）。
    ///
    /// 多个 `join` 按调用顺序拼接。`table` 与 `on` 中 `Expr::Column` 的
    /// 表名 / 列名均为编译期已知的静态字符串，无注入风险；`on` 中的绑定值
    /// 在 WHERE 之前绑定，保证参数占位符顺序正确。
    ///
    /// # 租户安全
    ///
    /// 本方法的表名是裸字符串，Builder **无法**判定从表模型是否声明了
    /// `#[tenant]`，因此不做隔离。需要「从表自动租户隔离」的联查请走
    /// [`ModelQuery::join`](crate::model_query::ModelQuery::join)（模型级入口，
    /// 由框架枚举从表并注入条件），或改用 [`join_scoped`](Self::join_scoped)
    /// 显式传入从表作用域。
    pub fn join(mut self, kind: JoinKind, table: &'static str, on: Expr) -> Self {
        self.joins.push(Join {
            kind,
            table,
            on,
            scope: None,
        });
        self
    }

    /// 添加**带租户作用域**的 JOIN 子句。
    ///
    /// 与 [`join`](Self::join) 的唯一区别是 `scope`：为 `Some` 时，渲染器会把
    /// 该从表的租户条件追加到 `ON` 之后，从而在**结构化层面**保证联查不跨租户。
    /// 这是 `ModelQuery::join::<U>` 的底层实现，也是原生 SQL
    /// `/*tenant:别名*/` 占位所无法提供的**完整性**保障（占位方案漏标一张表
    /// 就是一次静默跨租户泄漏）。
    ///
    /// `scope` 通常由 `crate::tenant::join_scope_for::<U>()` 从当前租户上下文
    /// 解析得到；`None` 表示该从表不参与隔离（无 `#[tenant]` / 显式跨租户）。
    pub fn join_scoped(
        mut self,
        kind: JoinKind,
        table: &'static str,
        on: Expr,
        scope: Option<ScopeFilter>,
    ) -> Self {
        self.joins.push(Join {
            kind,
            table,
            on,
            scope,
        });
        self
    }

    /// 设置 Keyset 游标分页条件。
    ///
    /// Keyset 分页避免了深翻页时 `OFFSET` 走全量的性能退化：提供上一页末尾
    /// 的游标 `after` 后，会在 WHERE 中追加行比较 `(sort_col, pk) > (?, ?)`
    /// （正向排序）或 `<`（反向排序），仅扫描游标之后的记录。
    ///
    /// # 参数
    ///
    /// - `sort_col`：排序列名（静态字符串）
    /// - `pk_col`：主键列名（静态字符串）
    /// - `direction`：排序方向，决定比较符方向
    /// - `after`：上一页的游标；传 `None` 表示从第一页开始
    pub fn page_after(
        mut self,
        sort_col: &'static str,
        pk_col: &'static str,
        direction: Order,
        after: Option<Cursor>,
    ) -> Self {
        self.keyset = Some(KeysetClause {
            sort_col,
            pk_col,
            direction,
            cursor: after,
        });
        self
    }

    /// 添加 ORDER BY 列。
    pub fn order_by(mut self, column: impl Into<String>, direction: Order) -> Self {
        self.order_by.push(OrderColumn {
            column: column.into(),
            direction,
        });
        self
    }

    /// 设置 LIMIT。
    pub fn limit(mut self, n: u64) -> Self {
        self.limit = Some(n);
        self
    }

    /// 设置 OFFSET。
    pub fn offset(mut self, n: u64) -> Self {
        self.offset = Some(n);
        self
    }

    /// 渲染为参数化 SQL。
    pub(crate) fn render(&self, dialect: &dyn Dialect) -> Result<RenderedSql> {
        let table = self.table.as_deref().ok_or_else(|| {
            alun_db_core::error::DbError::InvalidValue("builder: table is required".into())
        })?;
        let mut ctx = RenderCtx::new(dialect);
        let mut sql = format!("SELECT * FROM {}", quote_identifier(dialect, table)?);
        // JOIN：渲染 ON 时同步绑定其参数，先于 WHERE 绑定，保证占位符顺序
        for join in &self.joins {
            let kind = match join.kind {
                JoinKind::Inner => "INNER JOIN",
                JoinKind::Left => "LEFT JOIN",
                JoinKind::Right => "RIGHT JOIN",
                JoinKind::Full => "FULL JOIN",
            };
            let join_table = quote_identifier(dialect, join.table)?;
            let mut on = dialect.render_expr(&join.on, &mut ctx);
            // 从表租户作用域：追加到 `ON` 之后（**不是** WHERE）。
            // 这样在 LEFT/RIGHT JOIN 下语义正确：只联「本租户」的行，
            // 且不会把外连接退化为内连接。这是结构化注入——从表模型由
            // `ModelQuery::join::<U>` 枚举得到，因此不存在「漏标一张表」
            // 导致的跨租户泄漏（对比原生 SQL 的 `/*tenant:别名*/` 占位）。
            if let Some(scope) = &join.scope {
                alun_db_sql::render::validate_identifier(scope.column)?;
                let col = dialect.quote_identifier(scope.column);
                let placeholder = ctx.bind(scope.value.clone());
                on.push_str(&format!(" AND ({join_table}.{col} = {placeholder})"));
            }
            sql.push_str(&format!(" {kind} {join_table} ON {on}"));
        }
        // WHERE 片段：filter（AND）与 keyset 游标（AND）合并后用 AND 连接，
        // 二者皆无时不发 WHERE 子句
        let mut where_parts: Vec<String> = Vec::new();
        if let Some(filter) = &self.filter {
            where_parts.push(dialect.render_expr(filter, &mut ctx));
        }
        // Keyset 游标条件：追加行比较并参数化绑定游标值
        if let Some(k) = &self.keyset
            && let Some(part) = render_keyset_where(dialect, &mut ctx, k)?
        {
            where_parts.push(part);
        }
        // 租户作用域：置于 filter / keyset 之后、末位片段之前。
        // 顺序在此处至关重要——末位片段可自带 ORDER BY / LIMIT 等尾部子句，
        // 若排在片段之后会落进尾部子句内；同时保证参数绑定顺序与占位符一致。
        if let Some(part) =
            alun_db_sql::render::scope::render_scope_mode(&self.scope, true, &mut ctx)
        {
            where_parts.push(part);
        }
        // 末位片段：不加括号，允许自带 ORDER BY / LIMIT 等尾部子句
        if let Some(trailing) = &self.trailing {
            where_parts.push(dialect.render_expr(trailing, &mut ctx));
        }
        if !where_parts.is_empty() {
            sql.push_str(&format!(" WHERE {}", where_parts.join(" AND ")));
        }
        if !self.order_by.is_empty() {
            let parts: Vec<String> = self
                .order_by
                .iter()
                .map(|o| {
                    let dir = match o.direction {
                        Order::Asc => "ASC",
                        Order::Desc => "DESC",
                    };
                    Ok(format!(
                        "{} {dir}",
                        quote_qualified_identifier(dialect, &o.column)?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            sql.push_str(&format!(" ORDER BY {}", parts.join(", ")));
        }
        // LIMIT / OFFSET 内联为字面量（见 `Dialect::push_limit_offset`：
        // 参数化 LIMIT 会让 SQLite 每次查询多花约 2.7 µs）
        dialect.push_limit_offset(self.limit, self.offset, &mut sql);
        Ok(RenderedSql {
            sql,
            params: ctx.params,
        })
    }

    /// 执行查询。
    pub async fn exec<E: SqlExecutor>(self, ex: &E) -> Result<Vec<Row>> {
        let rendered = self.render(ex.sql_dialect())?;
        ex.query_sql(rendered.sql, rendered.params).await
    }

    /// 执行查询，返回首行。
    ///
    /// 未显式设置 `limit` 时自动追加 `LIMIT 1`，避免取全量结果。
    pub async fn first<E: SqlExecutor>(mut self, ex: &E) -> Result<Option<Row>> {
        if self.limit.is_none() {
            self.limit = Some(1);
        }
        let rendered = self.render(ex.sql_dialect())?;
        ex.query_first_sql(rendered.sql, rendered.params).await
    }
}

impl Default for FindBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// 分页 Builder。
///
/// ## 安全性
///
/// 同 [`FindBuilder`]：值参数化、标识符白名单校验。
pub struct PageBuilder {
    /// 页码（从 1 开始）。keyset 模式下不使用。
    page: u64,
    /// 每页大小。
    size: u64,
    /// 表名。
    table: Option<String>,
    /// WHERE 条件。
    filter: Option<Expr>,
    /// ORDER BY 列表。
    order_by: Vec<OrderColumn>,
    /// Keyset 游标条件。设置后 `exec` 走 keyset 分页，替代 OFFSET 分页。
    keyset: Option<KeysetClause>,
    /// 租户作用域。由模型级入口（`scope_for::<T>()`）注入，渲染时拼入 WHERE 最外层。
    scope: ScopeMode,
}

impl PageBuilder {
    /// 创建分页 Builder。
    ///
    /// # 参数
    ///
    /// - `page`：页码，从 1 开始；keyset 模式下忽略
    /// - `size`：每页大小，至少 1
    pub fn new(page: u64, size: u64) -> Self {
        PageBuilder {
            page,
            size,
            table: None,
            filter: None,
            order_by: Vec::new(),
            keyset: None,
            scope: ScopeMode::Unset,
        }
    }

    /// 设置租户作用域。
    ///
    /// 由模型级入口注入；手写分页操作租户表时需自行调用，否则该语句不隔离。
    pub fn scope(mut self, scope: ScopeMode) -> Self {
        self.scope = scope;
        self
    }

    /// 设置表名。
    pub fn table(mut self, name: impl Into<String>) -> Self {
        self.table = Some(name.into());
        self
    }

    /// 设置 WHERE 条件。
    pub fn filter(mut self, expr: Expr) -> Self {
        self.filter = Some(match self.filter.take() {
            Some(existing) => Expr::and_all([existing, expr]),
            None => expr,
        });
        self
    }

    /// 添加 ORDER BY 列。
    pub fn order_by(mut self, column: impl Into<String>, direction: Order) -> Self {
        self.order_by.push(OrderColumn {
            column: column.into(),
            direction,
        });
        self
    }

    /// 设置 Keyset 游标分页条件。
    ///
    /// 设置后 `exec` 放弃 OFFSET 分页，改为基于排序列的 keyset 分页（避免深翻页
    /// 性能退化）。由于 keyset 语义不预读总数，`exec` 返回的 `Page` 不计算
    /// `total` / `pages`（均为 0），下游按游标逐页取数。
    ///
    /// # 参数
    ///
    /// - `sort_col`：排序列名（静态字符串）
    /// - `pk_col`：主键列名（静态字符串）
    /// - `direction`：排序方向，决定比较符方向
    /// - `after`：上一页的游标；传 `None` 表示从第一页开始
    pub fn page_after(
        mut self,
        sort_col: &'static str,
        pk_col: &'static str,
        direction: Order,
        after: Option<Cursor>,
    ) -> Self {
        self.keyset = Some(KeysetClause {
            sort_col,
            pk_col,
            direction,
            cursor: after,
        });
        self
    }

    /// 渲染不含分页的基础查询（供分页包装）。
    pub(crate) fn render_base(&self, dialect: &dyn Dialect) -> Result<RenderedSql> {
        let table = self.table.as_deref().ok_or_else(|| {
            alun_db_core::error::DbError::InvalidValue("builder: table is required".into())
        })?;
        let mut ctx = RenderCtx::new(dialect);
        let mut sql = format!("SELECT * FROM {}", quote_identifier(dialect, table)?);
        let mut where_parts: Vec<String> = Vec::new();
        if let Some(filter) = &self.filter {
            where_parts.push(dialect.render_expr(filter, &mut ctx));
        }
        if let Some(k) = &self.keyset
            && let Some(part) = render_keyset_where(dialect, &mut ctx, k)?
        {
            where_parts.push(part);
        }
        // 租户作用域：置于所有条件之后，保证参数绑定顺序与占位符一致
        if let Some(part) =
            alun_db_sql::render::scope::render_scope_mode(&self.scope, true, &mut ctx)
        {
            where_parts.push(part);
        }
        if !where_parts.is_empty() {
            sql.push_str(&format!(" WHERE {}", where_parts.join(" AND ")));
        }
        if !self.order_by.is_empty() {
            let parts: Vec<String> = self
                .order_by
                .iter()
                .map(|o| {
                    let dir = match o.direction {
                        Order::Asc => "ASC",
                        Order::Desc => "DESC",
                    };
                    Ok(format!(
                        "{} {dir}",
                        quote_qualified_identifier(dialect, &o.column)?
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            sql.push_str(&format!(" ORDER BY {}", parts.join(", ")));
        }
        Ok(RenderedSql {
            sql,
            params: ctx.params,
        })
    }

    /// 执行分页查询。
    ///
    /// 调用 [`page_after`](Self::page_after) 后为 keyset 分页；否则为基于页码的
    /// OFFSET 分页（此时返回完整 `total` / `pages`）。
    pub async fn exec<E: SqlExecutor>(self, ex: &E) -> Result<Page<Row>> {
        if self.keyset.is_some() {
            return self.exec_keyset(ex).await;
        }
        let rendered = self.render_base(ex.sql_dialect())?;
        ex.paginate_sql(rendered.sql, rendered.params, self.page, self.size)
            .await
    }

    /// keyset 分页执行：追加 `LIMIT size`，不计算总数。
    ///
    /// # 返回
    ///
    /// `Page` 的 `list` 为本页数据；keyset 模式不预读总数，`total` / `pages`
    /// 均为 0，`page` 亦为 0——下游据 `list` 末行构造下一轮 `Cursor`。
    async fn exec_keyset<E: SqlExecutor>(self, ex: &E) -> Result<Page<Row>> {
        let mut rendered = self.render_base(ex.sql_dialect())?;
        let size = self.size.max(1);
        // LIMIT 内联为字面量（同 `FindBuilder::render`，避免参数化 LIMIT 的额外开销）
        ex.sql_dialect()
            .push_limit_offset(Some(size), None, &mut rendered.sql);
        let list = ex.query_sql(rendered.sql, rendered.params).await?;
        Ok(Page {
            list,
            total: 0,
            page: 0,
            size,
            pages: 0,
        })
    }
}

/// 计数 Builder。
///
/// ## 安全性
///
/// 同 [`FindBuilder`]：值参数化、标识符白名单校验。
pub struct CountBuilder {
    /// 表名。
    table: Option<String>,
    /// WHERE 条件。
    filter: Option<Expr>,
    /// 末位 WHERE 片段：同 [`FindBuilder::filter_fragment`]。
    trailing: Option<Expr>,
    /// 租户作用域。由模型级入口（`scope_for::<T>()`）注入，渲染时拼入 WHERE 最外层。
    scope: ScopeMode,
}

impl CountBuilder {
    /// 创建空 Builder。
    pub fn new() -> Self {
        CountBuilder {
            table: None,
            filter: None,
            trailing: None,
            scope: ScopeMode::Unset,
        }
    }

    /// 设置租户作用域。
    ///
    /// 由模型级入口注入；手写计数操作租户表时需自行调用，否则该语句不隔离。
    pub fn scope(mut self, scope: ScopeMode) -> Self {
        self.scope = scope;
        self
    }

    /// 设置表名。
    pub fn table(mut self, name: impl Into<String>) -> Self {
        self.table = Some(name.into());
        self
    }

    /// 设置 WHERE 条件。
    pub fn filter(mut self, expr: Expr) -> Self {
        self.filter = Some(match self.filter.take() {
            Some(existing) => Expr::and_all([existing, expr]),
            None => expr,
        });
        self
    }

    /// 设置「末位 WHERE 片段」（不参与 `AND (...)` 包裹）。
    pub fn filter_fragment(mut self, expr: Expr) -> Self {
        self.trailing = Some(expr);
        self
    }

    /// 渲染为参数化的 COUNT 查询。
    fn render(&self, dialect: &dyn Dialect) -> Result<RenderedSql> {
        let table = self.table.as_deref().ok_or_else(|| {
            alun_db_core::error::DbError::InvalidValue("builder: table is required".into())
        })?;
        let mut ctx = RenderCtx::new(dialect);
        let mut sql = format!("SELECT COUNT(*) FROM {}", quote_identifier(dialect, table)?);
        let mut where_parts: Vec<String> = Vec::new();
        if let Some(filter) = &self.filter {
            where_parts.push(dialect.render_expr(filter, &mut ctx));
        }
        // 租户作用域：置于普通条件之后、末位片段之前（后者可自带尾部子句）
        if let Some(part) =
            alun_db_sql::render::scope::render_scope_mode(&self.scope, true, &mut ctx)
        {
            where_parts.push(part);
        }
        if let Some(trailing) = &self.trailing {
            where_parts.push(dialect.render_expr(trailing, &mut ctx));
        }
        if !where_parts.is_empty() {
            sql.push_str(&format!(" WHERE {}", where_parts.join(" AND ")));
        }
        Ok(RenderedSql {
            sql,
            params: ctx.params,
        })
    }

    /// 执行计数。
    pub async fn exec<E: SqlExecutor>(self, ex: &E) -> Result<i64> {
        let rendered = self.render(ex.sql_dialect())?;
        ex.scalar_sql(rendered.sql, rendered.params).await
    }
}

impl Default for CountBuilder {
    fn default() -> Self {
        Self::new()
    }
}

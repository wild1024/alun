//! `Persist` trait 的具体实现。

use std::future::Future;

use alun_db_core::error::{DbError, Result};
use alun_db_core::expr::Expr;
use alun_db_core::field::FieldMeta;
use alun_db_core::row::Row;
use alun_db_core::statement::{ConflictAction, InsertStmt, OnConflict};
use alun_db_core::stmt::Value;
use alun_db_core::traits::dialect::{Dialect, RenderCtx, RenderedSql};
use alun_db_core::traits::model::Model;
use alun_db_sql::render::insert::render_insert;

use crate::query::SqlExecutor;
use alun_db_core::hook;

/// 为所有 `Model` 提供持久化方法。
///
/// 所有方法都会触发 `Model::hooks()` 的生命周期回调（洋葱模型），
/// 详见 [`alun_db_core::traits::hook::Hook`]。
///
/// 方法返回 `impl Future + Send`（RPITIT）而非 `async fn`：
/// 保证返回的 Future 可跨线程传递，适配 tokio::spawn / axum handler。
/// `self` 引用需要跨 await 持有，故要求 `Self: Send / Sync`。
/// 事务内同样可用：`save` / `update` 等接受 `&impl SqlExecutor`，
/// [`Db`](crate::db::Db) 与 [`Transaction`](crate::tx::Transaction) 都实现了它，
/// 因此无需在事务里退回 raw SQL：
///
/// ```ignore
/// db.transaction(|tx| async move {
///     let invoice = Invoice::find(&tx, id).await?;
///     invoice.status = Status::Paid;
///     Invoice::of(id).set(Invoice::status(), Status::Paid)
///         .update(&tx).await?;                         // Hook 只收「主键 + 变更列」
///     audit::record(&tx, &record).await?;               // 审计与业务同一事务
///     Ok(())
/// }).await?;
/// ```
///
/// 自增主键回填、Hook 触发与分块批量写入在事务内语义与事务外一致；
/// 批量写入不会另开嵌套事务（框架不支持嵌套，见缺口 G4）——它复用外层事务的原子性。
pub trait PersistExt: Model {
    /// 插入一行。
    ///
    /// 自增主键在值为占位（`Null` / `0`）时不参与 INSERT，由数据库生成后回填
    /// 到 `self`；显式赋值的主键（雪花 ID 等）照常写入。
    ///
    /// 触发 `before_insert` / `after_insert`；Hook 对行数据的修改
    /// （如时间戳填充）随主键回填同步写回 `self`。
    ///
    /// 需要「有则更新、无则插入」时用
    /// [`insert_or_update`](Self::insert_or_update)。
    fn insert(&mut self, ex: &impl SqlExecutor) -> impl Future<Output = Result<()>> + Send
    where
        Self: Send + Sync;

    /// 插入或更新：主键未赋值时 INSERT，否则按存在性决定 INSERT 还是 UPDATE。
    ///
    /// 判定规则（对齐 aifei 的 `insertOrUpdate`，但以存在性为准更安全）：
    ///
    /// - 自增主键为占位值（`Null` / 数值 `0`）→ 直接 INSERT，不额外查询；
    /// - 主键已有值（显式赋值的自增 ID、雪花 ID、UUID / 自然键）→
    ///   查一次存在性：不存在则 INSERT（带显式主键），存在则整行 UPDATE。
    ///
    /// 只按「主键是否为空」判断会把「首次带显式主键插入」误判为 UPDATE
    /// （影响 0 行 → `NotFound`）；代价是已存在场景多一次 `SELECT`。
    ///
    /// INSERT 路径调用 `before_insert` / `after_insert`，
    /// Hook 对行数据的修改（如时间戳填充）会随主键回填同步写回 `self`。
    fn insert_or_update(
        &mut self,
        ex: &impl SqlExecutor,
    ) -> impl Future<Output = Result<()>> + Send
    where
        Self: Send + Sync;

    /// 按主键更新全部字段。
    ///
    /// 调用 `before_update` / `after_update`。注意 Hook 对行数据的修改
    /// 只作用于发往数据库的数据，不写回 `&self`。
    fn update(&self, ex: &impl SqlExecutor) -> impl Future<Output = Result<()>> + Send
    where
        Self: Sync;

    /// 按主键删除。
    ///
    /// 调用 `before_delete` / `after_delete`；`before_delete` 返回 `Err`
    /// 则拦截本次删除。
    fn delete(&self, ex: &impl SqlExecutor) -> impl Future<Output = Result<u64>> + Send
    where
        Self: Sync;

    /// 批量插入多行（按方言参数上限自动分块的多行 `VALUES`）。
    ///
    /// 复用 [`alun_db_sql::render::insert::render_insert`] 渲染器生成一次提交多行
    /// 的 INSERT，性能优于逐行调用 [`insert`](Self::insert)。
    ///
    /// 要求各行**列集一致**（同一模型，天然一致）；各行列集不同的异构批量
    /// 改用 [`RowBatchExt::batch_insert`](crate::rowops::RowBatchExt::batch_insert)。
    ///
    /// # 分块与原子性
    ///
    /// 单条语句能容纳的绑定参数有上限（SQLite 999 / PostgreSQL 65535 /
    /// MySQL 65535 且受 `max_allowed_packet` 约束）；整批超过上限时自动按行
    /// 拆分多条语句，并把这些语句包在**一个事务**里提交：
    ///
    /// - 未超上限（单块）：不带事务，保持一次往返；
    /// - 超上限（多块）：整批要么全部写入、要么整体回滚，不存在写一半的状态。
    ///
    /// 无论分几块，本次调用写入的行数恒等于 `rows.len()`。
    ///
    /// # Hook
    ///
    /// **同步** Hook 基于单行语义设计（可改写行数据），批量插入**不触发**
    /// ——需要时间戳自动填充这类改写时请逐行调用 [`insert`](Self::insert)。
    ///
    /// **异步** Hook（`AsyncHook`，只观察不改写）**会按行派发**
    /// `after_insert`，并遵循与单行路径一致的事务边界：自动提交时立即执行，
    /// 事务内则排队到 `commit()` 成功后执行（`rollback()` 丢弃）。
    /// 这样"批量写入"不会成为审计 / 缓存失效的盲区。
    ///
    /// # 自增主键
    ///
    /// 自增主键由数据库统一生成，本方法不逐行回填主键到 `rows`（多行
    /// `RETURNING` 结果与输入行暂无可靠的位置映射键）。需回填主键请逐行
    /// `insert`。
    ///
    /// # 参数
    ///
    /// - `rows`：待批量插入的模型切片。
    /// - `ex`：执行器（`&Db` 或事务内的 `&Transaction`）。
    ///
    /// # 错误
    ///
    /// 模型缺少主键、字段缺失或数据库执行失败时返回 [`DbError`]。
    fn insert_many(rows: &[Self], ex: &impl SqlExecutor) -> impl Future<Output = Result<()>> + Send
    where
        Self: Send + Sync,
    {
        async move { insert_many_impl(rows, ex).await }
    }

    /// Upsert：插入一行，冲突时按 `update_fields` 更新已存在的行。
    ///
    /// 基于 AST 的 `ON CONFLICT ... DO UPDATE SET` 渲染，复用
    /// [`alun_db_sql::render::insert::render_insert`] 生成语句。
    /// 仅当方言具备 `on_conflict` 能力（PostgreSQL / SQLite）时可用；
    /// 不支持该能力的方言（如 MySQL）返回 [`DbError::UnsupportedFeature`]。
    ///
    /// # 自增主键回填
    ///
    /// 自增主键模型在方言支持 `RETURNING` 时，无论执行结果是 INSERT 还是
    /// UPDATE，都会把数据库生成/命中的真实主键回填到 `self`。
    ///
    /// # 参数
    ///
    /// - `ex`：执行器（`&Db` 或事务内的 `&Transaction`）。
    /// - `conflict_fields`：参与冲突判定的模型字段名（对应表上的唯一约束）。
    /// - `update_fields`：冲突时需更新的模型字段名，取值来自 `self` 的当前字段
    ///   （模型字段名，与 `#[column(...)]` 重命名无关）。定值更新用
    ///   [`ChangeSet`](crate::changeset::ChangeSet) 更直接。
    ///
    /// # 错误
    ///
    /// 方言不支持 upsert、`conflict_fields` / `update_fields` 为空、
    /// 字段缺失或数据库执行失败时返回 [`DbError`]。
    fn upsert(
        &mut self,
        ex: &impl SqlExecutor,
        conflict_fields: &[&'static str],
        update_fields: &[&'static str],
    ) -> impl Future<Output = Result<()>> + Send
    where
        Self: Send + Sync,
    {
        async move { upsert_impl(self, ex, conflict_fields, update_fields).await }
    }
}

impl<T: Model> PersistExt for T {
    async fn insert(&mut self, ex: &impl SqlExecutor) -> Result<()> {
        ensure_writable::<T>()?;
        insert_one(self, ex).await
    }

    async fn insert_or_update(&mut self, ex: &impl SqlExecutor) -> Result<()> {
        ensure_writable::<T>()?;

        // 自增主键未赋值（占位）→ 必然 INSERT，无需探询。
        // 主键已赋值（雪花 ID / 自然键 / 显式赋值的自增 ID）→ 查一次存在性来决定
        // INSERT 还是 UPDATE：只按「主键是否为空」判断会把「首次带显式主键插入」
        // 误判成 UPDATE（影响 0 行 → NotFound）。代价是已存在场景多一次 SELECT。
        let is_new = if auto_pk_unset(key_meta::<T>()?, &self.pk_value()) {
            true
        } else {
            let pk = self.pk_value();
            let exists_sql = format!(
                "SELECT 1 FROM {} WHERE {} = ?",
                T::TABLE,
                T::primary_key_column()
            );
            ex.query_first_sql(exists_sql, vec![pk]).await?.is_none()
        };

        if is_new {
            insert_one(self, ex).await
        } else {
            self.update(ex).await
        }
    }

    async fn update(&self, ex: &impl SqlExecutor) -> Result<()> {
        ensure_writable::<T>()?;
        let mut row = self.as_row()?;
        let pk = self.pk_value();

        // Hook：before_update 正序（如 TimestampHook 刷新 updated_at）
        hook::run_before_update(T::hooks(), &mut row)?;
        // 租户补齐 / 一致性校验：防止把行改到别的租户或把租户列写成占位值
        crate::tenant::fill_row_tenant::<T>(&mut row)?;

        // 列名取 f.column：重命名列（#[column(...)]）时字段名与列名不同
        let update_fields: Vec<_> = T::fields()
            .iter()
            .filter(|f| f.name != T::PRIMARY_KEY)
            .collect();
        let sets: Vec<String> = update_fields
            .iter()
            .map(|f| format!("{} = ?", f.column))
            .collect();
        let mut sql = format!(
            "UPDATE {} SET {} WHERE {} = ?",
            T::TABLE,
            sets.join(", "),
            T::primary_key_column()
        );

        let params: Vec<Value> = update_fields
            .iter()
            .map(|f| {
                row.get(f.name)
                    .cloned()
                    .ok_or_else(|| DbError::ColumnNotFound(f.name.to_string()))
            })
            .collect::<Result<Vec<Value>>>()?;
        let mut params = [params, vec![pk]].concat();

        // 手写 SQL 无法走 Builder 注入，租户谓词在此显式追加（防止跨租户更新）
        if let Some((predicate, value)) = crate::tenant::scoped_predicate::<T>(ex.sql_dialect())? {
            sql.push_str(&format!(" AND {predicate}"));
            params.push(value);
        }

        let affected = ex.exec_sql(sql, params).await?;
        if affected == 0 {
            return Err(DbError::NotFound);
        }

        // Hook：after_update 逆序，传入实际发往数据库的行数据
        hook::run_after_update(T::hooks(), &row)?;
        // 异步 Hook：提交边界感知（自动提交立即执行；事务内排队至 commit 后）
        crate::hook_async::dispatch_after_update::<T, _>(ex, &row).await;
        Ok(())
    }

    async fn delete(&self, ex: &impl SqlExecutor) -> Result<u64> {
        ensure_writable::<T>()?;
        let pk = self.pk_value();

        // Hook：before_delete 正序，返回 Err 则拦截删除
        hook::run_before_delete(T::hooks(), &pk)?;

        // 手写 SQL 无法走 Builder 注入，租户谓词在此显式追加（防止跨租户删除）
        let tenant = crate::tenant::scoped_predicate::<T>(ex.sql_dialect())?;

        // 软删除：改写为 UPDATE SET 标记列 = now WHERE pk，保留数据仅标记删除
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

        // Hook：after_delete 逆序
        hook::run_after_delete(T::hooks(), &pk)?;
        // 异步 Hook：提交边界感知
        crate::hook_async::dispatch_after_delete::<T, _>(ex, &pk).await;
        Ok(affected)
    }
}

/// 模型主键元数据。
fn key_meta<T: Model>() -> Result<&'static FieldMeta> {
    T::fields()
        .iter()
        .find(|f| f.is_key)
        .ok_or_else(|| DbError::InvalidValue("model has no key".into()))
}

/// 自增主键是否「未赋值」：占位值（`Null` / 数值 0）表示由数据库生成。
///
/// 非自增主键恒为 `false`——其值总是有效，必须参与 INSERT。
fn auto_pk_unset(pk_meta: &FieldMeta, value: &Value) -> bool {
    pk_meta.is_auto && value.is_unset_pk()
}

/// 插入一行的实现。见 [`PersistExt::insert`]。
async fn insert_one<T: Model>(model: &mut T, ex: &impl SqlExecutor) -> Result<()> {
    let pk_meta = key_meta::<T>()?;
    let mut row = model.as_row()?;
    // Hook：before_insert 正序（如 TimestampHook 填充时间戳）
    hook::run_before_insert(T::hooks(), &mut row)?;
    // 租户补齐放在 Hook 之后：Hook 若改写了租户列，在这里做一致性校验（防越权写入）
    crate::tenant::fill_row_tenant::<T>(&mut row)?;

    // 自增主键未赋值时排除该列（由数据库生成）；显式赋值的主键与自然键照常
    // 写入。列名取 f.column：重命名列（#[column(...)]）时字段名与列名不同。
    let pk_value = row.get(T::PRIMARY_KEY).cloned().unwrap_or(Value::Null);
    let pk_unset = auto_pk_unset(pk_meta, &pk_value);
    let insert_fields: Vec<_> = T::fields()
        .iter()
        .filter(|f| !(pk_unset && f.name == T::PRIMARY_KEY))
        .collect();
    // 统一经 render_insert 渲染（与 insert_many / upsert 同一套渲染逻辑），
    // 避免手拼占位符导致两套 SQL 生成路径不一致。
    let insert_cols: Vec<&'static str> = insert_fields.iter().map(|f| f.column).collect();
    let values: Vec<Expr> = insert_fields
        .iter()
        .map(|f| {
            let v = row
                .get(f.name)
                .ok_or_else(|| DbError::ColumnNotFound(f.name.to_string()))?;
            Ok(Expr::Value(v.clone()))
        })
        .collect::<Result<Vec<Expr>>>()?;

    if pk_unset {
        // 自增主键：支持 RETURNING 的方言直接取回生成主键，其余借
        // last_insert_id 回查
        let can_return = ex.sql_dialect().capability().returning_clause;
        let stmt = InsertStmt {
            table: T::TABLE,
            columns: insert_cols,
            values: vec![values],
            returning: if can_return {
                Some(vec![T::primary_key_column()])
            } else {
                None
            },
            on_conflict: None,
        };
        let mut ctx = RenderCtx::new(ex.sql_dialect());
        let sql = render_insert(&stmt, &mut ctx);
        let id = if can_return {
            let inserted = ex
                .query_first_sql(sql.clone(), ctx.params)
                .await?
                .ok_or_else(|| DbError::Query {
                    sql,
                    source: "INSERT ... RETURNING returned no row".into(),
                })?;
            // RETURNING 返回的键为列名，需用 primary_key_column 读取
            let pk_value = inserted
                .get(T::primary_key_column())
                .ok_or_else(|| DbError::ColumnNotFound(T::primary_key_column().to_string()))?;
            value_as_i64(pk_value)?
        } else {
            ex.insert_returning_id_sql(sql, ctx.params).await?
        };
        // 回填主键后从 row 重建，Hook 填充的值随之同步回 self
        let pk_value = match pk_meta.ty {
            alun_db_core::stmt::Type::I32 => Value::I32(id as i32),
            _ => Value::I64(id),
        };
        row.insert(T::PRIMARY_KEY, pk_value);
    } else {
        // 主键已含在 row 中（显式赋值的自增主键 / 自然键），无生成 id 可回填
        let stmt = InsertStmt {
            table: T::TABLE,
            columns: insert_cols,
            values: vec![values],
            returning: None,
            on_conflict: None,
        };
        let mut ctx = RenderCtx::new(ex.sql_dialect());
        let sql = render_insert(&stmt, &mut ctx);
        ex.exec_sql(sql, ctx.params).await?;
    }
    // Hook：after_insert 逆序，传入实际写入的完整行（含主键）
    hook::run_after_insert(T::hooks(), &row)?;
    // 异步 Hook：提交边界感知（自动提交立即执行；事务内排队至 commit 后）
    crate::hook_async::dispatch_after_insert::<T, _>(ex, &row).await;
    *model = T::from_row(row)?;
    Ok(())
}

/// 校验模型是否可写。只读模型（`#[view]` 视图）禁止任何写操作。
///
/// # 错误
///
/// 模型为只读时返回 [`DbError::Readonly`]。
pub(crate) fn ensure_writable<T: Model>() -> Result<()> {
    if T::READONLY {
        Err(DbError::Readonly(T::TABLE))
    } else {
        Ok(())
    }
}

/// 将模型字段名解析为数据库列名。
///
/// `#[column("...")]` 重命名列后字段名与列名不同，写语句的标识符一律使用列名。
///
/// # 参数
///
/// - `name`：模型字段名。
///
/// # 返回
///
/// 对应的数据库列名。
///
/// # 错误
///
/// 字段不存在时返回 [`DbError::ColumnNotFound`]。
fn resolve_column<T: Model>(name: &str) -> Result<&'static str> {
    T::fields()
        .iter()
        .find(|f| f.name == name)
        .map(|f| f.column)
        .ok_or_else(|| DbError::ColumnNotFound(name.to_string()))
}

/// 从 `Value` 提取 `i64`（整数家族的统一读取，含 u64 checked 转换）。
///
/// # 错误
///
/// 值不是整数或 `u64` 超出 `i64` 范围时返回 [`DbError::InvalidValue`]。
fn value_as_i64(value: &Value) -> Result<i64> {
    match value {
        Value::I8(v) => Ok(i64::from(*v)),
        Value::I16(v) => Ok(i64::from(*v)),
        Value::I32(v) => Ok(i64::from(*v)),
        Value::I64(v) => Ok(*v),
        Value::U8(v) => Ok(i64::from(*v)),
        Value::U16(v) => Ok(i64::from(*v)),
        Value::U32(v) => Ok(i64::from(*v)),
        Value::U64(v) => i64::try_from(*v)
            .map_err(|_| DbError::InvalidValue(format!("u64 value {v} exceeds i64"))),
        other => Err(DbError::InvalidValue(format!(
            "expected integer value, got {:?}",
            other.ty()
        ))),
    }
}

/// 单条 `INSERT` 允许的最大绑定参数个数（占位符上限）。
///
/// - SQLite：`SQLITE_MAX_VARIABLE_NUMBER` 在 3.32 之前为 999、之后为 32766；
///   取 999 兼容所有版本（分块只增加语句条数，不改变总吞吐）。
/// - PostgreSQL：扩展查询协议（Bind）参数上限 65535。
/// - MySQL：预处理语句协议参数上限 65535。
const SQLITE_MAX_BIND_PARAMS: usize = 999;
const MAX_BIND_PARAMS: usize = 65_535;

/// MySQL 参数载荷的保守字节预算。
///
/// 除参数个数上限外，MySQL 还受 `max_allowed_packet`（默认 64 MiB，生产常调小）
/// 约束整包字节数。按 4 MiB 保守估算，避免大 TEXT / BLOB 批量插入撑爆数据包。
const MYSQL_PARAM_BUDGET_BYTES: usize = 4 * 1024 * 1024;

/// 按方言取单条语句的绑定参数上限。
///
/// 限额是驱动的协议约束、与 SQL 渲染能力无关，故按方言名判定：为纯限额扩大
/// `DialectCapability` 会波及每个驱动实现，得不偿失。
pub(crate) fn max_bind_params(dialect: &dyn Dialect) -> usize {
    if dialect.name() == "sqlite" {
        SQLITE_MAX_BIND_PARAMS
    } else {
        MAX_BIND_PARAMS
    }
}

/// 计算单条 `INSERT` 可容纳的行数（至少 1 行）。
///
/// 双重约束：占位符个数上限（所有方言）与参数载荷字节预算（MySQL 的
/// `max_allowed_packet`）。单行自身超预算时不再拆——交由驱动如实报错。
fn rows_per_statement(dialect: &dyn Dialect, columns: usize, rows: &[Vec<Value>]) -> usize {
    let by_params = (max_bind_params(dialect) / columns.max(1)).max(1);
    if dialect.name() != "mysql" {
        return by_params;
    }

    let mut budget = MYSQL_PARAM_BUDGET_BYTES;
    let mut fitted = 0_usize;
    for row in rows {
        let bytes: usize = row.iter().map(value_param_bytes).sum();
        if bytes > budget && fitted > 0 {
            break;
        }
        budget = budget.saturating_sub(bytes);
        fitted += 1;
    }
    fitted.max(1).min(by_params)
}

/// 估算单个值在参数报文中的字节数（保守上界，用于载荷预算）。
fn value_param_bytes(value: &Value) -> usize {
    match value {
        Value::Text(text) => text.len(),
        Value::Bytes(bytes) => bytes.len(),
        Value::Null => 1,
        _ => 16,
    }
}

/// 直接渲染一段多行 `INSERT`（无 RETURNING / ON CONFLICT）。
///
/// 不构造 `Expr` 树、也不复制整批值：逐单元格把**已拥有**的 `Value` 交给
/// [`RenderCtx::bind`] 换取占位符（`?` 或 `$n`），整批只遍历一次、零深拷贝。
/// 标识符引用与占位符规则仍走方言实现，与 `render_insert` 完全一致。
fn render_values_batch(
    table: &'static str,
    columns: &[&'static str],
    values: Vec<Vec<Value>>,
    dialect: &'static dyn Dialect,
) -> RenderedSql {
    let mut ctx = RenderCtx::new(dialect);
    let table_q = ctx.dialect.quote_identifier(table);
    let columns_q = columns
        .iter()
        .map(|c| ctx.dialect.quote_identifier(c))
        .collect::<Vec<_>>()
        .join(", ");

    // 容量预估：每行 `(?, ?, …), `；占位符最长按 `$65535` 计
    let per_row = columns.len() * 8 + 4;
    let mut sql =
        String::with_capacity(table_q.len() + columns_q.len() + per_row * values.len() + 32);
    sql.push_str("INSERT INTO ");
    sql.push_str(&table_q);
    sql.push_str(" (");
    sql.push_str(&columns_q);
    sql.push_str(") VALUES ");
    for (row_index, row) in values.into_iter().enumerate() {
        if row_index > 0 {
            sql.push_str(", ");
        }
        sql.push('(');
        for (column_index, value) in row.into_iter().enumerate() {
            if column_index > 0 {
                sql.push_str(", ");
            }
            sql.push_str(&ctx.bind(value));
        }
        sql.push(')');
    }

    RenderedSql {
        sql,
        params: ctx.params,
    }
}

/// 批量插入的实现。见 [`PersistExt::insert_many`]。
async fn insert_many_impl<T: Model>(rows: &[T], ex: &impl SqlExecutor) -> Result<()> {
    ensure_writable::<T>()?;
    // 空批直接成功，避免构造无行的 INSERT
    if rows.is_empty() {
        return Ok(());
    }

    // 主键元数据，用于判断是否自增：自增主键由数据库生成，插入时排除该列
    let pk_meta = T::fields()
        .iter()
        .find(|f| f.is_key)
        .ok_or_else(|| DbError::InvalidValue("model has no key".into()))?;
    // 自增主键：仅当所有行都未赋值（占位）时排除该列，由数据库统一生成；
    // 任一行显式赋值时保留该列，整批按显式主键写入。
    let pk_unset = rows.iter().all(|r| auto_pk_unset(pk_meta, &r.pk_value()));

    // 待插入列的 (DB 列名, 在 `fields()` 中的下标)
    let insert_cols: Vec<(&'static str, usize)> = T::fields()
        .iter()
        .enumerate()
        .filter(|(_, f)| !(pk_unset && f.name == T::PRIMARY_KEY))
        .map(|(index, f)| (f.column, index))
        .collect();
    if insert_cols.is_empty() {
        return Err(DbError::InvalidValue(
            "model has no insertable columns".into(),
        ));
    }

    // 直接构造多行 VALUES：按 `fields()` 顺序取出字段值（复用同一 scratch 缓冲，
    // 不构造 `Row`、不分配字段名），再原位 move 出插入列对应的值——整批零深拷贝。
    let field_count = T::fields().len();
    let mut scratch: Vec<Value> = Vec::with_capacity(field_count);
    let mut values: Vec<Vec<Value>> = Vec::with_capacity(rows.len());
    for row in rows {
        row.field_values_into(&mut scratch)?;
        if scratch.len() != field_count {
            return Err(DbError::InvalidValue(format!(
                "model '{}' produced {} field values, expected {} (fields()/field_values_into mismatch)",
                T::TABLE,
                scratch.len(),
                field_count
            )));
        }
        let mut row_values = Vec::with_capacity(insert_cols.len());
        for &(_, index) in &insert_cols {
            // 原位取出（换成 Null），避免 `Value` 的深拷贝
            row_values.push(std::mem::replace(&mut scratch[index], Value::Null));
        }
        values.push(row_values);
    }

    let dialect = ex.sql_dialect();
    let mut columns: Vec<&'static str> = insert_cols.iter().map(|(c, _)| *c).collect();

    // 租户补齐：`insert_many` 不触发 Hook，必须在渲染前显式补齐（设计约束 C1）。
    // 租户列通常在插入列中，逐行校验 / 补齐；若因边界情况不在（如与主键同名），
    // 追加该列并在每行补占位值，交由 `resolve_tenant_value` 统一补齐。
    let mut tenant_pos = T::TENANT.and_then(|col| columns.iter().position(|c| *c == col));
    if let Some(col) = T::TENANT
        && tenant_pos.is_none()
    {
        columns.push(col);
        tenant_pos = Some(columns.len() - 1);
        for row_values in values.iter_mut() {
            row_values.push(Value::Null);
        }
    }
    if let Some(pos) = tenant_pos {
        // 模型需隔离但无上下文时首行即报错（fail-closed），绝不写入未隔离数据
        for row_values in values.iter_mut() {
            if let Some(filled) = crate::tenant::resolve_tenant_value::<T>(&row_values[pos])? {
                row_values[pos] = filled;
            }
        }
    }

    // 异步 Hook 的行快照：**只有模型声明了 AsyncHook 才构造**（批量可达上万行，
    // 无 Hook 时不该为每行多分配一个 `Row`）。值取的是**真正写入**的内容
    // （含上面补齐的租户值）；自增主键仍是占位值——批量路径不回填主键。
    let hook_rows = if T::async_hooks().is_empty() {
        Vec::new()
    } else {
        let field_names: Vec<&'static str> = columns
            .iter()
            .map(|c| {
                T::fields()
                    .iter()
                    .find(|f| f.column == *c)
                    .map_or(*c, |f| f.name)
            })
            .collect();
        values
            .iter()
            .map(|vals| {
                let mut row = Row::new();
                for (name, v) in field_names.iter().zip(vals.iter()) {
                    row.insert(name, v.clone());
                }
                row
            })
            .collect()
    };

    let chunk = rows_per_statement(dialect, columns.len(), &values);

    // 单条语句即可容纳整批：保持一次往返，不额外开启事务
    if values.len() <= chunk {
        let rendered = render_values_batch(T::TABLE, &columns, values, dialect);
        ex.exec_sql(rendered.sql, rendered.params).await?;
        dispatch_batch_after_insert::<T, _>(ex, &hook_rows).await;
        return Ok(());
    }

    // 超过方言占位符上限：按块拆分，整批包在一个事务内保证原子性
    // （「一次调用 = 全部行生效或全部不生效」）
    tracing::debug!(
        table = T::TABLE,
        rows = values.len(),
        chunk,
        "insert_many 超过方言参数上限，已分块"
    );
    let mut remaining: std::collections::VecDeque<Vec<Value>> = values.into();
    ex.in_tx(|ex| {
        Box::pin(async move {
            while !remaining.is_empty() {
                let take = chunk.min(remaining.len());
                let batch: Vec<Vec<Value>> = remaining.drain(..take).collect();
                let rendered = render_values_batch(T::TABLE, &columns, batch, dialect);
                ex.exec_sql(rendered.sql, rendered.params).await?;
            }
            // 派发放在事务**内**的句柄上：异步 Hook 会排队到 commit 之后，
            // 因此"写一半失败回滚"时不会留下已执行的副作用。
            dispatch_batch_after_insert::<T, _>(ex, &hook_rows).await;
            Ok(())
        })
    })
    .await
}

/// 批量路径的异步 Hook 派发：逐行触发 `after_insert`。
///
/// 与单行路径共用同一个派发函数，因此事务边界语义完全一致
/// （自动提交 → 立即执行；事务内 → 提交成功后执行；回滚 → 丢弃）。
async fn dispatch_batch_after_insert<T: Model, E: SqlExecutor>(
    ex: &E,
    rows: &[alun_db_core::row::Row],
) {
    for row in rows {
        crate::hook_async::dispatch_after_insert::<T, _>(ex, row).await;
    }
}

/// Upsert 的实现。见 [`PersistExt::upsert`]。
async fn upsert_impl<T: Model>(
    model: &mut T,
    ex: &impl SqlExecutor,
    conflict_fields: &[&'static str],
    update_fields: &[&'static str],
) -> Result<()> {
    ensure_writable::<T>()?;
    // 方言能力判定：仅 PostgreSQL / SQLite（on_conflict=true）支持 ON CONFLICT
    if !ex.sql_dialect().capability().on_conflict {
        return Err(DbError::UnsupportedFeature {
            feature: "upsert",
            dialect: ex.sql_dialect().name().to_string(),
        });
    }
    if conflict_fields.is_empty() {
        return Err(DbError::InvalidValue(
            "conflict_fields must not be empty".into(),
        ));
    }
    if update_fields.is_empty() {
        return Err(DbError::InvalidValue(
            "update_fields must not be empty".into(),
        ));
    }

    let pk_meta = T::fields()
        .iter()
        .find(|f| f.is_key)
        .ok_or_else(|| DbError::InvalidValue("model has no key".into()))?;
    // 自增主键未赋值时才排除该列（由数据库生成）；显式赋值的主键参与 INSERT
    let pk_unset = auto_pk_unset(pk_meta, &model.pk_value());

    // 待插入的列（DB 列名, 行内字段名）
    let insert_cols: Vec<(&'static str, &'static str)> = T::fields()
        .iter()
        .filter(|f| !(pk_unset && f.name == T::PRIMARY_KEY))
        .map(|f| (f.column, f.name))
        .collect();
    if insert_cols.is_empty() {
        return Err(DbError::InvalidValue(
            "model has no insertable columns".into(),
        ));
    }

    let mut row = model.as_row()?;
    // 租户补齐 / 一致性校验（同 INSERT：无上下文则 fail-closed，值冲突则报错）
    crate::tenant::fill_row_tenant::<T>(&mut row)?;

    // 待插入的单行值
    let values = insert_cols
        .iter()
        .map(|(_, name)| {
            let v = row
                .get(name)
                .ok_or_else(|| DbError::ColumnNotFound((*name).into()))?;
            Ok(Expr::Value(v.clone()))
        })
        .collect::<Result<Vec<Expr>>>()?;

    // ON CONFLICT 的 DO UPDATE SET：update_fields 为字段名，
    // 映射为 DB 列名作 SET 目标，取值仍按行内字段名
    let sets = update_fields
        .iter()
        .map(|name| {
            let column = resolve_column::<T>(name)?;
            let v = row
                .get(name)
                .ok_or_else(|| DbError::ColumnNotFound((*name).into()))?;
            Ok((column, Expr::Value(v.clone())))
        })
        .collect::<Result<Vec<(&'static str, Expr)>>>()?;

    // conflict_fields 为字段名，映射为 DB 列名（对应表上的唯一约束）
    let target = conflict_fields
        .iter()
        .map(|name| resolve_column::<T>(name))
        .collect::<Result<Vec<&'static str>>>()?;

    // 自增主键：INSERT 路径的 id 由数据库生成，需依赖方言 RETURNING 能力
    // 回填到 model（PostgreSQL / SQLite 均支持）。无论 ON CONFLICT 实际走
    // INSERT 还是 UPDATE，RETURNING 主键都返回后续操作对应的真实主键：
    // INSERT 返回新生成 id，UPDATE 返回已存在行的 id。
    let can_return = pk_unset && ex.sql_dialect().capability().returning_clause;

    let stmt = InsertStmt {
        table: T::TABLE,
        columns: insert_cols.iter().map(|(c, _)| *c).collect(),
        values: vec![values],
        returning: if can_return {
            Some(vec![T::primary_key_column()])
        } else {
            None
        },
        on_conflict: Some(OnConflict {
            target,
            action: ConflictAction::DoUpdate(sets),
        }),
    };

    let mut ctx = RenderCtx::new(ex.sql_dialect());
    let sql = render_insert(&stmt, &mut ctx);

    if can_return {
        // 读取 RETURNING 返回的主键并回填到 model（写法与 save 的 INSERT 路径一致）
        let returned = ex
            .query_first_sql(sql.clone(), ctx.params)
            .await?
            .ok_or_else(|| DbError::Query {
                sql: sql.clone(),
                source: "INSERT ... ON CONFLICT ... RETURNING returned no row".into(),
            })?;
        let pk_value = returned
            .get(T::primary_key_column())
            .ok_or_else(|| DbError::ColumnNotFound(T::primary_key_column().to_string()))?;
        let id = value_as_i64(pk_value)?;
        let pk_value = match pk_meta.ty {
            alun_db_core::stmt::Type::I32 => Value::I32(id as i32),
            _ => Value::I64(id),
        };
        // 回填主键后从 row 重建，使模型与数据库实际主键一致
        let mut row = model.as_row()?;
        row.insert(T::PRIMARY_KEY, pk_value);
        // 同步回填租户列，避免模型停留在占位值（与 INSERT 路径一致）
        crate::tenant::fill_row_tenant::<T>(&mut row)?;
        *model = T::from_row(row)?;
    } else {
        ex.exec_sql(sql, ctx.params).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alun_db_sql::dialects::{
        mysql::MysqlDialect, postgres::PostgresDialect, sqlite::SqliteDialect,
    };

    /// 构造 `count` 行、每行 `columns` 列的值矩阵。
    fn matrix(count: usize, columns: usize) -> Vec<Vec<Value>> {
        vec![vec![Value::I64(1); columns]; count]
    }

    /// SQLite 按 999 个占位符分块（兼容 3.32 之前的 `SQLITE_MAX_VARIABLE_NUMBER`）。
    #[test]
    fn sqlite_chunks_by_placeholder_limit() {
        assert_eq!(
            rows_per_statement(&SqliteDialect, 10, &matrix(5_000, 10)),
            99
        );
        // 单行本身超上限：不再拆分，交由驱动如实报错
        assert_eq!(
            rows_per_statement(&SqliteDialect, 1_500, &matrix(2, 1_500)),
            1
        );
    }

    /// PostgreSQL 取扩展查询协议的参数上限 65535。
    #[test]
    fn postgres_chunks_by_protocol_limit() {
        assert_eq!(
            rows_per_statement(&PostgresDialect, 10, &matrix(7_000, 10)),
            6_553
        );
    }

    /// MySQL 除参数个数上限外，还按参数载荷预算提前分块（`max_allowed_packet`）。
    #[test]
    fn mysql_chunks_by_parameter_byte_budget() {
        // 每行 512 KiB 文本 → 4 MiB 预算内每块 8 行
        let big = vec![vec![Value::Text("x".repeat(512 * 1024))]; 100];
        assert_eq!(rows_per_statement(&MysqlDialect, 1, &big), 8);
        // 单行超预算：至少 1 行
        let huge = vec![vec![Value::Text("y".repeat(5 * 1024 * 1024))]; 3];
        assert_eq!(rows_per_statement(&MysqlDialect, 1, &huge), 1);
    }
}

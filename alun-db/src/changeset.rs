//! `ChangeSet<T>` —— 变更集（只带主键的引用对象）。
//!
//! 对齐 aifei 的 `Model.of(id).set(...).update()`：不先 `SELECT` 就能更新
//! 指定列，且**只更新被 `set` 过的列**（未设置的列保持数据库原值，不会像整行
//! `update` 那样用内存中的旧值覆盖）。
//!
//! ```ignore
//! User::of(123)
//!     .set(User::name(), "james")      // 键是 FieldExpr → 编译期校验字段与值类型
//!     .set(User::status(), Status::Paid)
//!     .update(&db).await?;
//!
//! User::of(123).delete(&db).await?;    // 免 SELECT 删除（软删模型自动改写）
//! ```
//!
//! 变更集交给 Hook 的行**只含主键与本次变更列**（`Row::keys()` 即变更集合），
//! 审计 Hook 无需自行 `SELECT` + diff。

use std::marker::PhantomData;

use alun_db_core::error::{DbError, Result};
use alun_db_core::field::{FieldExpr, FieldValue};
use alun_db_core::row::Row;
use alun_db_core::stmt::Value;
use alun_db_core::traits::{Model, TypeMapper};

use crate::persist::ensure_writable;
use crate::query::SqlExecutor;
use alun_db_core::hook;

/// 一条变更项。
struct Changed {
    /// 列所属表名（校验字段确实属于本模型）。
    table: &'static str,
    /// 数据库列名。
    column: &'static str,
    /// 新值。
    value: Value,
}

/// 变更集：以主键定位、以列变更描述的一次写操作。
///
/// 由 [`CrudExt::of`](crate::crud::CrudExt::of) 构造。
pub struct ChangeSet<T: Model> {
    /// 主键值。
    pk: Value,
    /// 变更项（按 `set` 调用顺序，同列去重后保留最后一次）。
    changed: Vec<Changed>,
    /// 标记 `T`，不占用运行时空间。
    _marker: PhantomData<fn() -> T>,
}

impl<T: Model> ChangeSet<T> {
    /// 构造变更集。由 `CrudExt::of` 调用。
    pub(crate) fn new(pk: Value) -> Self {
        ChangeSet {
            pk,
            changed: Vec::new(),
            _marker: PhantomData,
        }
    }

    /// 设置一列的新值。
    ///
    /// 键是字段表达式（[`FieldExpr`]），值类型由 `FieldValue` 在编译期约束：
    ///
    /// ```ignore
    /// User::of(1).set(User::age(), 18);          // ✅
    /// User::of(1).set(User::age(), "18");        // ❌ 编译失败
    /// ```
    ///
    /// 同一列多次 `set` 时保留最后一次。
    pub fn set<F, V>(mut self, field: FieldExpr<F>, value: V) -> Self
    where
        F: TypeMapper,
        V: FieldValue<F>,
    {
        let column = field.column();
        let value = value.into_value();
        if let Some(slot) = self.changed.iter_mut().find(|c| c.column == column) {
            slot.value = value;
        } else {
            self.changed.push(Changed {
                table: field.table(),
                column,
                value,
            });
        }
        self
    }

    /// 校验变更项并把它们映射为「(列名, 字段名)」，同时构造交给 Hook 的行。
    fn prepare(&self) -> Result<(Vec<(&'static str, &'static str)>, Row)> {
        if self.changed.is_empty() {
            return Err(DbError::InvalidValue(
                "ChangeSet has no fields set; call `set` before `update`".into(),
            ));
        }
        let mut row = Row::with_capacity(self.changed.len() + 1);
        row.insert(T::PRIMARY_KEY, self.pk.clone());
        let mut targets = Vec::with_capacity(self.changed.len());
        for c in &self.changed {
            if c.table != T::TABLE {
                return Err(DbError::InvalidValue(format!(
                    "ChangeSet::set field belongs to table `{}`, not `{}`",
                    c.table,
                    T::TABLE
                )));
            }
            let field = T::fields()
                .iter()
                .find(|f| f.column == c.column)
                .ok_or_else(|| DbError::ColumnNotFound(c.column.to_string()))?;
            // 主键由 `of` 定位，不参与 SET：改主键请用 `update` 整行写或先删后插
            if field.is_key {
                return Err(DbError::InvalidValue(format!(
                    "ChangeSet cannot set the primary key `{}`",
                    field.name
                )));
            }
            row.insert(field.name, c.value.clone());
            targets.push((c.column, field.name));
        }
        Ok((targets, row))
    }

    /// 执行部分更新，返回受影响行数。
    ///
    /// SET 的目标只含被 `set` 过的列；主键不在 SET 中。触发
    /// `before_update` / `after_update`（Hook 可见「主键 + 变更列」，可改写
    /// 将要写入的值）。
    pub async fn update<E: SqlExecutor>(self, ex: &E) -> Result<u64> {
        ensure_writable::<T>()?;
        let (targets, mut row) = self.prepare()?;

        hook::run_before_update(T::hooks(), &mut row)?;
        // 租户补齐 / 一致性校验：防止把行改到别的租户
        crate::tenant::fill_row_tenant::<T>(&mut row)?;

        let sets: Vec<String> = targets
            .iter()
            .map(|(column, _)| format!("{column} = ?"))
            .collect();
        let mut params = Vec::with_capacity(targets.len() + 1);
        for (_, name) in &targets {
            params.push(
                row.get(name)
                    .cloned()
                    .ok_or_else(|| DbError::ColumnNotFound((*name).to_string()))?,
            );
        }
        params.push(self.pk.clone());

        let mut sql = format!(
            "UPDATE {} SET {} WHERE {} = ?",
            T::TABLE,
            sets.join(", "),
            T::primary_key_column()
        );
        // 手写 SQL 无法走 Builder 注入，租户谓词在此显式追加（防止跨租户更新）
        if let Some((predicate, value)) = crate::tenant::scoped_predicate::<T>(ex.sql_dialect())? {
            sql.push_str(&format!(" AND {predicate}"));
            params.push(value);
        }
        let affected = ex.exec_sql(sql, params).await?;

        hook::run_after_update(T::hooks(), &row)?;
        // 异步 Hook：提交边界感知（自动提交立即执行；事务内排队至 commit 后）
        crate::hook_async::dispatch_after_update::<T, _>(ex, &row).await;
        Ok(affected)
    }

    /// 按主键删除，返回受影响行数。
    ///
    /// 与 [`CrudExt::delete_by_pk`](crate::crud::CrudExt::delete_by_pk) 同语义：软删模型
    /// 改写为 `UPDATE SET <标记列> = now`，并触发 `before_delete` /
    /// `after_delete`。
    pub async fn delete<E: SqlExecutor>(self, ex: &E) -> Result<u64> {
        crate::crud::delete_by_pk_impl::<T, E>(ex, self.pk).await
    }
}

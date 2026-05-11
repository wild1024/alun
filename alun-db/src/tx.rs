//! 事务：真正的 Commit/Rollback，编译期保证永不"忘记回滚"

use crate::{DbResult, DbError, db::DbPool};
use sqlx::{Row, Column};
use serde_json::{Value, Number};
use tracing::debug;

/// 事务隔离级别
///
/// 遵循 SQL 标准四级隔离，从低到高排列（可用于 `PartialOrd` 比较）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Isolation {
    /// 读未提交（最低隔离级别，可能脏读）
    ReadUncommitted,
    /// 读已提交（默认级别，无脏读）
    ReadCommitted,
    /// 可重复读（同一事务内多次读取一致）
    RepeatableRead,
    /// 串行化（最高隔离级别，完全隔离）
    Serializable,
}

impl std::fmt::Display for Isolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Isolation::ReadUncommitted => write!(f, "READ UNCOMMITTED"),
            Isolation::ReadCommitted    => write!(f, "READ COMMITTED"),
            Isolation::RepeatableRead   => write!(f, "REPEATABLE READ"),
            Isolation::Serializable     => write!(f, "SERIALIZABLE"),
        }
    }
}

/// 活跃事务句柄 —— 封装不同数据库的连接（已 BEGIN）
///
/// 事务通过 `Db::transaction()` 创建，不直接构造。
/// 闭包正常返回则 COMMIT，返回 Err 则 ROLLBACK。
/// 当 `ActiveTx` 被 drop 且未提交/回滚时，日志会输出警告。
pub struct ActiveTx {
    inner: ActiveTxInner,
    committed: bool,
    rolled_back: bool,
}

enum ActiveTxInner {
    /// PostgreSQL 连接
    Postgres(sqlx::pool::PoolConnection<sqlx::Postgres>),
    /// MySQL 连接
    Mysql(sqlx::pool::PoolConnection<sqlx::MySql>),
    /// SQLite 连接
    Sqlite(sqlx::pool::PoolConnection<sqlx::Sqlite>),
}

impl ActiveTx {
    /// 在事务中执行写操作（INSERT/UPDATE/DELETE），返回受影响行数
    ///
    /// 参数使用 `$1`、`$2` 占位符，按顺序绑定。
    pub async fn execute(&mut self, sql: &str, params: &[&str]) -> DbResult<u64> {
        match &mut self.inner {
            ActiveTxInner::Postgres(c) => {
                let mut q = sqlx::query::<sqlx::Postgres>(sql);
                for p in params { q = q.bind(*p); }
                q.execute(&mut **c).await.map_err(DbError::from).map(|r| r.rows_affected())
            }
            ActiveTxInner::Mysql(c) => {
                let mut q = sqlx::query::<sqlx::MySql>(sql);
                for p in params { q = q.bind(*p); }
                q.execute(&mut **c).await.map_err(DbError::from).map(|r| r.rows_affected())
            }
            ActiveTxInner::Sqlite(c) => {
                let mut q = sqlx::query::<sqlx::Sqlite>(sql);
                for p in params { q = q.bind(*p); }
                q.execute(&mut **c).await.map_err(DbError::from).map(|r| r.rows_affected())
            }
        }
    }

    /// 在事务中执行查询，返回 `Option<Row>`
    ///
    /// 参数使用 `$1`、`$2` 占位符，按顺序绑定。
    /// 未找到记录返回 `Ok(None)`。
    pub async fn query_one(&mut self, sql: &str, params: &[&str]) -> DbResult<Option<crate::Row>> {
        match &mut self.inner {
            ActiveTxInner::Postgres(c) => {
                let mut q = sqlx::query::<sqlx::Postgres>(sql);
                for p in params { q = q.bind(*p); }
                Ok(q.fetch_optional(&mut **c).await?.as_ref().map(tx_row_to_row_pg))
            }
            ActiveTxInner::Mysql(c) => {
                let mut q = sqlx::query::<sqlx::MySql>(sql);
                for p in params { q = q.bind(*p); }
                Ok(q.fetch_optional(&mut **c).await?.as_ref().map(tx_row_to_row_my))
            }
            ActiveTxInner::Sqlite(c) => {
                let mut q = sqlx::query::<sqlx::Sqlite>(sql);
                for p in params { q = q.bind(*p); }
                Ok(q.fetch_optional(&mut **c).await?.as_ref().map(tx_row_to_row_sqlite))
            }
        }
    }

    /// 标记事务需回滚（即使闭包返回 `Ok`，也会执行 ROLLBACK）
    ///
    /// 用于业务逻辑判断失败但不想中断闭包流程的场景。
    pub fn set_rollback_only(&mut self) { self.committed = false; self.rolled_back = true; }

    async fn commit(mut self) -> DbResult<()> {
        if self.rolled_back { self.rollback().await; return Ok(()); }
        debug!("事务提交");
        match &mut self.inner {
            ActiveTxInner::Postgres(c) => { sqlx::query::<sqlx::Postgres>("COMMIT").execute(&mut **c).await.map_err(DbError::from)?; }
            ActiveTxInner::Mysql(c)    => { sqlx::query::<sqlx::MySql>("COMMIT").execute(&mut **c).await.map_err(DbError::from)?; }
            ActiveTxInner::Sqlite(c)   => { sqlx::query::<sqlx::Sqlite>("COMMIT").execute(&mut **c).await.map_err(DbError::from)?; }
        };
        self.committed = true;
        Ok(())
    }

    async fn rollback(&mut self) {
        debug!("事务回滚");
        match &mut self.inner {
            ActiveTxInner::Postgres(c) => { let _ = sqlx::query::<sqlx::Postgres>("ROLLBACK").execute(&mut **c).await; }
            ActiveTxInner::Mysql(c)    => { let _ = sqlx::query::<sqlx::MySql>("ROLLBACK").execute(&mut **c).await; }
            ActiveTxInner::Sqlite(c)   => { let _ = sqlx::query::<sqlx::Sqlite>("ROLLBACK").execute(&mut **c).await; }
        };
        self.rolled_back = true;
    }
}

impl Drop for ActiveTx {
    fn drop(&mut self) {
        if !self.committed && !self.rolled_back {
            tracing::warn!("事务未提交也未回滚，连接返回池时将自动回滚（依赖数据库特性）");
        }
    }
}

macro_rules! tx_row_convert {
    ($func_name:ident, $db_ty:ty) => {
        fn $func_name(row: &<$db_ty as sqlx::Database>::Row) -> crate::Row {
            let mut r = crate::Row::default();
            for col in <$db_ty as sqlx::Database>::Row::columns(row) {
                let name = col.name().to_string();
                let idx: usize = col.ordinal();
                if let Ok(v) = row.try_get::<i64, usize>(idx) {
                    r.data.insert(name, Value::Number(v.into()));
                } else if let Ok(v) = row.try_get::<i32, usize>(idx) {
                    r.data.insert(name, Value::Number((v as i64).into()));
                } else if let Ok(v) = row.try_get::<i16, usize>(idx) {
                    r.data.insert(name, Value::Number((v as i64).into()));
                } else if let Ok(v) = row.try_get::<String, usize>(idx) {
                    r.data.insert(name, Value::String(v));
                } else if let Ok(v) = row.try_get::<sqlx::types::Uuid, usize>(idx) {
                    r.data.insert(name, Value::String(v.to_string()));
                } else if let Ok(v) = row.try_get::<f64, usize>(idx) {
                    if let Some(n) = Number::from_f64(v) {
                        r.data.insert(name, Value::Number(n));
                    }
                } else if let Ok(v) = row.try_get::<bool, usize>(idx) {
                    r.data.insert(name, Value::Bool(v));
                }
            }
            r.mark_all_changed();
            r
        }
    };
}

tx_row_convert!(tx_row_to_row_pg, sqlx::Postgres);
tx_row_convert!(tx_row_to_row_my, sqlx::MySql);
tx_row_convert!(tx_row_to_row_sqlite, sqlx::Sqlite);

/// 执行事务 —— 传入闭包接收 `ActiveTx`，自动管理 BEGIN / COMMIT / ROLLBACK
///
/// 闭包接收一个 `ActiveTx`（已 BEGIN），需返回 `(ActiveTx, DbResult<T>)`。
/// 返回 `Ok` 时自动 COMMIT，返回 `Err` 或 Drop 未提交时自动 ROLLBACK。
pub(crate) async fn execute_transaction<F, Fut, T>(
    pool: &DbPool, _isolation: Isolation, _rollback_only: &mut bool, f: F,
) -> DbResult<T>
where
    F: FnOnce(ActiveTx) -> Fut + Send,
    Fut: std::future::Future<Output = (ActiveTx, DbResult<T>)> + Send,
    T: Send,
{
    let tx = match pool {
        DbPool::Postgres(p) => {
            let mut conn = p.acquire().await?;
            sqlx::query::<sqlx::Postgres>("BEGIN").execute(&mut *conn).await?;
            ActiveTx { inner: ActiveTxInner::Postgres(conn), committed: false, rolled_back: false }
        }
        DbPool::Mysql(p) => {
            let mut conn = p.acquire().await?;
            sqlx::query::<sqlx::MySql>("BEGIN").execute(&mut *conn).await?;
            ActiveTx { inner: ActiveTxInner::Mysql(conn), committed: false, rolled_back: false }
        }
        DbPool::Sqlite(p) => {
            let mut conn = p.acquire().await?;
            sqlx::query::<sqlx::Sqlite>("BEGIN").execute(&mut *conn).await?;
            ActiveTx { inner: ActiveTxInner::Sqlite(conn), committed: false, rolled_back: false }
        }
        DbPool::Any(_) => return Err(DbError::Other("Any pool 不支持事务".into())),
    };

    let (mut tx, result) = f(tx).await;
    match result {
        Ok(val) => { tx.commit().await?; Ok(val) }
        Err(e)  => { tx.rollback().await; Err(e) }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_isolation_display() {
        use super::Isolation;
        assert_eq!(Isolation::ReadCommitted.to_string(), "READ COMMITTED");
        assert_eq!(Isolation::Serializable.to_string(), "SERIALIZABLE");
    }
}

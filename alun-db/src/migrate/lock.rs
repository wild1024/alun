//! 迁移互斥锁。
//!
//! 所有迁移入口全程持有迁移互斥锁：PostgreSQL 用 `pg_try_advisory_lock`、
//! MySQL 用 `GET_LOCK`（会话级，跨进程有效）；SQLite 退化为**进程内** `Mutex`
//! ——SQLite 没有咨询锁，跨进程并发迁移仍可能撞 `SQLITE_BUSY`
//! （映射为 [`DbError::LockTimeout`]），须由部署编排保证同一库同一时刻只有一个
//! 迁移进程。

use std::future::Future;
use std::time::{Duration, Instant};

use alun_db_core::error::{DbError, Result};
use alun_db_core::stmt::Value;

use crate::db::Db;
use crate::tx::Transaction;

/// 咨询锁的 64 位键（`"alunmigr"` 的 ASCII），用于 PostgreSQL 咨询锁。
const MIGRATION_LOCK_KEY: i64 = 0x616C_756E_6D69_6772;

/// 咨询锁名，用于 MySQL `GET_LOCK` / `RELEASE_LOCK`。
const MIGRATION_LOCK_NAME: &str = "alun:schema_migrations";

/// 等待迁移互斥锁的上限；超时返回 [`DbError::LockTimeout`]。
const MIGRATION_LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// 咨询锁轮询间隔（`pg_try_advisory_lock` 非阻塞，需轮询重试）。
const MIGRATION_LOCK_POLL: Duration = Duration::from_millis(50);

/// SQLite 迁移的进程内互斥。
static SQLITE_MIGRATION_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 迁移互斥锁的方言类别。
#[derive(Debug, Clone, Copy)]
enum LockKind {
    /// SQLite：进程内互斥。
    Local,
    /// PostgreSQL：会话级 `pg_advisory_lock`。
    Postgres,
    /// MySQL：会话级 `GET_LOCK`。
    Mysql,
}

/// 迁移互斥锁。
///
/// PG / MySQL 的咨询锁是**会话级**的：持有它需要一条专属连接
/// （[`Transaction`] 恰好提供连接独占 + RAII 归还），因此这两个方言要求连接池
/// 至少 2 条连接（一条持锁、其余执行迁移语句）；SQLite 只取进程内互斥。
pub(crate) struct MigrationLock {
    /// 方言类别（决定解锁语句）。
    kind: LockKind,
    /// 持有会话级咨询锁的专属连接。
    session: Option<Transaction>,
    /// 进程内互斥守卫。
    _local: Option<tokio::sync::MutexGuard<'static, ()>>,
}

impl MigrationLock {
    /// 获取迁移互斥锁。
    ///
    /// # 错误
    ///
    /// 建会话失败、等待咨询锁超时（[`DbError::LockTimeout`]）时返回错误。
    async fn acquire(db: &Db) -> Result<Self> {
        match db.dialect().name() {
            // MySQL：GET_LOCK 阻塞等待至多 `timeout` 秒；取得返回 1、超时 0、出错 NULL
            "mysql" => {
                let session = db.begin().await?;
                let acquired = session_scalar(
                    &session,
                    "SELECT GET_LOCK(?, ?)",
                    vec![
                        Value::Text(MIGRATION_LOCK_NAME.into()),
                        Value::I64(MIGRATION_LOCK_TIMEOUT.as_secs() as i64),
                    ],
                )
                .await?;
                if !lock_acquired(acquired.as_ref()) {
                    return Err(lock_timeout_error("mysql GET_LOCK"));
                }
                Ok(MigrationLock {
                    kind: LockKind::Mysql,
                    session: Some(session),
                    _local: None,
                })
            }
            // SQLite：无咨询锁，进程内串行
            "sqlite" => Ok(MigrationLock {
                kind: LockKind::Local,
                session: None,
                _local: Some(SQLITE_MIGRATION_MUTEX.lock().await),
            }),
            // PostgreSQL：阻塞版本 `pg_advisory_lock` 返回 `void`，驱动不解码 `void`
            // 列，故用布尔返回值的 `pg_try_advisory_lock` 轮询。
            _ => {
                let session = db.begin().await?;
                let deadline = Instant::now() + MIGRATION_LOCK_TIMEOUT;
                loop {
                    let acquired = session_scalar(
                        &session,
                        "SELECT pg_try_advisory_lock(?)",
                        vec![Value::I64(MIGRATION_LOCK_KEY)],
                    )
                    .await?;
                    if lock_acquired(acquired.as_ref()) {
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err(lock_timeout_error("postgres pg_try_advisory_lock"));
                    }
                    tokio::time::sleep(MIGRATION_LOCK_POLL).await;
                }
                Ok(MigrationLock {
                    kind: LockKind::Postgres,
                    session: Some(session),
                    _local: None,
                })
            }
        }
    }

    /// 释放咨询锁并归还专属连接。
    ///
    /// 解锁失败只告警不阻断：会话随着连接的 RAII 终结归还池，锁随会话结束释放。
    async fn release(self) -> Result<()> {
        let Some(session) = self.session else {
            return Ok(());
        };
        let unlock = match self.kind {
            LockKind::Postgres => "SELECT pg_advisory_unlock(?)",
            LockKind::Mysql => "SELECT RELEASE_LOCK(?)",
            LockKind::Local => return Ok(()),
        };
        let params = match self.kind {
            LockKind::Postgres => vec![Value::I64(MIGRATION_LOCK_KEY)],
            _ => vec![Value::Text(MIGRATION_LOCK_NAME.into())],
        };
        if let Err(e) = session.find_with_params(unlock, params).await {
            tracing::warn!("释放迁移咨询锁失败（连接归还后由会话结束兜底）: {e}");
        }
        session.rollback().await
    }
}

/// 在迁移互斥保护下执行 `body`，无论成败都释放锁。
pub(crate) async fn with_migration_lock<F, Fut, T>(db: &Db, body: F) -> Result<T>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let lock = MigrationLock::acquire(db).await?;
    let result = body().await;
    match lock.release().await {
        // 释放失败只在业务成功时上抛，避免覆盖真实错误
        Ok(()) => result,
        Err(e) if result.is_ok() => Err(e),
        Err(_) => result,
    }
}

/// 在会话连接上执行查询并取首行首列（`Transaction` 未提供 scalar 便捷方法）。
async fn session_scalar(
    session: &Transaction,
    sql: &str,
    params: Vec<Value>,
) -> Result<Option<Value>> {
    let rows = session.find_with_params(sql, params).await?;
    Ok(rows
        .into_iter()
        .next()
        .and_then(|row| row.values().next().cloned()))
}

/// 咨询锁函数的返回值是否表示「已取得锁」。
///
/// 驱动对 `1` 的承载可能是整数、文本或单字节（各驱动协议不同），统一判定。
fn lock_acquired(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(v)) => *v,
        Some(Value::I8(v)) => *v == 1,
        Some(Value::I16(v)) => *v == 1,
        Some(Value::I32(v)) => *v == 1,
        Some(Value::I64(v)) => *v == 1,
        Some(Value::U8(v)) => *v == 1,
        Some(Value::U16(v)) => *v == 1,
        Some(Value::U32(v)) => *v == 1,
        Some(Value::U64(v)) => *v == 1,
        Some(Value::Text(t)) => t.trim() == "1",
        Some(Value::Bytes(b)) => b.as_slice() == b"1",
        _ => false,
    }
}

/// 构造「等待迁移互斥锁超时」错误（可识别、可重试）。
fn lock_timeout_error(what: &str) -> DbError {
    DbError::LockTimeout {
        source: Box::new(std::io::Error::other(format!(
            "timed out after {}s waiting for the migration lock ({what})",
            MIGRATION_LOCK_TIMEOUT.as_secs()
        ))),
    }
}

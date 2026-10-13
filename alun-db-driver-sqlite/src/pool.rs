//! SQLite 连接池。
//!
//! SQLite 是嵌入式数据库，连接池的语义与其他数据库不同：
//!
//! - 内存数据库（`:memory:`）每个连接是**独立的数据库**，必须全程共享
//!   同一连接，否则数据互相不可见
//! - 文件数据库可以多连接，但 SQLite 的写锁是全局的
//!
//! 因此本实现按 URL 区分两种模式：
//!
//! - **共享模式**（内存数据库）：强制单连接 + 单许可，所有操作串行复用
//!   同一连接；事务独占期间其他操作等待
//! - **多连接模式**（文件数据库）：`Semaphore` 限制并发借出数不超过
//!   `max_connections`，空闲连接复用，不足时新建
//!
//! 池不引入 bb8 / deadpool 等外部依赖，符合 SQLite 的实际使用模式。

use alun_db_core::error::{DbError, Result};
use alun_db_core::traits::driver::PoolConfig;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::connection::SqliteConnection;
use crate::transaction::SqlitePoolTransaction;

/// SQLite 连接池。
pub struct SqlitePool {
    /// 池共享状态。
    inner: Arc<PoolInner>,
}

/// 池的共享状态。
struct PoolInner {
    /// 连接 URL。
    url: String,
    /// 池配置。
    config: PoolConfig,
    /// 空闲连接栈。归还的连接等待复用。
    idle: Mutex<Vec<SqliteConnection>>,
    /// 借出许可信号量。并发借出数不超过其容量。
    permits: Arc<Semaphore>,
    /// 是否为内存数据库单连接共享模式。
    is_shared_mode: bool,
}

impl SqlitePool {
    /// 创建连接池。
    ///
    /// 内存数据库强制单连接模式；文件数据库按配置预热最小连接数，
    /// 并发借出上限为 `max_connections`（至少 1）。
    pub fn new(url: &str, config: PoolConfig) -> Result<Self> {
        let is_shared_mode = is_memory_url(url);
        // 共享模式容量固定为 1；多连接模式至少允许 1 个并发
        let capacity = if is_shared_mode {
            1
        } else {
            config.max_connections.max(1) as usize
        };
        // 预热连接数不超过容量
        let warmup = if is_shared_mode {
            1
        } else {
            config.min_connections.min(config.max_connections) as usize
        };

        let inner = PoolInner {
            url: url.to_string(),
            idle: Mutex::new(Vec::new()),
            permits: Arc::new(Semaphore::new(capacity)),
            is_shared_mode,
            config,
        };
        // 预热连接（同步打开，池创建发生在异步上下文的准备阶段）
        {
            let mut idle = inner
                .idle
                .lock()
                .map_err(|e| DbError::Connection(format!("pool lock poisoned: {e}").into()))?;
            for _ in 0..warmup {
                idle.push(SqliteConnection::open(url)?);
            }
        }

        Ok(SqlitePool {
            inner: Arc::new(inner),
        })
    }

    /// 获取一个连接。
    ///
    /// 空闲连接优先复用；不足时打开新连接。并发借出数受
    /// `max_connections` 限制，等待超过 `connect_timeout` 返回错误。
    /// 连接在 `Drop` 时自动归还。
    pub async fn get(&self) -> Result<PooledSqliteConnection> {
        // 快路径：容量空闲时同步取许可。省掉 `tokio::time::timeout` 的构造
        // （含 `Sleep` 与定时器驱动的准备）——点查这类高频短操作上，容量几乎
        // 总是空闲的，这份构造纯属每次调用的固定开销。
        let permits = Arc::clone(&self.inner.permits);
        let permit = match Arc::clone(&permits).try_acquire_owned() {
            Ok(permit) => permit,
            Err(tokio::sync::TryAcquireError::Closed) => {
                return Err(DbError::Connection("pool: semaphore closed".into()));
            }
            Err(tokio::sync::TryAcquireError::NoPermits) => {
                // 慢路径：容量用尽，等待归还；等待超过 connect_timeout 视为获取失败
                let timeout = self.inner.config.connect_timeout;
                tokio::time::timeout(timeout, permits.acquire_owned())
                    .await
                    .map_err(|_| {
                        DbError::Connection(
                            format!("pool: acquire connection timed out after {timeout:?}").into(),
                        )
                    })?
                    .map_err(|_| DbError::Connection("pool: semaphore closed".into()))?
            }
        };

        // 先尝试弹出空闲连接；锁在块结束时释放，不跨越后续 await
        let popped = {
            let mut idle = self
                .inner
                .idle
                .lock()
                .map_err(|e| DbError::Connection(format!("pool lock poisoned: {e}").into()))?;
            idle.pop()
        };

        let conn = match popped {
            Some(conn) => conn,
            None => {
                if self.inner.is_shared_mode {
                    // 共享模式容量为 1，取得许可时空闲栈必然非空；
                    // 走到此处说明不变量被破坏，显式报错而非静默新建
                    return Err(DbError::Connection(
                        "shared pool invariant violated: connection missing".into(),
                    ));
                }
                // 打开文件连接涉及磁盘 IO，放入阻塞线程避免阻塞 reactor
                let url = self.inner.url.clone();
                tokio::task::spawn_blocking(move || SqliteConnection::open(&url))
                    .await
                    .map_err(|e| {
                        DbError::Connection(format!("pool: blocking task join failed: {e}").into())
                    })??
            }
        };

        Ok(PooledSqliteConnection {
            conn: Some(conn),
            pool: self.inner.clone(),
            permit: Some(permit),
        })
    }

    /// 开始事务。从事务池借出专属连接并执行 `BEGIN`。
    ///
    /// 事务期间连接被独占（共享模式下其他操作等待事务终结），
    /// 提交 / 回滚 / 句柄销毁后连接归还池。
    pub async fn begin_transaction(&self) -> Result<SqlitePoolTransaction> {
        let pooled = self.get().await?;
        // BEGIN 是 SQLite 本地操作，同步执行即可
        pooled.conn().begin_transaction()?;
        Ok(SqlitePoolTransaction::begun(pooled))
    }
}

/// 判断 URL 是否指向内存数据库。
fn is_memory_url(url: &str) -> bool {
    let path = url.strip_prefix("sqlite://").unwrap_or(url);
    path == ":memory:" || url == "sqlite::memory:"
}

/// 池化连接。`Drop` 时自动归还到池中。
pub struct PooledSqliteConnection {
    /// 池化连接本体。
    conn: Option<SqliteConnection>,
    /// 所属池的共享状态。
    pool: Arc<PoolInner>,
    /// 借出许可。`Drop` 时自动释放，归还并发容量。
    ///
    /// RAII 守卫字段：仅持有不读取，容量随字段 Drop 归还。
    #[allow(dead_code)]
    permit: Option<OwnedSemaphorePermit>,
}

impl PooledSqliteConnection {
    /// 访问内部连接。
    pub fn conn(&self) -> &SqliteConnection {
        match self.conn.as_ref() {
            Some(conn) => conn,
            // conn 字段仅在 Drop 中被取走；&self 存在时 Drop 尚未运行，
            // 借用检查保证此分支不可达
            None => unreachable!("pooled connection already returned to pool"),
        }
    }
}

impl Drop for PooledSqliteConnection {
    fn drop(&mut self) {
        // 归还连接；锁中毒时丢弃连接（容量随 permit 释放恢复）
        if let Some(conn) = self.conn.take() {
            match self.pool.idle.lock() {
                Ok(mut idle) => idle.push(conn),
                Err(_) => { /* 锁中毒：连接丢弃，不归还 */ }
            }
        }
        // permit 随字段 Drop 自动释放
    }
}

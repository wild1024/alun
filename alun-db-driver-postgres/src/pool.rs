//! PostgreSQL 连接池。
//!
//! 自研轻量池（与 SQLite 驱动同构，不引入 bb8 / deadpool）：
//!
//! - `Semaphore(max_connections)` 限制并发借出数
//! - `Mutex<Vec<Connection>>` 空闲栈复用归还的连接
//! - 借出前检查 `max_lifetime` 与断连状态，过期连接直接丢弃
//!
//! 简化约定：无后台清扫任务，过期连接在**下一次借出时**惰性淘汰。

use std::sync::{Arc, Mutex};

use alun_db_core::error::{DbError, Result};
use alun_db_core::traits::driver::PoolConfig;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::connection::PostgresConnection;
use crate::transaction::PostgresPoolTransaction;

/// PostgreSQL 连接池。
pub struct PostgresPool {
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
    idle: Mutex<Vec<PostgresConnection>>,
    /// 借出许可信号量。并发借出数不超过其容量。
    permits: Arc<Semaphore>,
}

impl PostgresPool {
    /// 创建连接池。
    ///
    /// 按 `min_connections` 预热连接（预热失败即创建失败——启动时
    /// 数据库不可达应视为错误）。并发借出上限为 `max_connections`（至少 1）。
    ///
    /// # 错误
    ///
    /// 预热连接失败时返回 [`DbError::Connection`]。
    pub async fn new(url: &str, config: PoolConfig) -> Result<Self> {
        let capacity = config.max_connections.max(1) as usize;
        // 预热连接数不超过容量
        let warmup = config.min_connections.min(config.max_connections) as usize;

        let inner = PoolInner {
            url: url.to_string(),
            idle: Mutex::new(Vec::new()),
            permits: Arc::new(Semaphore::new(capacity)),
            config,
        };

        // 预热：逐条建立，任一失败即中止建池
        for _ in 0..warmup {
            let conn = PostgresConnection::connect(url).await?;
            push_idle(&inner, conn)?;
        }

        Ok(PostgresPool {
            inner: Arc::new(inner),
        })
    }

    /// 获取一个连接。
    ///
    /// 空闲连接优先复用（过期或断连的直接丢弃）；不足时新建。
    /// 并发借出数受 `max_connections` 限制，等待超过 `connect_timeout`
    /// 返回错误。连接在 `Drop` 时自动归还。
    ///
    /// # 错误
    ///
    /// 获取超时、锁中毒或新建连接失败时返回 [`DbError`]。
    pub async fn get(&self) -> Result<PooledPgConnection> {
        // 信号量限流：等待超过 connect_timeout 视为获取失败
        let permit = {
            let timeout = self.inner.config.connect_timeout;
            tokio::time::timeout(timeout, self.inner.permits.clone().acquire_owned())
                .await
                .map_err(|_| {
                    DbError::Connection(
                        format!("pool: acquire connection timed out after {timeout:?}").into(),
                    )
                })?
                .map_err(|_| DbError::Connection("pool: semaphore closed".into()))?
        };

        // 先尝试弹出可用空闲连接；锁在块内释放，不跨越后续 await
        let popped = pop_available(&self.inner)?;
        let conn = match popped {
            Some(conn) => conn,
            None => PostgresConnection::connect(&self.inner.url).await?,
        };

        Ok(PooledPgConnection {
            conn: Some(conn),
            pool: self.inner.clone(),
            permit: Some(permit),
        })
    }

    /// 开始事务。从事务池借出专属连接并执行 `BEGIN`。
    ///
    /// 事务期间连接被独占，提交 / 回滚 / 句柄销毁后连接归还池
    /// （未终结销毁的连接按 Drop 策略丢弃，见 [`crate::transaction`]）。
    ///
    /// # 错误
    ///
    /// 借出连接或执行 `BEGIN` 失败时返回 [`DbError`]。
    pub async fn begin_transaction(&self) -> Result<PostgresPoolTransaction> {
        let pooled = self.get().await?;
        pooled.conn().begin().await?;
        Ok(PostgresPoolTransaction::begun(pooled))
    }

    /// 获取连接并设置本次借出的租户 `search_path`（PG schema-per-tenant）。
    ///
    /// 语义同 [`get`](Self::get)，额外在连接借出后应用
    /// [`PostgresConnection::apply_search_path`]。`schema = None` 表示使用默认
    /// `search_path`（若连接上次被设过租户 schema 则重置）。
    ///
    /// # 错误
    ///
    /// 获取连接失败、schema 名非法或 `SET search_path` 失败时返回 [`DbError`]。
    pub async fn get_with_search_path(&self, schema: Option<&str>) -> Result<PooledPgConnection> {
        let pooled = self.get().await?;
        pooled.conn().apply_search_path(schema).await?;
        Ok(pooled)
    }

    /// 开始事务，并在 `BEGIN` 之前设置本次借出的租户 `search_path`。
    ///
    /// `SET` 必须在 `BEGIN` 之前（会话级）执行，否则事务回滚会连带撤销
    /// `search_path`。其余语义同 [`begin_transaction`](Self::begin_transaction)。
    ///
    /// # 错误
    ///
    /// 获取连接失败、schema 名非法、`SET search_path` 或 `BEGIN` 失败时返回
    /// [`DbError`]。
    pub async fn begin_transaction_with_search_path(
        &self,
        schema: Option<&str>,
    ) -> Result<PostgresPoolTransaction> {
        let pooled = self.get_with_search_path(schema).await?;
        pooled.conn().begin().await?;
        Ok(PostgresPoolTransaction::begun(pooled))
    }
}

/// 归还连接到空闲栈。
///
/// 锁中毒时返回错误（调用方决定丢弃连接）。
fn push_idle(inner: &PoolInner, conn: PostgresConnection) -> Result<()> {
    let mut idle = inner
        .idle
        .lock()
        .map_err(|e| DbError::Connection(format!("pool lock poisoned: {e}").into()))?;
    idle.push(conn);
    Ok(())
}

/// 弹出一条可用的空闲连接。
///
/// 过期（超过 `max_lifetime`）或已断开的连接直接丢弃，继续尝试下一条。
fn pop_available(inner: &PoolInner) -> Result<Option<PostgresConnection>> {
    loop {
        let popped = {
            let mut idle = inner
                .idle
                .lock()
                .map_err(|e| DbError::Connection(format!("pool lock poisoned: {e}").into()))?;
            idle.pop()
        };
        match popped {
            Some(conn) if conn.is_available(&inner.config) => return Ok(Some(conn)),
            // 不可用：丢弃（Drop），继续弹出下一条
            Some(_) => continue,
            None => return Ok(None),
        }
    }
}

/// 池化连接。`Drop` 时自动归还到池中。
pub struct PooledPgConnection {
    /// 池化连接本体。仅 Drop / discard 中被取走。
    conn: Option<PostgresConnection>,
    /// 所属池的共享状态。
    pool: Arc<PoolInner>,
    /// 借出许可。`Drop` 时自动释放，归还并发容量。
    ///
    /// RAII 守卫字段：仅持有不读取，容量随字段 Drop 归还。
    #[allow(dead_code)]
    permit: Option<OwnedSemaphorePermit>,
}

impl PooledPgConnection {
    /// 访问内部连接。
    pub fn conn(&self) -> &PostgresConnection {
        match self.conn.as_ref() {
            Some(conn) => conn,
            // conn 字段仅在 Drop 中被取走；&self 存在时 Drop 尚未运行，
            // 借用检查保证此分支不可达
            None => unreachable!("pooled connection already returned to pool"),
        }
    }

    /// 丢弃连接：**不归还池**，permit 随之释放。
    ///
    /// 用于未终结事务的 Drop 语义：连接销毁触发服务端断连，
    /// 未提交事务由服务端自动回滚，池内不残留可疑状态的连接。
    pub fn discard(mut self) {
        // 先取走连接令其销毁（绕过 Drop 的归还逻辑）
        drop(self.conn.take());
        // self 随后 Drop：permit 释放，容量恢复
    }
}

impl Drop for PooledPgConnection {
    fn drop(&mut self) {
        // 归还连接；锁中毒时连接丢弃（容量随 permit 释放恢复）
        if let Some(conn) = self.conn.take() {
            let _ = push_idle(&self.pool, conn);
        }
        // permit 随字段 Drop 自动释放
    }
}

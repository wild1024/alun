//! MySQL 连接池。
//!
//! 自研轻量池（与 SQLite / PostgreSQL 驱动同构，不使用 mysql_async
//! 自带的池）：
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

use crate::connection::MysqlConnection;
use crate::transaction::MysqlPoolTransaction;

/// MySQL 连接池。
pub struct MysqlPool {
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
    idle: Mutex<Vec<MysqlConnection>>,
    /// 借出许可信号量。并发借出数不超过其容量。
    permits: Arc<Semaphore>,
}

impl MysqlPool {
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
            let conn = MysqlConnection::connect(url).await?;
            push_idle(&inner, conn)?;
        }

        Ok(MysqlPool {
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
    pub async fn get(&self) -> Result<PooledMysqlConnection> {
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
            None => MysqlConnection::connect(&self.inner.url).await?,
        };

        Ok(PooledMysqlConnection {
            conn: Some(conn),
            pool: self.inner.clone(),
            permit: Some(permit),
        })
    }

    /// 开始事务。从事务池借出专属连接并执行 `START TRANSACTION`。
    ///
    /// 事务期间连接被独占，提交 / 回滚 / 句柄销毁后连接归还池
    /// （未终结销毁的连接按 Drop 策略丢弃，见 [`crate::transaction`]）。
    ///
    /// # 错误
    ///
    /// 借出连接或执行 `START TRANSACTION` 失败时返回 [`DbError`]。
    pub async fn begin_transaction(&self) -> Result<MysqlPoolTransaction> {
        let mut pooled = self.get().await?;
        pooled.conn_mut().begin().await?;
        Ok(MysqlPoolTransaction::begun(pooled))
    }
}

/// 归还连接到空闲栈。
///
/// 锁中毒时返回错误（调用方决定丢弃连接）。
fn push_idle(inner: &PoolInner, conn: MysqlConnection) -> Result<()> {
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
fn pop_available(inner: &PoolInner) -> Result<Option<MysqlConnection>> {
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
pub struct PooledMysqlConnection {
    /// 池化连接本体。仅 Drop / discard 中被取走。
    conn: Option<MysqlConnection>,
    /// 所属池的共享状态。
    pool: Arc<PoolInner>,
    /// 借出许可。`Drop` 时自动释放，归还并发容量。
    ///
    /// RAII 守卫字段：仅持有不读取，容量随字段 Drop 归还。
    #[allow(dead_code)]
    permit: Option<OwnedSemaphorePermit>,
}

impl PooledMysqlConnection {
    /// 可变访问内部连接。
    ///
    /// mysql_async 的 `Conn` 以 `&mut self` 执行查询，与 tokio-postgres
    /// 的 `&self` Client 不同，池化句柄提供可变借用。
    pub fn conn_mut(&mut self) -> &mut MysqlConnection {
        match self.conn.as_mut() {
            Some(conn) => conn,
            // conn 字段仅在 Drop 中被取走；&mut self 存在时 Drop 尚未运行，
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

impl Drop for PooledMysqlConnection {
    fn drop(&mut self) {
        // 归还连接；锁中毒时连接丢弃（容量随 permit 释放恢复）
        if let Some(conn) = self.conn.take() {
            let _ = push_idle(&self.pool, conn);
        }
        // permit 随字段 Drop 自动释放
    }
}

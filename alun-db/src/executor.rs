//! facade 内部的 dyn-safe 执行抽象。
//!
//! `Db` 需要在运行时按配置选择驱动，必须持有 trait object。core 层保持
//! 零异步依赖，因此这里定义 facade 专用的 dyn-safe trait，用
//! `Pin<Box<dyn Future>>` 做异步分发。本模块只服务 facade 内部，
//! 不对外暴露。

use std::future::Future;
use std::pin::Pin;

use alun_db_core::error::{DbError, Result};
use alun_db_core::statement::Statement;
use alun_db_core::traits::dialect::Dialect;
use alun_db_core::traits::executor::{ExecuteResult, Rows};

/// `begin_raw` 的异步返回类型别名（dyn-safe，携带生命周期）。
///
/// 返回类型嵌套 `Box<dyn RawTransaction>` 层级较深，clippy `type_complexity`
/// 会报警，抽出别名使 trait 方法签名（见 [`RawExecutor::begin_raw`]）可读。
type RawTxBeginFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn RawTransaction>>> + Send + 'a>>;

/// 在阻塞上下文执行一段同步 SQLite 调用。
///
/// ## 为什么不用单纯 `spawn_blocking`
///
/// `spawn_blocking` 每次都要把任务投递到阻塞线程池并等待回传，实测空任务
/// 往返约 3.4 µs——而一次 SQLite 点查总共才 ~9.5 µs，派发就占了三分之一。
/// 多线程运行时改用 `tokio::task::block_in_place` 就地执行（实测 ~0.01 µs），
/// 直接省掉这次线程往返。
///
/// ## 为什么仍要保留 `spawn_blocking` 分支
///
/// `block_in_place` 会 panic 的场合（tokio 在“进入运行时但禁止就地阻塞”时
/// 直接 panic）：当前线程运行时、以及多线程运行时上的 `LocalSet`。前者可由
/// [`is_multi_thread_runtime`] 排除；后者无法从公开 API 判定——alun-db 的
/// future 均为 `Send`，`LocalSet` 只用于 `!Send` 场景，故不在本层的支持范围内。
/// flavor 判定失败（不在运行时上下文中）同样退回 `spawn_blocking`。
async fn run_blocking<F, R>(f: F) -> Result<R>
where
    F: FnOnce() -> Result<R> + Send + 'static,
    R: Send + 'static,
{
    if is_multi_thread_runtime() {
        tokio::task::block_in_place(f)
    } else {
        tokio::task::spawn_blocking(f)
            .await
            .map_err(|e| DbError::Connection(format!("blocking task join failed: {e}").into()))?
    }
}

/// 当前是否运行在 tokio 多线程运行时上（`block_in_place` 仅在该形态下可用）。
fn is_multi_thread_runtime() -> bool {
    tokio::runtime::Handle::try_current()
        .map(|handle| handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
        .unwrap_or(false)
}

/// 内部执行器（dyn-safe）。
pub(crate) trait RawExecutor: Send + Sync {
    /// 驱动使用的方言。供 Builder 等渲染入口使用。
    fn dialect(&self) -> &'static dyn Dialect;

    /// 执行 INSERT / UPDATE / DELETE。
    fn execute_raw(
        &self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<ExecuteResult>> + Send + '_>>;

    /// 执行 SELECT。
    fn query_raw(&self, stmt: Statement)
    -> Pin<Box<dyn Future<Output = Result<Rows>> + Send + '_>>;

    /// 执行多语句 SQL 脚本（`;` 分隔的完整脚本，不支持参数绑定）。
    ///
    /// 驱动未启用多语句能力时返回 [`DbError::UnsupportedFeature`]。
    fn execute_script(&self, _sql: &str) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let dialect = self.dialect().name();
        Box::pin(async move {
            Err(DbError::UnsupportedFeature {
                feature: "execute_script",
                dialect: dialect.to_string(),
            })
        })
    }

    /// 开始事务，返回 dyn-safe 事务句柄。
    fn begin_raw(&self) -> RawTxBeginFuture<'_>;
}

/// 内部事务句柄（dyn-safe）。
///
/// 实现方负责 RAII 语义：句柄销毁时若事务未终结，同步回滚后归还连接。
///
/// ## 线程安全
///
/// 方法借用 `&mut self`（而非 `&self`）：驱动的底层连接（如 rusqlite）
/// 通常不是 `Sync`，`&self` 引用无法跨线程；`&mut` 借用只要求 `Send`。
pub(crate) trait RawTransaction: Send {
    /// 事务内执行 INSERT / UPDATE / DELETE。
    fn execute_raw(
        &mut self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<ExecuteResult>> + Send + '_>>;

    /// 事务内执行 SELECT。
    fn query_raw(
        &mut self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<Rows>> + Send + '_>>;

    /// 提交事务。消耗句柄。
    fn commit_raw(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;

    /// 回滚事务。消耗句柄。
    fn rollback_raw(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;
}

#[cfg(feature = "sqlite")]
impl RawExecutor for alun_db_driver_sqlite::SqlitePool {
    fn dialect(&self) -> &'static dyn Dialect {
        &alun_db_driver_sqlite::SqliteDialect
    }

    fn execute_script(&self, sql: &str) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let sql = sql.to_string();
        Box::pin(async move {
            let pooled = self.get().await?;
            // 脚本（迁移 / 初始化）可能长时间运行，保持 `spawn_blocking`：
            // 就地阻塞会占住一个工作线程，拖累同运行时的其他任务
            tokio::task::spawn_blocking(move || pooled.conn().execute_script_sync(&sql))
                .await
                .map_err(|e| {
                    DbError::Connection(format!("blocking task join failed: {e}").into())
                })?
        })
    }

    fn execute_raw(
        &self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<ExecuteResult>> + Send + '_>> {
        Box::pin(async move {
            let pooled = self.get().await?;
            // rusqlite 是同步库，必须放在阻塞上下文执行
            run_blocking(move || pooled.conn().execute_sync(&stmt)).await
        })
    }

    fn query_raw(
        &self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<Rows>> + Send + '_>> {
        Box::pin(async move {
            let pooled = self.get().await?;
            // rusqlite 是同步库，必须放在阻塞上下文执行
            run_blocking(move || pooled.conn().query_sync(&stmt)).await
        })
    }

    fn begin_raw(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn RawTransaction>>> + Send + '_>> {
        Box::pin(async move {
            let tx = self.begin_transaction().await?;
            Ok(Box::new(tx) as Box<dyn RawTransaction>)
        })
    }
}

#[cfg(feature = "postgres")]
impl RawExecutor for alun_db_driver_postgres::PostgresPool {
    fn dialect(&self) -> &'static dyn Dialect {
        &alun_db_driver_postgres::PostgresDialect
    }

    fn execute_script(&self, sql: &str) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let sql = sql.to_string();
        Box::pin(async move {
            let schema = crate::tenant::pg_schema_for_current_tenant()?;
            let pooled = self.get_with_search_path(schema.as_deref()).await?;
            pooled.conn().execute_script_async(&sql).await
        })
    }

    // tokio-postgres 原生异步，无需阻塞线程包装
    fn execute_raw(
        &self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<ExecuteResult>> + Send + '_>> {
        Box::pin(async move {
            let schema = crate::tenant::pg_schema_for_current_tenant()?;
            let pooled = self.get_with_search_path(schema.as_deref()).await?;
            pooled.conn().execute_stmt(&stmt).await
        })
    }

    fn query_raw(
        &self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<Rows>> + Send + '_>> {
        Box::pin(async move {
            let schema = crate::tenant::pg_schema_for_current_tenant()?;
            let pooled = self.get_with_search_path(schema.as_deref()).await?;
            pooled.conn().query_stmt(&stmt).await
        })
    }

    fn begin_raw(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn RawTransaction>>> + Send + '_>> {
        Box::pin(async move {
            let schema = crate::tenant::pg_schema_for_current_tenant()?;
            let tx = self
                .begin_transaction_with_search_path(schema.as_deref())
                .await?;
            Ok(Box::new(tx) as Box<dyn RawTransaction>)
        })
    }
}

#[cfg(feature = "postgres")]
impl RawTransaction for alun_db_driver_postgres::PostgresPoolTransaction {
    fn execute_raw(
        &mut self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<ExecuteResult>> + Send + '_>> {
        Box::pin(async move { self.execute_stmt(&stmt).await })
    }

    fn query_raw(
        &mut self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<Rows>> + Send + '_>> {
        Box::pin(async move { self.query_stmt(&stmt).await })
    }

    fn commit_raw(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        Box::pin(async move {
            let mut tx = *self;
            tx.commit().await
        })
    }

    fn rollback_raw(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        Box::pin(async move {
            let mut tx = *self;
            tx.rollback().await
        })
    }
}

#[cfg(feature = "mysql")]
impl RawExecutor for alun_db_driver_mysql::MysqlPool {
    fn dialect(&self) -> &'static dyn Dialect {
        &alun_db_driver_mysql::MysqlDialect
    }

    // mysql_async 原生异步；Conn 以 &mut self 执行，借出的池化句柄独占连接
    fn execute_raw(
        &self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<ExecuteResult>> + Send + '_>> {
        Box::pin(async move {
            let mut pooled = self.get().await?;
            pooled.conn_mut().execute_stmt(&stmt).await
        })
    }

    fn query_raw(
        &self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<Rows>> + Send + '_>> {
        Box::pin(async move {
            let mut pooled = self.get().await?;
            pooled.conn_mut().query_stmt(&stmt).await
        })
    }

    fn begin_raw(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Box<dyn RawTransaction>>> + Send + '_>> {
        Box::pin(async move {
            let tx = self.begin_transaction().await?;
            Ok(Box::new(tx) as Box<dyn RawTransaction>)
        })
    }
}

#[cfg(feature = "mysql")]
impl RawTransaction for alun_db_driver_mysql::MysqlPoolTransaction {
    fn execute_raw(
        &mut self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<ExecuteResult>> + Send + '_>> {
        Box::pin(async move { self.execute_stmt(&stmt).await })
    }

    fn query_raw(
        &mut self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<Rows>> + Send + '_>> {
        Box::pin(async move { self.query_stmt(&stmt).await })
    }

    fn commit_raw(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        Box::pin(async move {
            let mut tx = *self;
            tx.commit().await
        })
    }

    fn rollback_raw(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        Box::pin(async move {
            let mut tx = *self;
            tx.rollback().await
        })
    }
}

#[cfg(feature = "sqlite")]
impl RawTransaction for alun_db_driver_sqlite::SqlitePoolTransaction {
    fn execute_raw(
        &mut self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<ExecuteResult>> + Send + '_>> {
        Box::pin(async move {
            // 连接临时移出句柄放入阻塞上下文执行，完成后放回，
            // 避免 future 持有非 Sync 的连接引用
            let pooled = self.take_pooled()?;
            let (pooled, result) = run_blocking(move || {
                let r = pooled.conn().execute_sync(&stmt);
                Ok((pooled, r))
            })
            .await?;
            match result {
                Ok(r) => {
                    self.restore_pooled(pooled);
                    Ok(r)
                }
                Err(e) => {
                    // 语句失败：先归还连接并同步回滚，卸载未提交的事务状态，
                    // 避免带残留事务的连接被归还池；回滚自身失败不掩盖原始错误
                    self.restore_pooled(pooled);
                    let _ = self.rollback();
                    Err(e)
                }
            }
        })
    }

    fn query_raw(
        &mut self,
        stmt: Statement,
    ) -> Pin<Box<dyn Future<Output = Result<Rows>> + Send + '_>> {
        Box::pin(async move {
            let pooled = self.take_pooled()?;
            let (pooled, result) = run_blocking(move || {
                let r = pooled.conn().query_sync(&stmt);
                Ok((pooled, r))
            })
            .await?;
            match result {
                Ok(r) => {
                    self.restore_pooled(pooled);
                    Ok(r)
                }
                Err(e) => {
                    // 语句失败：先归还连接并同步回滚，卸载未提交的事务状态，
                    // 避免带残留事务的连接被归还池；回滚自身失败不掩盖原始错误
                    self.restore_pooled(pooled);
                    let _ = self.rollback();
                    Err(e)
                }
            }
        })
    }

    fn commit_raw(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        // COMMIT 是 SQLite 本地操作，同步完成（连接随终结归还池）
        Box::pin(async move {
            let mut tx = *self;
            tx.commit()
        })
    }

    fn rollback_raw(self: Box<Self>) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        Box::pin(async move {
            let mut tx = *self;
            tx.rollback()
        })
    }
}

//! 异步 Hook（[`AsyncHook`](alun_db_core::traits::AsyncHook)）的**事务感知派发**。
//!
//! # 为什么需要「队列」而不是直接 await
//!
//! 写入路径在语句执行后立即调用 Hook。若直接 `await` 异步副作用，会得到这样的
//! 时序（在显式事务中）：
//!
//! ```text
//! BEGIN → INSERT → 【异步审计落库】 → ROLLBACK
//!                     ↑ 审计已写，但业务被回滚 → 数据不一致
//! ```
//!
//! 因此本模块把异步钩子**推迟到提交边界**：
//!
//! - `&Db`（自动提交）：语句本身即为一次提交 → 立即 `await`；
//! - `&Transaction`：任务压入 [`PostCommitQueue`]，`commit()` 成功后才统一执行；
//!   `rollback()` 时整体丢弃。
//!
//! # 失败语义
//!
//! 提交后的钩子失败**不会**回滚（事务已提交），仅记 `warn` 日志——避免
//! "已提交却因审计失败而向调用方报错"这种更糟的不一致。

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use alun_db_core::row::Row;
use alun_db_core::stmt::Value;
use alun_db_core::traits::model::Model;

use crate::query::SqlExecutor;

/// 提交后要执行的异步任务（已装箱、`Send`、`'static`）。
pub(crate) type PostCommitTask = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// 提交后任务队列。
///
/// 挂载在 [`Transaction`](crate::tx::Transaction) 上：写入路径把异步副作用压入
/// 队列，事务提交成功后统一执行、回滚时丢弃。
///
/// 用 `Mutex` 而非无锁结构：队列只在写入路径与 commit 两处访问，且 commit 本身
/// 是独占语义，锁竞争可忽略；实现简单且无 `unsafe`。
#[derive(Default)]
pub struct PostCommitQueue {
    tasks: Mutex<Vec<PostCommitTask>>,
}

impl PostCommitQueue {
    /// 压入一个待执行任务。
    pub fn push(&self, task: PostCommitTask) {
        self.tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(task);
    }

    /// 取出全部任务并依次 `await`，返回执行条数。
    ///
    /// 任务内部自行吞掉错误（只记日志），因此本方法不会失败。
    pub async fn drain_and_run(&self) -> usize {
        let tasks: Vec<PostCommitTask> = {
            let mut guard = self
                .tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *guard)
        };
        let count = tasks.len();
        for task in tasks {
            task.await;
        }
        count
    }

    /// 丢弃全部待执行任务（回滚路径），返回丢弃条数。
    pub fn clear(&self) -> usize {
        let mut guard = self
            .tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let count = guard.len();
        guard.clear();
        count
    }

    /// 当前待执行任务数（测试 / 观测用）。
    pub fn len(&self) -> usize {
        self.tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    /// 队列是否为空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// 记录一条异步 Hook 失败日志（提交后失败不回滚，仅告警）。
fn warn_hook(model: &str, phase: &str, err: &dyn std::fmt::Display) {
    tracing::warn!(
        model = model,
        phase = phase,
        "异步 Hook 执行失败（事务已提交，不回滚；请检查副作用实现）: {err}"
    );
}

/// 派发 `after_insert`（提交边界感知）。
pub(crate) async fn dispatch_after_insert<T: Model, E: SqlExecutor>(ex: &E, row: &Row) {
    let hooks = T::async_hooks();
    if hooks.is_empty() {
        return;
    }
    match ex.post_commit_queue() {
        // 自动提交：语句已提交，立即执行
        None => {
            for hook in hooks {
                if let Err(e) = hook.after_insert(row).await {
                    warn_hook(T::TABLE, "after_insert", &e);
                }
            }
        }
        // 显式事务：排队至 commit 成功后
        Some(queue) => {
            for hook in hooks {
                let owned = row.clone();
                queue.push(Box::pin(async move {
                    if let Err(e) = hook.after_insert(&owned).await {
                        tracing::warn!(model = T::TABLE, phase = "after_insert",
                            "异步 Hook 执行失败（事务已提交，不回滚）: {e}");
                    }
                }));
            }
        }
    }
}

/// 派发 `after_update`（提交边界感知）。
pub(crate) async fn dispatch_after_update<T: Model, E: SqlExecutor>(ex: &E, row: &Row) {
    let hooks = T::async_hooks();
    if hooks.is_empty() {
        return;
    }
    match ex.post_commit_queue() {
        None => {
            for hook in hooks {
                if let Err(e) = hook.after_update(row).await {
                    warn_hook(T::TABLE, "after_update", &e);
                }
            }
        }
        Some(queue) => {
            for hook in hooks {
                let owned = row.clone();
                queue.push(Box::pin(async move {
                    if let Err(e) = hook.after_update(&owned).await {
                        tracing::warn!(model = T::TABLE, phase = "after_update",
                            "异步 Hook 执行失败（事务已提交，不回滚）: {e}");
                    }
                }));
            }
        }
    }
}

/// 派发 `after_delete`（提交边界感知）。
pub(crate) async fn dispatch_after_delete<T: Model, E: SqlExecutor>(ex: &E, pk: &Value) {
    let hooks = T::async_hooks();
    if hooks.is_empty() {
        return;
    }
    match ex.post_commit_queue() {
        None => {
            for hook in hooks {
                if let Err(e) = hook.after_delete(pk).await {
                    warn_hook(T::TABLE, "after_delete", &e);
                }
            }
        }
        Some(queue) => {
            for hook in hooks {
                let owned = pk.clone();
                queue.push(Box::pin(async move {
                    if let Err(e) = hook.after_delete(&owned).await {
                        tracing::warn!(model = T::TABLE, phase = "after_delete",
                            "异步 Hook 执行失败（事务已提交，不回滚）: {e}");
                    }
                }));
            }
        }
    }
}

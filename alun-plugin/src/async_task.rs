//! 异步任务插件：后台任务队列

use async_trait::async_trait;
use alun_core::{Plugin, Result};
use std::sync::Arc;
use tokio::sync::Semaphore;

/// 异步任务插件：基于 Semaphore 的并发控制后台任务队列
///
/// `stop()` 时会等待所有正在执行的任务完成后才返回。
pub struct AsyncTaskPlugin {
    /// 并发工作线程数
    workers: usize,
    /// Semaphore 用于控制并发数
    semaphore: Arc<Semaphore>,
    /// 运行状态标志
    running: Arc<parking_lot::Mutex<bool>>,
}

impl AsyncTaskPlugin {
    /// 创建异步任务插件
    ///
    /// `workers` = 0 时自动设为 4（CPU 核数的合理默认值）。
    pub fn new(workers: usize) -> Self {
        let w = if workers == 0 { 4 } else { workers };
        Self {
            workers: w,
            semaphore: Arc::new(Semaphore::new(w)),
            running: Arc::new(parking_lot::Mutex::new(false)),
        }
    }

    /// 提交异步任务
    pub async fn submit<F>(&self, task: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let sem = self.semaphore.clone();
        tokio::spawn(async move {
            let _permit = sem.acquire().await.expect("AsyncTask semaphore 异常");
            task.await;
        });
    }

    /// 工作线程数
    pub fn worker_count(&self) -> usize { self.workers }
}

#[async_trait]
impl Plugin for AsyncTaskPlugin {
    fn name(&self) -> &str { "async-task" }

    async fn start(&self) -> Result<()> {
        *self.running.lock() = true;
        tracing::info!("异步任务插件: workers={} 已就绪", self.workers);
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        *self.running.lock() = false;
        tracing::info!("异步任务插件: 等待任务完成...");
        // 等待所有 semaphore permit 回收
        let _permits = self.semaphore.acquire_many(self.workers as u32).await;
        Ok(())
    }
}

//! 定时任务插件：cron 表达式调度

use async_trait::async_trait;
use alun_core::{Plugin, Result};
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;

type JobFn = Box<dyn Fn() -> tokio::task::JoinHandle<()> + Send + Sync>;

/// 定时任务插件：cron 表达式调度
///
/// 支持注册/移除/手动触发/列举任务。注意：本实现为任务注册中心，
/// 实际调度需搭配 cron runner（如 `tokio-cron-scheduler`）。
pub struct SchedulerPlugin {
    /// 任务注册表（name → ScheduledJob）
    jobs: Arc<RwLock<HashMap<String, ScheduledJob>>>,
}

struct ScheduledJob {
    cron: String,
    description: String,
    runner: JobFn,
}

impl SchedulerPlugin {
    /// 创建空的调度器
    pub fn new() -> Self {
        Self { jobs: Arc::new(RwLock::new(HashMap::new())) }
    }

    /// 注册定时任务
    ///
    /// ```ignore
    /// scheduler.register("cleanup", "0 */5 * * * *", "每5分钟清理", || {
    ///     tokio::spawn(async { cleanup_task().await })
    /// });
    /// ```
    pub fn register<F>(
        &self,
        name: &str,
        cron: &str,
        desc: &str,
        runner: F,
    ) where
        F: Fn() -> tokio::task::JoinHandle<()> + Send + Sync + 'static,
    {
        self.jobs.write().insert(name.to_string(), ScheduledJob {
            cron: cron.to_string(),
            description: desc.to_string(),
            runner: Box::new(runner),
        });
    }

    /// 移除定时任务
    pub fn remove(&self, name: &str) {
        self.jobs.write().remove(name);
    }

    /// 列出所有注册的任务
    pub fn list(&self) -> Vec<(String, String, String)> {
        self.jobs.read()
            .iter()
            .map(|(k, v)| (k.clone(), v.cron.clone(), v.description.clone()))
            .collect()
    }

    /// 手动触发指定的定时任务
    pub fn trigger(&self, name: &str) -> Option<tokio::task::JoinHandle<()>> {
        let guard = self.jobs.read();
        guard.get(name).map(|job| (job.runner)())
    }
}

#[async_trait]
impl Plugin for SchedulerPlugin {
    fn name(&self) -> &str { "scheduler" }

    async fn start(&self) -> Result<()> {
        let count = self.jobs.read().len();
        tracing::info!("定时任务插件: {} 个任务已注册", count);
        for (name, job) in self.jobs.read().iter() {
            tracing::info!("  - {} [{}] {}", name, job.cron, job.description);
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        tracing::info!("定时任务插件: 已停止");
        Ok(())
    }
}

impl Default for SchedulerPlugin {
    fn default() -> Self { Self::new() }
}

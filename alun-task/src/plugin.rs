//! TaskPlugin —— 任务插件入口
//!
//! 实现 `alun_core::Plugin` trait，通过 `PluginManager` 管理生命周期。
//! 不持有任何数据库连接——持久化通过 `TaskStorage` trait 委托给业务方。

use std::sync::Arc;
use async_trait::async_trait;
use alun_core::{Plugin, Result as CoreResult};
use tokio::task::JoinHandle;
use tracing::{info, error};

use crate::storage::TaskStorage;
use crate::HandlerRegistry;
use crate::TaskWorker;
use crate::RetryScanner;
use crate::TaskMetrics;
use crate::TaskWorkerConfig;

/// 任务插件
///
/// 管理 TaskWorker 和 RetryScanner 的生命周期。
/// 持久化通过 `TaskStorage` trait 委托给业务方。
///
/// 实现 `alun_core::Plugin`，可通过 `PluginManager` 或 `App::plugin()` 统一管理。
///
/// # 使用示例
///
/// ```ignore
/// // 配置从 config.toml 的 [task] section 读取
/// let task_cfg: TaskWorkerConfig = app.config().get_section("task")?;
/// let storage = Arc::new(DbTaskStorage::new());
/// let task_plugin = TaskPlugin::new(task_cfg, storage, registry)?;
/// app.plugin(task_plugin).scan().start().await
/// ```
pub struct TaskPlugin {
    /// TaskWorker 运行时配置
    config: TaskWorkerConfig,
    /// 任务持久化接口（由业务方实现）
    storage: Arc<dyn TaskStorage>,
    /// 处理器注册中心
    registry: HandlerRegistry,
    /// tokio 任务句柄（用于停止时等待任务完成）
    handles: parking_lot::Mutex<Vec<JoinHandle<()>>>,
    /// 运行状态标志
    running: Arc<std::sync::atomic::AtomicBool>,
    /// 任务执行指标
    metrics: Arc<TaskMetrics>,
    /// 已注册的 Kafka topic 列表
    topics: Vec<String>,
}

impl TaskPlugin {
    /// 创建任务插件
    ///
    /// - `config`: TaskWorker 运行时配置（建议从 `[task]` section 读取，支持 Deserialize）
    /// - `storage`: 由业务方实现的持久化接口
    /// - `registry`: 已注册 handler 的注册中心
    pub fn new(
        config: TaskWorkerConfig,
        storage: Arc<dyn TaskStorage>,
        registry: HandlerRegistry,
    ) -> Result<Self, String> {
        let mut topics: Vec<String> = registry
            .task_types()
            .iter()
            .filter_map(|tt| registry.get_config(*tt).map(|c| c.topic.clone()))
            .collect();
        topics.sort_unstable();
        topics.dedup();

        let metrics = Arc::new(TaskMetrics::new());

        Ok(Self {
            config,
            storage,
            registry,
            handles: parking_lot::Mutex::new(Vec::new()),
            running: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            metrics,
            topics,
        })
    }

    /// 返回任务指标（供外部查询）
    pub fn metrics(&self) -> Arc<TaskMetrics> {
        Arc::clone(&self.metrics)
    }

    /// 返回已注册的 topic 列表
    pub fn topics(&self) -> &[String] {
        &self.topics
    }

    fn signal_stop(&self) {
        self.running.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    async fn wait_for_tasks(&self) {
        let handles: Vec<JoinHandle<()>> = {
            let mut guard = self.handles.lock();
            std::mem::take(&mut *guard)
        };
        for handle in handles {
            let _ = handle.await;
        }
    }
}

#[async_trait]
impl Plugin for TaskPlugin {
    fn name(&self) -> &str {
        "task"
    }

    fn depends_on(&self) -> &[&str] {
        &[]
    }

    /// 启动插件：在后台 tokio task 中启动 Worker 和 RetryScanner
    async fn start(&self) -> CoreResult<()> {
        self.running.store(true, std::sync::atomic::Ordering::Relaxed);

        let topics = self.topics.clone();
        let running = Arc::clone(&self.running);
        let metrics = Arc::clone(&self.metrics);

        let worker = {
            let w_config = self.config.clone();
            let w_storage = Arc::clone(&self.storage);
            let w_registry = self.registry.clone();
            let w_metrics = Arc::clone(&metrics);
            let w_running = Arc::clone(&running);
            let w_topics = topics.clone();

            tokio::spawn(async move {
                let w_config_clone = w_config.clone();
                let w_storage_clone = Arc::clone(&w_storage);
                let w_registry_clone = w_registry.clone();
                let worker = match TaskWorker::new(w_config_clone, w_storage_clone, w_registry_clone, w_metrics, &w_topics) {
                    Ok(w) => Arc::new(w),
                    Err(e) => {
                        error!("TaskWorker 创建失败: {}", e);
                        return;
                    }
                };

                let worker_ref = Arc::clone(&worker);
                let run_fut = worker_ref.run();

                tokio::select! {
                    result = run_fut => {
                        if let Err(e) = result {
                            error!("TaskWorker 运行异常: {}", e);
                        }
                    }
                    _ = async {
                        while w_running.load(std::sync::atomic::Ordering::Relaxed) {
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        }
                    } => {
                        worker_ref.stop();
                    }
                }
            })
        };

        let scanner = {
            let s_brokers = self.config.brokers.clone();
            let s_storage = Arc::clone(&self.storage);
            let s_registry = self.registry.clone();
            let s_interval = self.config.scan_interval_secs;
            let s_batch = self.config.max_batch_size;
            let s_running = Arc::clone(&running);

            tokio::spawn(async move {
                let scanner = match RetryScanner::new(
                    &s_brokers, s_storage, s_registry, s_interval, s_batch,
                ) {
                    Ok(s) => Arc::new(s),
                    Err(e) => {
                        error!("RetryScanner 创建失败: {}", e);
                        return;
                    }
                };

                let scanner_ref = Arc::clone(&scanner);

                tokio::select! {
                    _ = scanner_ref.run() => {}
                    _ = async {
                        while s_running.load(std::sync::atomic::Ordering::Relaxed) {
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        }
                    } => {
                        scanner_ref.stop();
                    }
                }
            })
        };

        self.handles.lock().push(worker);
        self.handles.lock().push(scanner);

        info!(
            "TaskPlugin 启动: topics={:?}, brokers={}, group={}",
            self.topics, self.config.brokers, self.config.group_id
        );
        Ok(())
    }

    /// 停止插件：发送停止信号并等待后台任务完成
    async fn stop(&self) -> CoreResult<()> {
        info!("TaskPlugin 收到停止信号");
        self.signal_stop();
        self.wait_for_tasks().await;
        info!("TaskPlugin 已停止");
        Ok(())
    }
}
//! 重试机制 —— 后台定期扫描待重试任务并重新推入 Kafka

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use rdkafka::producer::{FutureProducer, FutureRecord};

use crate::storage::TaskStorage;
use crate::types::*;
use crate::HandlerRegistry;

/// 重试扫描器
///
/// 定期从 `TaskStorage` 扫描可重试任务，计算延迟后重新推入 Kafka。
/// 不持有 SQL——扫描逻辑完全由 `TaskStorage::scan_retryable_tasks()` 实现。
pub struct RetryScanner {
    /// 任务持久化接口
    storage: Arc<dyn TaskStorage>,
    /// 处理器注册中心
    registry: HandlerRegistry,
    /// Kafka 生产者（用于重新发送任务消息）
    producer: FutureProducer,
    /// 扫描间隔（秒）
    interval_secs: u64,
    /// 每批次扫描最大任务数
    max_batch_size: usize,
    /// 运行状态标志
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl RetryScanner {
    /// 创建重试扫描器
    pub fn new(
        brokers: &str,
        storage: Arc<dyn TaskStorage>,
        registry: HandlerRegistry,
        interval_secs: u64,
        max_batch_size: usize,
    ) -> Result<Self, String> {
        let producer: FutureProducer = rdkafka::ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("message.timeout.ms", "5000")
            .create()
            .map_err(|e| format!("Kafka Producer 创建失败（RetryScanner）: {}", e))?;

        Ok(Self {
            storage,
            registry,
            producer,
            interval_secs,
            max_batch_size,
            running: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        })
    }

    /// 启动扫描循环
    pub async fn run(&self) {
        info!("RetryScanner 启动，扫描间隔: {}s", self.interval_secs);

        while self.running.load(std::sync::atomic::Ordering::Relaxed) {
            if let Err(e) = self.scan().await {
                warn!("RetryScanner 扫描出错: {}", e);
            }
            tokio::time::sleep(Duration::from_secs(self.interval_secs)).await;
        }

        info!("RetryScanner 已停止");
    }

    /// 停止扫描循环
    pub fn stop(&self) {
        self.running.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// 执行一轮扫描：将可重试任务重新推入 Kafka
    async fn scan(&self) -> Result<(), String> {
        let task_types: Vec<i16> = self.registry.task_types();
        if task_types.is_empty() {
            return Ok(());
        }

        let tasks = self
            .storage
            .scan_retryable_tasks(&task_types, self.max_batch_size)
            .await?;

        let mut retried = 0usize;
        for task in &tasks {
            let config = match self.registry.get_config(task.task_type) {
                Some(c) => c,
                None => continue,
            };

            if task.retry_count >= task.max_retries {
                continue;
            }

            let delay = compute_retry_delay(
                &config.retry_strategy,
                config.retry_delay_seconds,
                config.max_retry_delay_seconds,
                task.retry_count as u32,
            );

            let _ = self
                .storage
                .update_retry(&task.task_id, task.retry_count)
                .await;

            let msg = TaskMessage {
                task_id: task.task_id.clone(),
                task_type: task.task_type,
                payload: task.payload.clone(),
                priority: 2i16,
                user_id: None,
                resource_id: None,
                resource_type: None,
                submitted_at: chrono::Utc::now().to_rfc3339(),
            };

            let payload = match serde_json::to_vec(&msg) {
                Ok(p) => p,
                Err(e) => {
                    warn!(task_id = %task.task_id, "重试消息序列化失败: {}", e);
                    continue;
                }
            };
            let record = FutureRecord::to(&config.topic)
                .key(&task.task_id)
                .payload(&payload);
            if let Err((e, _)) = self.producer.send(record, Duration::from_secs(5)).await {
                warn!(task_id = %task.task_id, "重试 Kafka 推送失败: {}", e);
                continue;
            }

            retried += 1;
            info!(task_id = %task.task_id, retry_count = task.retry_count, delay_s = delay, "重试任务推送成功");
        }

        if retried > 0 {
            info!("本轮 RetryScanner 重试 {} 个任务", retried);
        }
        Ok(())
    }
}

/// 计算重试延迟（秒）
pub fn compute_retry_delay(strategy: &RetryStrategy, base: u64, max: u64, attempt: u32) -> u64 {
    match strategy {
        RetryStrategy::Fixed => base,
        RetryStrategy::Linear => (base * (attempt + 1) as u64).min(max),
        RetryStrategy::Exponential => {
            let delay = base * 2u64.pow(attempt);
            delay.min(max)
        }
    }
}
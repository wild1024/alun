//! 任务消费者 —— 从 Kafka 消费任务并分发给 Handler 执行

use std::sync::Arc;
use std::time::Instant;
use rdkafka::consumer::{StreamConsumer, Consumer};
use rdkafka::ClientConfig;
use rdkafka::message::BorrowedMessage;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::Message;
use tokio::time::timeout;
use tracing::{info, warn, error};

use crate::storage::TaskStorage;
use crate::HandlerRegistry;
use crate::types::*;
use crate::TaskMetrics;
use crate::metrics::AtomicInc;

/// 任务执行器
///
/// 从 Kafka 消费 `TaskMessage`，按 `task_type` 查找注册的 `TaskHandler`，
/// 执行并委托 `TaskStorage` 记录结果、更新状态、处理重试和死信队列。
/// 不持有任何 SQL 或表名——持久化完全交由 storage 代理。
pub struct TaskWorker {
    /// Kafka 消费者
    consumer: Arc<StreamConsumer>,
    /// 任务持久化接口
    storage: Arc<dyn TaskStorage>,
    /// 处理器注册中心
    registry: HandlerRegistry,
    /// 任务执行指标
    metrics: Arc<TaskMetrics>,
    /// TaskWorker 运行时配置
    config: TaskWorkerConfig,
    /// Kafka 生产者（用于发送死信消息）
    producer: FutureProducer,
    /// 运行状态标志
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl TaskWorker {
    /// 创建任务执行器并订阅指定的 topics
    ///
    /// 注意：consumer 创建和 subscribe 必须在同一线程中完成，
    /// 因此此方法在 spawn_blocking 中调用。
    pub fn new(
        config: TaskWorkerConfig,
        storage: Arc<dyn TaskStorage>,
        registry: HandlerRegistry,
        metrics: Arc<TaskMetrics>,
        topics: &[String],
    ) -> Result<Self, String> {
        let consumer: StreamConsumer = ClientConfig::new()
            .set("bootstrap.servers", &config.brokers)
            .set("group.id", &config.group_id)
            .set("enable.auto.commit", "false")
            .set("auto.offset.reset", "earliest")
            .set("allow.auto.create.topics", "true")
            .create()
            .map_err(|e| format!("Kafka Consumer 创建失败: {}", e))?;

        // 在同一线程中订阅 topics（rdkafka 要求 subscribe 与 consumer 创建在同一线程）
        // 去重：同一 topic 注册多个 handler 时避免重复订阅
        let mut unique_topics: Vec<&str> = topics.iter().map(|s| s.as_str()).collect();
        unique_topics.sort_unstable();
        unique_topics.dedup();
        consumer
            .subscribe(&unique_topics)
            .map_err(|e| format!("Kafka Consumer 订阅失败: {}", e))?;

        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", &config.brokers)
            .set("message.timeout.ms", "5000")
            .create()
            .map_err(|e| format!("Kafka DLQ Producer 创建失败: {}", e))?;

        Ok(Self {
            consumer: Arc::new(consumer),
            storage,
            registry,
            metrics,
            config,
            producer,
            running: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        })
    }

    /// 启动消费循环（subscribe 已在 new 中完成）
    pub async fn run(&self) -> Result<(), String> {
        info!("TaskWorker 启动，开始消费消息");

        while self.running.load(std::sync::atomic::Ordering::Relaxed) {
            match self.consumer.recv().await {
                Err(e) => {
                    error!("Kafka 接收消息失败: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
                Ok(msg) => {
                    self.handle_message(&msg).await;
                }
            }
        }

        info!("TaskWorker 已停止");
        Ok(())
    }

    /// 停止消费循环
    pub fn stop(&self) {
        self.running.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// 处理单条 Kafka 消息：反序列化、查找 handler、执行、更新状态
    async fn handle_message(&self, msg: &BorrowedMessage<'_>) {
        let payload = match msg.payload() {
            Some(p) => p,
            None => {
                warn!("收到空消息");
                return;
            }
        };

        let task_msg: TaskMessage = match serde_json::from_slice(payload) {
            Ok(m) => m,
            Err(e) => {
                error!("消息反序列化失败: {}", e);
                return;
            }
        };

        if let Err(reason) = self.check_message_age(&task_msg) {
            warn!(task_id = %task_msg.task_id, reason = reason, "消息已过期，跳过");
            self.commit_offset(msg).await;
            return;
        }

        self.metrics.total.inc();

        let (handler, config) = match self.registry.get(task_msg.task_type) {
            Some(h) => h,
            None => {
                warn!(task_type = task_msg.task_type, "未找到 handler");
                self.commit_offset(msg).await;
                return;
            }
        };

        if let Err(e) = self.storage.update_task_status(&task_msg.task_id, TaskStatus::Processing).await {
            error!(task_id = %task_msg.task_id, error = %e, "update_task_status(Processing) 失败");
        }

        let started_at = Instant::now();

        let result = if config.timeout_seconds > 0 {
            match timeout(
                tokio::time::Duration::from_secs(config.timeout_seconds),
                handler.execute(task_msg.payload.clone()),
            )
            .await
            {
                Ok(r) => r,
                Err(_) => Err(format!("任务超时 ({}s)", config.timeout_seconds)),
            }
        } else {
            handler.execute(task_msg.payload.clone()).await
        };

        let elapsed_ms = started_at.elapsed().as_millis() as i64;

        match result {
            Ok(output) => {
                self.handle_success(&task_msg, &output, elapsed_ms).await;
            }
            Err(e) => {
                self.handle_failure(&task_msg, &e, &config, elapsed_ms).await;
            }
        }

        self.commit_offset(msg).await;
    }

    /// 异步提交 Kafka 消息 offset
    async fn commit_offset(&self, msg: &BorrowedMessage<'_>) {
        if let Err(e) = self.consumer.commit_message(msg, rdkafka::consumer::CommitMode::Async) {
            error!(error = %e, "Kafka offset 提交失败");
        }
    }

    /// 检查消息是否超过最大时效，超过则返回错误
    fn check_message_age(&self, msg: &TaskMessage) -> Result<(), String> {
        let submitted = chrono::DateTime::parse_from_rfc3339(&msg.submitted_at)
            .map_err(|e| format!("解析 submitted_at 失败: {}", e))?;
        let age = chrono::Utc::now()
            .signed_duration_since(submitted.with_timezone(&chrono::Utc))
            .num_seconds();
        if age > self.config.max_message_age_secs as i64 {
            return Err(format!(
                "消息已超过最大时效 {}s（实际 {}s）",
                self.config.max_message_age_secs, age
            ));
        }
        Ok(())
    }

    /// 处理任务执行成功：更新状态为 Completed、存储结果、记录执行日志
    async fn handle_success(&self, msg: &TaskMessage, output: &serde_json::Value, elapsed_ms: i64) {
        if let Err(e) = self.storage.update_task_status(&msg.task_id, TaskStatus::Completed).await {
            error!(task_id = %msg.task_id, error = %e, "update_task_status(Completed) 失败");
        }
        if let Err(e) = self.storage.save_task_result(&msg.task_id, output).await {
            error!(task_id = %msg.task_id, error = %e, "save_task_result 失败");
        }
        if let Err(e) = self.storage.log_execution(&msg.task_id, TaskStatus::Completed, None, elapsed_ms).await {
            error!(task_id = %msg.task_id, error = %e, "log_execution 失败");
        }
        self.metrics.completed.inc();
        info!(task_id = %msg.task_id, elapsed_ms = elapsed_ms, "任务执行成功");
    }

    /// 处理任务执行失败：判断是否超过最大重试次数，决定是转入死信队列还是等待重试
    async fn handle_failure(
        &self,
        msg: &TaskMessage,
        err_msg: &str,
        config: &TaskConfig,
        elapsed_ms: i64,
    ) {
        self.metrics.failed.inc();

        let current_retries = self.storage.get_retry_count(&msg.task_id).await.unwrap_or(0);
        let attempt = current_retries + 1;

        if attempt as u32 > config.max_retries {
            if let Some(ref dlq_topic) = config.dead_letter_topic {
                warn!(task_id = %msg.task_id, attempt = attempt, "转入死信队列");
                if let Err(e) = self.send_to_dlq(msg, dlq_topic, err_msg).await {
                    error!(task_id = %msg.task_id, error = %e, "send_to_dlq 失败");
                }
                if let Err(e) = self.storage.update_task_status(&msg.task_id, TaskStatus::DeadLetter).await {
                    error!(task_id = %msg.task_id, error = %e, "update_task_status(DeadLetter) 失败");
                }
            } else {
                warn!(task_id = %msg.task_id, attempt = attempt, "超过最大重试次数");
                if let Err(e) = self.storage.update_task_status(&msg.task_id, TaskStatus::Failed).await {
                    error!(task_id = %msg.task_id, error = %e, "update_task_status(Failed) 失败");
                }
                if let Err(e) = self.storage.save_task_result(
                    &msg.task_id,
                    &serde_json::json!({"error": err_msg, "retries": attempt}),
                ).await {
                    error!(task_id = %msg.task_id, error = %e, "save_task_result 失败");
                }
            }
            if let Err(e) = self.storage.log_execution(&msg.task_id, TaskStatus::Failed, Some(err_msg), elapsed_ms).await {
                error!(task_id = %msg.task_id, error = %e, "log_execution 失败");
            }
        } else {
            if let Err(e) = self.storage.update_retry(&msg.task_id, attempt).await {
                error!(task_id = %msg.task_id, error = %e, "update_retry 失败");
            }
            if let Err(e) = self.storage.log_execution(&msg.task_id, TaskStatus::Failed, Some(err_msg), elapsed_ms).await {
                error!(task_id = %msg.task_id, error = %e, "log_execution 失败");
            }
            info!(task_id = %msg.task_id, attempt = attempt, "任务失败，等待重试: {}", err_msg);
        }
    }

    /// 发送任务到死信队列（DLQ）
    async fn send_to_dlq(
        &self,
        msg: &TaskMessage,
        dead_letter_topic: &str,
        reason: &str,
    ) -> Result<(), String> {
        let payload = serde_json::to_vec(msg).map_err(|e| format!("DLQ 序列化失败: {}", e))?;
        let record = FutureRecord::to(dead_letter_topic)
            .key(&msg.task_id)
            .payload(&payload);

        self.producer
            .send(record, std::time::Duration::from_secs(5))
            .await
            .map_err(|(e, _)| format!("DLQ 发送失败: {}", e))?;

        info!(task_id = %msg.task_id, reason = reason, "任务已转入死信队列: {}", dead_letter_topic);
        Ok(())
    }
}
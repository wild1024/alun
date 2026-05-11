//! 任务生产者 —— 提交任务到 Kafka 并委托给 TaskStorage 持久化

use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::storage::TaskStorage;
use crate::types::*;
use crate::HandlerRegistry;

const MSG_TIMEOUT: Duration = Duration::from_secs(5);

/// 任务生产者
///
/// 将任务提交到 Kafka 主题，并委托 `TaskStorage` 持久化任务日志与队列记录。
/// 不持有任何 SQL 语句或数据库表名——完全由 `TaskStorage` 实现方控制。
pub struct TaskProducer {
    /// Kafka 生产者
    producer: FutureProducer,
    /// 任务持久化接口
    storage: Arc<dyn TaskStorage>,
    /// 处理器注册中心（用于获取 topic 等配置）
    registry: HandlerRegistry,
}

impl TaskProducer {
    /// 创建任务生产者
    ///
    /// - `brokers`: Kafka broker 地址
    /// - `storage`: 由业务方实现的持久化接口
    /// - `registry`: 已注册 handler 的注册中心
    ///
    /// 返回 `Err` 而非 panic，便于外部处理 Kafka 连接失败。
    pub fn new(
        brokers: &str,
        storage: Arc<dyn TaskStorage>,
        registry: HandlerRegistry,
    ) -> Result<Self, String> {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("message.timeout.ms", "5000")
            .create()
            .map_err(|e| format!("Kafka Producer 创建失败: {}", e))?;
        Ok(Self {
            producer,
            storage,
            registry,
        })
    }

    /// 提交任务
    ///
    /// 1. 生成 task_id
    /// 2. 委托 storage 持久化任务日志与队列记录
    /// 3. 发送 Kafka 消息
    pub async fn submit(&self, params: SubmitTaskParams) -> Result<String, String> {
        let config = self
            .registry
            .get_config(params.task_type)
            .ok_or_else(|| format!("未注册的 task_type: {}", params.task_type))?;

        let task_id = uuid::Uuid::new_v4().to_string();
        let priority = params.priority.unwrap_or(config.priority).to_i16();
        let now = chrono::Utc::now().to_rfc3339();

        self.storage
            .save_task_log(&task_id, params.task_type, priority, &config, &params)
            .await
            .map_err(|e| format!("持久化 task_logs 失败: {}", e))?;

        self.storage
            .save_task_queue(&task_id, &config.topic, priority)
            .await
            .map_err(|e| format!("持久化 task_queue 失败: {}", e))?;

        let msg = TaskMessage {
            task_id: task_id.clone(),
            task_type: params.task_type,
            payload: params.payload.clone(),
            priority,
            user_id: params.user_id,
            resource_id: params.resource_id,
            resource_type: params.resource_type.map(|r| r.to_i16()),
            submitted_at: now,
        };

        let payload = serde_json::to_vec(&msg).map_err(|e| format!("序列化失败: {}", e))?;
        let record = FutureRecord::to(&config.topic)
            .key(&task_id)
            .payload(&payload);

        self.producer
            .send(record, MSG_TIMEOUT)
            .await
            .map_err(|(e, _)| format!("Kafka 发送失败: {}", e))?;

        info!(task_id = %task_id, task_type = params.task_type, "任务已提交");
        Ok(task_id)
    }

    /// 批量提交任务
    ///
    /// 逐个提交，部分失败不影响其他任务。
    /// 返回 `(成功数, 失败详情列表)`。
    pub async fn submit_batch(&self, params: SubmitBatchParams) -> (usize, Vec<(usize, String)>) {
        let mut succeeded = 0usize;
        let mut failures = Vec::new();

        for (idx, task) in params.tasks.into_iter().enumerate() {
            match self.submit(task).await {
                Ok(_) => succeeded += 1,
                Err(e) => {
                    warn!(index = idx, error = %e, "批量提交任务失败");
                    failures.push((idx, e));
                }
            }
        }

        info!(succeeded = succeeded, failed = failures.len(), "批量提交完成");
        (succeeded, failures)
    }

    /// 发送消息到死信队列
    ///
    /// 将失败的 TaskMessage 转发到 dead_letter_topic，同时委托 storage 更新状态为 DeadLetter。
    pub async fn send_to_dlq(
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
            .send(record, MSG_TIMEOUT)
            .await
            .map_err(|(e, _)| format!("DLQ 发送失败: {}", e))?;

        let _ = self
            .storage
            .update_task_status(&msg.task_id, TaskStatus::DeadLetter)
            .await;

        info!(task_id = %msg.task_id, reason = reason, "任务已转入死信队列");
        Ok(())
    }
}
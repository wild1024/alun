use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;
use serde::Serialize;
use std::time::Duration;
use tracing::{info, error};

/// Kafka 异步生产者（基于 rdkafka FutureProducer）
///
/// 支持 JSON 序列化消息和原始字节消息，5 秒超时。
pub struct KafkaProducer {
    /// 底层 rdkafka FutureProducer
    inner: FutureProducer,
}

impl KafkaProducer {
    /// 创建 Kafka 生产者（默认配置，5 秒消息超时）
    pub fn new(brokers: &str) -> Self {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("message.timeout.ms", "5000")
            .create()
            .expect("Kafka Producer 创建失败");

        info!("Kafka Producer 已连接: {}", brokers);
        Self { inner: producer }
    }

    /// 创建 Kafka 生产者（自定义配置）
    pub fn with_config(brokers: &str, configs: &[(&str, &str)]) -> Self {
        let mut cfg = ClientConfig::new();
        cfg.set("bootstrap.servers", brokers);

        for (key, value) in configs {
            cfg.set(*key, *value);
        }

        let producer: FutureProducer = cfg.create().expect("Kafka Producer 创建失败");
        info!("Kafka Producer 已连接: {}", brokers);
        Self { inner: producer }
    }

    /// 发送 JSON 序列化消息
    ///
    /// - `topic`: Kafka 主题
    /// - `key`: 分区键（用于同一 key 的消息落到同一分区）
    /// - `payload`: 需实现 `Serialize + Debug` 的消息体
    ///
    /// 发送失败返回 `KafkaError::Send`。
    pub async fn send<T: Serialize + std::fmt::Debug>(
        &self,
        topic: &str,
        key: &str,
        payload: &T,
    ) -> Result<(), KafkaError> {
        let value = serde_json::to_string(payload).map_err(|e| KafkaError::Serialize(e.to_string()))?;

        let record = FutureRecord::to(topic).key(key).payload(&value);

        self.inner
            .send(record, Duration::from_secs(5))
            .await
            .map_err(|(e, _)| {
                error!("Kafka 发送失败 topic={}, key={}: {}", topic, key, e);
                KafkaError::Send(e.to_string())
            })?;

        info!("Kafka 消息已发送 topic={}, key={}", topic, key);
        Ok(())
    }

    /// 发送原始字节消息
    ///
    /// 适用于 Protobuf 等二进制序列化格式。
    pub async fn send_bytes(&self, topic: &str, key: &str, payload: &[u8]) -> Result<(), KafkaError> {
        let record = FutureRecord::to(topic).key(key).payload(payload);

        self.inner
            .send(record, Duration::from_secs(5))
            .await
            .map_err(|(e, _)| {
                error!("Kafka 发送失败 topic={}, key={}: {}", topic, key, e);
                KafkaError::Send(e.to_string())
            })?;

        info!("Kafka 消息已发送 topic={}, key={}", topic, key);
        Ok(())
    }
}

/// Kafka 错误类型
#[derive(Debug, thiserror::Error)]
pub enum KafkaError {
    /// JSON 序列化失败
    #[error("序列化失败: {0}")]
    Serialize(String),

    /// 消息发送失败
    #[error("发送失败: {0}")]
    Send(String),

    /// 消息消费失败
    #[error("消费失败: {0}")]
    Consume(String),
}

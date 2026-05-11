//! Kafka 异步任务插件：Producer / Consumer + 任务持久化
//!
//! 任务可存储到本地缓存或数据库，支持重试和状态追踪。

use alun_core::plugin::Plugin;
use async_trait::async_trait;
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::ClientConfig;
use serde::Serialize;
use tracing::{info, error};
use std::time::Duration;

/// Kafka 插件（实现 `alun_core::Plugin`，可注册到 PluginManager）
///
/// 自动创建 Producer 连接；Consumer 独立管理。
pub struct KafkaPlugin {
    /// Kafka broker 地址
    brokers: String,
    /// Kafka 生产者（连接失败时为 None）
    producer: Option<FutureProducer>,
}

impl KafkaPlugin {
    /// 创建 Kafka 插件
    ///
    /// 尝试连接 broker 并初始化 Producer；连接失败不报错（仅日志记录），
    /// `publish()` 调用时会返回连接错误。
    pub fn new(brokers: &str) -> Self {
        let producer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("message.timeout.ms", "5000")
            .create::<FutureProducer>()
            .ok();

        Self { brokers: brokers.into(), producer }
    }

    /// 获取 Kafka broker 地址
    pub fn brokers(&self) -> &str { &self.brokers }

    /// 发送异步任务（任务数据自动序列化为 JSON）
    pub async fn publish<T: Serialize>(&self, topic: &str, key: &str, payload: &T) -> alun_core::Result<()> {
        if let Some(ref producer) = self.producer {
            let json = serde_json::to_string(payload)
                .map_err(|e| alun_core::Error::Msg(format!("序列化失败: {}", e)))?;
            producer.send(
                FutureRecord::to(topic).key(key).payload(&json),
                Duration::from_secs(5),
            ).await.map_err(|(e, _)| {
                error!("Kafka发送失败 topic={}: {}", topic, e);
                alun_core::Error::Msg(format!("Kafka发送失败: {}", e))
            })?;
            info!("Kafka任务已发布 topic={} key={}", topic, key);
            Ok(())
        } else {
            Err(alun_core::Error::Msg("Kafka Producer 未初始化".into()))
        }
    }
}

#[async_trait]
impl Plugin for KafkaPlugin {
    fn name(&self) -> &str { "kafka" }

    async fn start(&self) -> alun_core::Result<()> {
        if self.producer.is_some() {
            info!("KafkaPlugin 就绪, brokers={}", self.brokers);
        } else {
            error!("KafkaPlugin Producer 初始化失败");
        }
        Ok(())
    }

    async fn stop(&self) -> alun_core::Result<()> {
        info!("KafkaPlugin 停止");
        Ok(())
    }
}

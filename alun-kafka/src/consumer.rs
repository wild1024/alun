use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::ClientConfig;
use tracing::info;

/// Kafka 流式消费者（基于 rdkafka StreamConsumer）
///
/// 支持订阅多个 topic，自动 commit offset，可通过 `with_config()` 传入自定义配置。
pub struct KafkaConsumer {
    /// 底层 rdkafka StreamConsumer
    inner: StreamConsumer,
}

impl KafkaConsumer {
    /// 创建 Kafka 消费者（默认配置）
    ///
    /// - `brokers`: Kafka 集群地址（如 `localhost:9092`）
    /// - `group_id`: 消费组 ID
    /// - `topics`: 订阅的主题列表
    pub fn new(brokers: &str, group_id: &str, topics: &[&str]) -> Self {
        let consumer: StreamConsumer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("group.id", group_id)
            .set("enable.auto.commit", "true")
            .set("auto.offset.reset", "earliest")
            .create()
            .expect("Kafka Consumer 创建失败");

        consumer
            .subscribe(topics)
            .expect("Kafka 订阅主题失败");

        info!("Kafka Consumer 已连接: {}, group={}, topics={:?}", brokers, group_id, topics);
        Self { inner: consumer }
    }

    /// 创建 Kafka 消费者（自定义配置）
    ///
    /// - `configs`: 额外的 `(key, value)` 配置项（如 `"session.timeout.ms"`, `"30000"`）
    pub fn with_config(
        brokers: &str,
        group_id: &str,
        topics: &[&str],
        configs: &[(&str, &str)],
    ) -> Self {
        let mut cfg = ClientConfig::new();
        cfg.set("bootstrap.servers", brokers)
           .set("group.id", group_id);

        for (key, value) in configs {
            cfg.set(*key, *value);
        }

        let consumer: StreamConsumer = cfg.create().expect("Kafka Consumer 创建失败");

        consumer
            .subscribe(topics)
            .expect("Kafka 订阅主题失败");

        info!("Kafka Consumer 已连接: {}, group={}, topics={:?}", brokers, group_id, topics);
        Self { inner: consumer }
    }

    /// 获取底层 rdkafka StreamConsumer 引用（用于流式消费）
    pub fn inner(&self) -> &StreamConsumer { &self.inner }
}

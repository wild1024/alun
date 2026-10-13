---
id: reference.crates.kafka
title: `alun-kafka` — Kafka 生产者/消费者
summary: `alun-kafka` — Kafka 生产者/消费者——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [kafka, reference, rdkafka]
status: active
owner: alun-core-team
source_of_truth: [alun-kafka/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-kafka` — Kafka 生产者/消费者

> 本文件是 [`crates.md`](../crates.md) 的 `kafka` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。



## `alun-kafka`

| 导出项             | 说明                                              |
| --------------- | ----------------------------------------------- |
| `KafkaProducer` | Kafka 生产者（基于 rdkafka）：`new(brokers)`、`with_config(...)`、`send(topic, key, &payload)`、`send_bytes(...)` |
| `KafkaConsumer` | Kafka 消费者：`new(brokers, group_id, &[topics])`、`with_config(...)`、`inner() -> &rdkafka::consumer::StreamConsumer` |
| `KafkaPlugin`   | Kafka 插件（管理连接生命周期）：`new(brokers)`、`brokers()`、`publish(topic, key, &payload)` |

> `KafkaError` 定义在私有模块 `producer` 中且未再导出，因此 `send` / `send_bytes` 的返回错误类型在 crate 外**无法书写**（不要 `match` 它）；`KafkaPlugin::publish` 返回的是 `alun_core::Result<()>`。

## 使用示例

```rust,ignore
// ── Kafka 生产者 ──
use alun_kafka::{KafkaProducer, KafkaPlugin};

let producer = KafkaProducer::new("localhost:9092");   // 注意：new 不返回 Result
producer.send("order-events", "order.created", &order_json).await?;

// Kafka 插件管理连接生命周期
let kafka_plugin = KafkaPlugin::new("localhost:9092");
kafka_plugin.publish("order-events", "order.paid", &order_json).await?;

// ── Kafka 消费者（消费循环自行基于 inner() 实现） ──
use alun_kafka::KafkaConsumer;
use rdkafka::consumer::StreamConsumer;

let consumer = KafkaConsumer::new("localhost:9092", "order-group", &["order-events"]);
let stream: &StreamConsumer = consumer.inner();
// 在循环中 stream.recv() 拉取消息，再交给业务处理
```

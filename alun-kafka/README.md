# alun-kafka

Alun 的 Kafka 集成：Producer 与 Consumer，统一 API。

> 门面侧 `features = ["kafka"]`，类型经 `alun::alun_kafka::{KafkaProducer, KafkaConsumer, KafkaPlugin}` 取用。

## 主要 API

Producer / Consumer / Plugin 三个模块是私有的，公开类型在 crate 根再导出：

- `KafkaProducer` —— `new(brokers)`、`with_config(brokers, &[(key, value)])`、
  `async send<T: Serialize + Debug>(topic, key, &payload)`、`async send_bytes(topic, key, &[u8])`
- `KafkaConsumer` —— `new(brokers, group_id, &[topics])`、
  `with_config(brokers, group_id, &[topics], &[(k, v)])`、
  `inner() -> &rdkafka::consumer::StreamConsumer`（自己驱动这条流）
- `KafkaPlugin` —— `new(brokers)`、`brokers()`、`async publish<T: Serialize>(topic, key, &payload) -> alun_core::Result<()>`；
  实现 `alun_core::Plugin`，名字为 `"kafka"`

`KafkaProducer::send` 的错误类型来自私有模块，所以匹配消息（`to_string()`）而不是具名错误枚举。

## 示例

```rust,ignore
use alun_kafka::{KafkaConsumer, KafkaProducer};

let producer = KafkaProducer::new("localhost:9092");
producer.send("orders", "order-1", &payload).await?;
producer.send_bytes("orders", "order-2", b"raw").await?;

let consumer = KafkaConsumer::new("localhost:9092", "order-group", &["orders"]);
let stream = consumer.inner();   // rdkafka StreamConsumer —— 在自己的循环里 poll
```

注册成插件（这样 `App::new().plugin(..)` 会一并管理它的生命周期）：

```rust,ignore
use alun::prelude::*;
use alun_kafka::KafkaPlugin;

let app = App::new()?.plugin(KafkaPlugin::new("localhost:9092"));
```

## Feature

本 crate 不声明任何 cargo feature，始终依赖 `rdkafka`。

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)
- 速查参考：`skill-alun/references/task.md`（Kafka 一节）

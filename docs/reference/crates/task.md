---
id: reference.crates.task
title: `alun-task` — 异步任务框架
summary: `alun-task` — 异步任务框架——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [task, reference, kafka, retry, dlq]
status: active
owner: alun-core-team
source_of_truth: [alun-task/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-task` — 异步任务框架

> 本文件是 [`crates.md`](../crates.md) 的 `task` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun-task/src/`

基于 Kafka 消息队列的异步任务分发与处理框架。核心设计理念：**插件零 SQL 依赖**——所有持久化通过 `TaskStorage` trait 委托给业务方，不与任何表结构耦合。

**结构概览**：

| 模块         | 职责                                                                                |
| ---------- | --------------------------------------------------------------------------------- |
| `storage`  | `TaskStorage` trait（泛型持久化接口）+ `RetryableTask`（重试数据结构）                             |
| `handler`  | `TaskHandler` trait（业务处理器接口）                                                      |
| `registry` | `HandlerRegistry`（按 task\_type 索引 handler）+ `from_discovered()` 宏自动发现             |
| `producer` | `TaskProducer`（Kafka 消息发送 + storage 持久化）                                          |
| `worker`   | `TaskWorker`（Kafka 消费循环 + handler 分发执行 + DLQ）                                     |
| `retry`    | `RetryScanner`（定期扫描待重试任务 + 重推 Kafka）                                              |
| `metrics`  | `TaskMetrics`（原子计数：total / completed / failed）                                    |
| `plugin`   | `TaskPlugin`（实现 `alun_core::Plugin`，统一启停）                                         |
| `types`    | 枚举和配置：`TaskStatus`、`TaskPriority`、`RetryStrategy`、`TaskConfig`、`TaskWorkerConfig` |

## 关键 Trait

### `TaskStorage` （[storage.rs](../../../alun-task/src/storage.rs)）

**插件不持有 SQL 语句、表名、字段名**。所有持久化逻辑由业务方通过实现此 trait 控制。共 8 个方法：

| 方法                                                            | 调用时机              | 说明                                             |
| ------------------------------------------------------------- | ----------------- | ---------------------------------------------- |
| `save_task_log(task_id, task_type, priority, config, params)` | 提交任务              | 持久化任务日志（完整入参）                                  |
| `save_task_queue(task_id, topic, priority)`                   | 提交任务              | 持久化队列记录                                        |
| `update_task_status(task_id, status)`                         | 状态变更              | Pending→Processing→Completed/Failed/DeadLetter |
| `get_retry_count(task_id)`                                    | 失败时               | 获取当前重试次数                                       |
| `update_retry(task_id, retry_count)`                          | 重试时               | 递增 retry\_count                                |
| `save_task_result(task_id, output)`                           | 执行完成              | 保存成功/失败输出                                      |
| `log_execution(task_id, status, error, elapsed_ms)`           | 每次执行              | 记录执行日志（含重试）                                    |
| `scan_retryable_tasks(task_types, limit)`                     | RetryScanner 定期调用 | 查询可重试任务列表                                      |

所有方法返回 `Result<(), String>`，失败原因通过 `Err(String)` 传递。业务方可自由选择后端（PG / MySQL / MongoDB / 文件等）。

```rust,ignore
#[async_trait]
pub trait TaskStorage: Send + Sync {
    async fn save_task_log(&self, task_id: &str, task_type: i16, priority: i16,
        config: &TaskConfig, params: &SubmitTaskParams) -> Result<(), String>;
    async fn save_task_queue(&self, task_id: &str, topic: &str, priority: i16) -> Result<(), String>;
    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<(), String>;
    async fn get_retry_count(&self, task_id: &str) -> Result<i64, String>;
    async fn update_retry(&self, task_id: &str, retry_count: i64) -> Result<(), String>;
    async fn save_task_result(&self, task_id: &str, output: &serde_json::Value) -> Result<(), String>;
    async fn log_execution(&self, task_id: &str, status: TaskStatus, error: Option<&str>, elapsed_ms: i64) -> Result<(), String>;
    async fn scan_retryable_tasks(&self, task_types: &[i16], limit: usize) -> Result<Vec<RetryableTask>, String>;
}
```

### `TaskHandler` （[handler.rs](../../../alun-task/src/handler.rs)）

业务处理器特质——每种任务类型对应一个实现。

```rust,ignore
#[async_trait]
pub trait TaskHandler: Send + Sync {
    fn task_type(&self) -> i16;
    async fn execute(&self, payload: Value) -> Result<Value, String>;
}
```

## 关键结构体

### `RetryableTask`

扫描到的可重试任务——由 `TaskStorage::scan_retryable_tasks()` 返回。

| 字段            | 类型       | 说明      |
| ------------- | -------- | ------- |
| `task_id`     | `String` | 任务 ID   |
| `task_type`   | `i16`    | 任务类型    |
| `retry_count` | `i64`    | 当前重试次数  |
| `max_retries` | `i64`    | 最大重试次数  |
| `payload`     | `Value`  | 任务携带的数据 |

### `HandlerRegistry` （[registry.rs](../../../alun-task/src/registry.rs)）

处理器注册中心——按 `task_type` 索引 handler 和 config。

| 方法                          | 说明                                               |
| --------------------------- | ------------------------------------------------ |
| `new()`                     | 创建空注册中心                                          |
| `register(handler, config)` | 手动注册一个 handler                                   |
| `from_discovered()`         | **批量注册编译期发现的 handler**（读取 `TASK_HANDLERS` 分布式切片） |
| `get(task_type)`            | 按 task\_type 获取 handler 和 config                 |
| `get_config(task_type)`     | 按 task\_type 获取配置                                |
| `task_types()`              | 获取所有已注册的 task\_type                              |
| `len()` / `is_empty()`      | 统计信息                                             |

### `TaskProducer` （[producer.rs](../../../alun-task/src/producer.rs)）

任务生产者——提交任务到 Kafka 并委托 storage 持久化。

| 方法                                | 说明                                 |
| --------------------------------- | ---------------------------------- |
| `new(brokers, storage, registry)` | 创建生产者，返回 `Result`                  |
| `submit(params)`                  | 提交单个任务（持久化 + Kafka 发送），返回 task\_id |
| `submit_batch(params)`            | 批量提交，返回 `(成功数, 失败列表)`              |
| `send_to_dlq(msg, topic, reason)` | 发送消息到死信队列                          |

### `TaskWorker` （[worker.rs](../../../alun-task/src/worker.rs)）

任务消费者——Kafka 消费循环 + handler 分发执行。

| 方法                                        | 说明                    |
| ----------------------------------------- | --------------------- |
| `new(config, storage, registry, metrics)` | 创建 worker，返回 `Result` |
| `run(topics)`                             | 订阅 topic 并启动消费循环      |
| `stop()`                                  | 发送停止信号                |

内部逻辑：

1. 消费 Kafka 消息 → 反序列化 `TaskMessage`
2. `check_message_age()` 过滤超时消息
3. 查找 `HandlerRegistry` 找到对应 handler
4. `std::time::timeout` 限时执行
5. 成功 → 更新 status + 保存 result + 记执行日志
6. 失败 → 检查重试次数：
   - 未超限 → `update_retry()`（等待 RetryScanner 重推）
   - 超限 → 检查 `dead_letter_topic`：
     - 有 DLQ → 推入死信队列 + 状态 DeadLetter
     - 无 DLQ → 状态 Failed

### `RetryScanner` （[retry.rs](../../../alun-task/src/retry.rs)）

后台定期扫描待重试任务。

| 方法                                                               | 说明                |
| ---------------------------------------------------------------- | ----------------- |
| `new(brokers, storage, registry, interval_secs, max_batch_size)` | 创建扫描器，返回 `Result` |
| `run()`                                                          | 启动扫描循环            |
| `stop()`                                                         | 发送停止信号            |

### `TaskPlugin` （[plugin.rs](../../../alun-task/src/plugin.rs)）

任务插件入口——实现 `alun_core::Plugin` trait。

| 方法                               | 说明                                        |
| -------------------------------- | ----------------------------------------- |
| `new(config, storage, registry)` | 创建插件（配置从 `[task]` section 读取），返回 `Result` |
| `name()`                         | 返回 `"task"`                               |
| `start()`                        | 在后台 tokio task 中启动 Worker + RetryScanner  |
| `stop()`                         | AtomicBool 信号停止 → join 所有后台任务             |
| `depends_on()`                   | 无依赖                                       |
| `metrics()`                      | 返回 `Arc<TaskMetrics>`                     |
| `topics()`                       | 返回已注册的 topic 列表                           |

### `TaskMetrics` （[metrics.rs](../../../alun-task/src/metrics.rs)）

原子计数器。

| 字段          | 类型          | 说明      |
| ----------- | ----------- | ------- |
| `total`     | `AtomicU64` | 接收的总任务数 |
| `completed` | `AtomicU64` | 成功完成数   |
| `failed`    | `AtomicU64` | 失败数     |

## 枚举与配置

### `TaskStatus` — 任务状态

| 变体           | 值 | 说明          |
| ------------ | - | ----------- |
| `Pending`    | 1 | 等待处理        |
| `Processing` | 2 | 处理中         |
| `Completed`  | 3 | 已完成         |
| `Failed`     | 4 | 失败（等待重试）    |
| `Cancelled`  | 5 | 已取消         |
| `Scheduled`  | 6 | 已调度（定时任务预留） |
| `DeadLetter` | 7 | 死信（超过最大重试）  |

`is_terminal()` 判断是否为终态（Completed / Cancelled / DeadLetter）。

### `RetryStrategy` — 重试策略

| 变体            | 公式                   | 说明   |
| ------------- | -------------------- | ---- |
| `Fixed`       | `base`               | 固定延迟 |
| `Linear`      | `base × (attempt+1)` | 线性增长 |
| `Exponential` | `base × 2^attempt`   | 指数退避 |

### `TaskConfig` — 每种 task\_type 一份配置

| 字段                        | 类型               | 说明                            |
| ------------------------- | ---------------- | ----------------------------- |
| `task_type`               | `i16`            | 任务类型标识                        |
| `priority`                | `TaskPriority`   | 优先级（Low/Normal/High/Critical） |
| `topic`                   | `String`         | Kafka topic 名称                |
| `timeout_seconds`         | `u64`            | 执行超时（秒）                       |
| `max_retries`             | `u32`            | 最大重试次数                        |
| `retry_strategy`          | `RetryStrategy`  | 重试策略                          |
| `retry_delay_seconds`     | `u64`            | 基础重试延迟                        |
| `max_retry_delay_seconds` | `u64`            | 最大重试延迟（上限）                    |
| `description`             | `&'static str`   | 任务描述                          |
| `dead_letter_topic`       | `Option<String>` | 死信队列 topic（None 不启用）          |

### `TaskWorkerConfig` — 运行时配置（支持从 `[task]` section 反序列化）

| 字段                     | 类型       | 默认值                | 说明              |
| ---------------------- | -------- | ------------------ | --------------- |
| `brokers`              | `String` | `localhost:9092`   | Kafka broker 地址 |
| `group_id`             | `String` | `alun-task-worker` | 消费组 ID          |
| `scan_interval_secs`   | `u64`    | 30                 | 重试扫描间隔（秒）       |
| `max_batch_size`       | `usize`  | 100                | 每批次扫描最大任务数      |
| `max_message_age_secs` | `u64`    | 3600               | 消息最大时效（秒）       |
| `auto_create_topics`   | `bool`   | false              | 启动时自动创建 topic   |
| `topic_partitions`     | `i32`    | 1                  | topic 分区数       |
| `topic_replication`    | `i16`    | 1                  | topic 副本数       |

## `#[task_handler]` Proc Macro

位于 `alun-macros/src/task_handler.rs`，由 `alun` 门面 re-export 为 `alun::task_handler`。

**参数一览**：

| 参数                        | 类型                                 | 默认值                  | 必填    |
| ------------------------- | ---------------------------------- | -------------------- | ----- |
| `task_type`               | `i16`                              | —                    | **是** |
| `topic`                   | `&str`                             | `"task_{task_type}"` | 否     |
| `priority`                | `"Normal"/"High"/"Low"/"Critical"` | Normal               | 否     |
| `timeout_seconds`         | `u64`                              | 300                  | 否     |
| `max_retries`             | `u32`                              | 3                    | 否     |
| `retry_strategy`          | `"Fixed"/"Linear"/"Exponential"`   | Linear               | 否     |
| `retry_delay_seconds`     | `u64`                              | 30                   | 否     |
| `max_retry_delay_seconds` | `u64`                              | 300                  | 否     |
| `description`             | `&str`                             | ""                   | 否     |
| `dead_letter_topic`       | `Option<&str>`                     | None                 | 否     |

**编译期行为**：

1. 为结构体生成 `inventory::submit!(TaskHandlerEntry)` 注册项
2. `TASK_HANDLERS` 在链接期自动汇集所有标注了 `#[task_handler]` 的 handler
3. `HandlerRegistry::from_discovered()` 读取切片并一键注册

## 使用示例

```rust,ignore
use alun::alun_task::*;
use async_trait::async_trait;

// ── 1. 定义 Handler（宏自动发现） ──
#[alun::task_handler(
    task_type = 1,
    topic = "export_tasks",
    timeout_seconds = 60,
    max_retries = 3,
    retry_strategy = "Exponential",
    retry_delay_seconds = 10,
    description = "数据导出任务",
    dead_letter_topic = "export_dlq"
)]
struct ExportHandler;

#[async_trait]
impl TaskHandler for ExportHandler {
    fn task_type(&self) -> i16 { 1 }
    async fn execute(&self, payload: Value) -> Result<Value, String> {
        let file_id = payload["file_id"].as_str().unwrap_or("");
        // 执行导出逻辑 ...
        Ok(json!({"url": "https://...", "file_id": file_id}))
    }
}

// ── 2. 实现 TaskStorage（内部通过 db() 全局函数操作数据库，无需持有连接） ──
use alun_db::prelude::{Row, Value as DbValue};

struct DbTaskStorage;

fn text(s: &str) -> DbValue { DbValue::Text(s.to_string()) }

#[async_trait]
impl TaskStorage for DbTaskStorage {
    async fn save_task_log(&self, task_id: &str, task_type: i16, priority: i16,
        config: &TaskConfig, params: &SubmitTaskParams) -> Result<(), String>
    {
        let mut row = Row::new().with_entity("task_logs", "id");
        row.insert("id", text(&Sid::uuid()));
        row.insert("task_id", text(task_id));
        row.insert("task_type", DbValue::I16(task_type));
        row.insert("priority", DbValue::I16(priority));
        row.insert("topic", text(&config.topic));
        row.insert("payload", text(&params.payload.to_string()));
        db().insert_row(&row).await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn save_task_queue(&self, task_id: &str, topic: &str, priority: i16) -> Result<(), String> {
        let mut row = Row::new().with_entity("task_queue", "id");
        row.insert("id", text(&Sid::uuid()));
        row.insert("task_id", text(task_id));
        row.insert("topic", text(topic));
        row.insert("priority", DbValue::I16(priority));
        db().insert_row(&row).await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<(), String> {
        db().execute_with_params(
            "UPDATE task_logs SET status = ? WHERE task_id = ?",
            vec![DbValue::I16(status.to_i16()), text(task_id)],
        ).await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn get_retry_count(&self, task_id: &str) -> Result<i64, String> {
        let row = db().find_first_with_params(
            "SELECT retry_count FROM task_logs WHERE task_id = ?",
            vec![text(task_id)],
        ).await.map_err(|e| e.to_string())?;
        Ok(row.and_then(|r| r.get_typed::<i64>("retry_count").ok()).unwrap_or(0))
    }

    async fn update_retry(&self, task_id: &str, retry_count: i64) -> Result<(), String> {
        db().execute_with_params(
            "UPDATE task_logs SET retry_count = ?, status = 1 WHERE task_id = ?",
            vec![DbValue::I64(retry_count), text(task_id)],
        ).await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn save_task_result(&self, task_id: &str, output: &serde_json::Value) -> Result<(), String> {
        let mut row = Row::new().with_entity("task_results", "id");
        row.insert("id", text(&Sid::uuid()));
        row.insert("task_id", text(task_id));
        row.insert("output", text(&output.to_string()));
        db().insert_row(&row).await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn log_execution(&self, task_id: &str, status: TaskStatus, error: Option<&str>,
        elapsed_ms: i64) -> Result<(), String>
    {
        let mut row = Row::new().with_entity("task_execution_logs", "id");
        row.insert("id", text(&Sid::uuid()));
        row.insert("task_id", text(task_id));
        row.insert("status", DbValue::I16(status.to_i16()));
        row.insert("error", text(error.unwrap_or("")));
        row.insert("elapsed_ms", DbValue::I64(elapsed_ms));
        db().insert_row(&row).await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn scan_retryable_tasks(&self, task_types: &[i16], limit: usize) -> Result<Vec<RetryableTask>, String> {
        let types = task_types.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT task_id, task_type, retry_count, max_retries, payload FROM task_logs \
             WHERE status = 4 AND task_type IN ({}) AND retry_count < max_retries ORDER BY priority DESC LIMIT {}",
            types, limit
        );
        let rows = db().find(&sql).await.map_err(|e| e.to_string())?;
        rows.iter().map(|r| {
            Ok(RetryableTask {
                task_id: r.get_typed::<String>("task_id").unwrap_or_default(),
                task_type: r.get_typed::<i16>("task_type").unwrap_or(0),
                retry_count: r.get_typed::<i64>("retry_count").unwrap_or(0),
                max_retries: r.get_typed::<i64>("max_retries").unwrap_or(0),
                payload: r.get_typed::<String>("payload")
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or(serde_json::Value::Null),
            })
        }).collect()
    }
}

// ── 3. 启动时创建插件（TaskStorage 无字段，Arc::new 即可） ──
#[tokio::main]
async fn main() {
    App::new().unwrap();  // 初始化全局资源（DB/Cache/Config）

    let task_cfg: TaskWorkerConfig = cfg().custom.get("task")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    App::new().unwrap()
        .plugin(TaskPlugin::new(
            task_cfg,
            Arc::new(DbTaskStorage),
            HandlerRegistry::new().from_discovered(),
        ).unwrap())
        .scan().start().await.unwrap();
}

// ── 4. 在 Handler 中提交任务（直接 Arc::new，无需传 db） ──
#[alun::post("/api/export")]
async fn export() -> Result<Res<String>, ApiError> {
    let task_cfg: TaskWorkerConfig = cfg().custom.get("task")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let producer = TaskProducer::new(
        &task_cfg.brokers,
        Arc::new(DbTaskStorage),
        HandlerRegistry::new().from_discovered(),
    ).map_err(|e| ApiError::internal(e))?;

    let task_id = producer.submit(SubmitTaskParams {
        task_type: 1,
        payload: json!({"file_id": "f1", "format": "xlsx"}),
        priority: Some(TaskPriority::High),
        user_id: Some("u1".into()),
        resource_id: None, resource_type: None,
    }).await.map_err(|e| ApiError::internal(e))?;

    Ok(Res::ok_with_msg(task_id, "任务已提交"))
}

// ── 5. 批量提交 ──
let batch = SubmitBatchParams {
    tasks: (0..10).map(|i| SubmitTaskParams {
        task_type: 1,
        payload: json!({"file_id": format!("f{}", i)}),
        priority: None, user_id: None, resource_id: None, resource_type: None,
    }).collect(),
};
let (succeeded, failures) = producer.submit_batch(batch).await;

// ── 6. 配置文件示例 ──
// [task]
// brokers = "kafka.internal:9092"
// group_id = "my-app-task-worker"
// scan_interval_secs = 30
// max_batch_size = 100
// max_message_age_secs = 3600
// auto_create_topics = false
```


***

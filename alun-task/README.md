# alun-task

Alun 异步任务框架：任务提交、处理、**重试与死信队列**、生命周期管理。

> 门面侧 `features = ["task"]`（任务模型，零外部构建链）；要用 Kafka worker 再加
> `task-kafka`（需 CMake + C 工具链编译 librdkafka）。

## 主要 API

- `trait TaskHandler` —— `task_type() -> i16`、
  `async execute(payload: serde_json::Value) -> Result<serde_json::Value, String>`
- `trait TaskStorage` —— 持久化契约：`save_task_log`、`save_task_queue`、`update_task_status`、
  `get_retry_count`、`update_retry`、`save_task_result`、`log_execution`、`scan_retryable_tasks`
- `HandlerRegistry` —— `new()`、`register(handler, TaskConfig)`、`from_discovered()`、`get(task_type)`、
  `get_config(task_type)`、`task_types()`、`len()`、`is_empty()`
- `TaskProducer` —— 提交入口：`new(brokers, Arc<dyn TaskStorage>, HandlerRegistry)`、
  `async submit(SubmitTaskParams) -> Result<String, String>`（返回任务 ID）、
  `async submit_batch(SubmitBatchParams)`、`async send_to_dlq(&TaskMessage, topic, reason)`
- `TaskWorker` —— `new(TaskWorkerConfig, storage, registry, Arc<TaskMetrics>, &[topics])`、`async run()`、`stop()`
- `RetryScanner` —— `new(brokers, storage, registry, interval_secs, max_batch_size)`、`async run()`、`stop()`；
  自由函数 `compute_retry_delay(strategy, base, max, attempt)`
- `TaskPlugin` —— `new(TaskWorkerConfig, storage, registry)`、`metrics()`、`topics()`；插件名 `"task"`
- `TaskMetrics` —— 原子计数器（`total` / `pending` / `running` / `completed` / `failed` / `cancelled` /
  `retried`），另有 `new()` 与 `snapshot()`

### 类型

`TaskStatus`（`Pending` / `Processing` / `Completed` / `Failed` / `Cancelled` / `Scheduled` / `DeadLetter`）、
`TaskPriority`（`Low` / `Normal` / `High` / `Critical`）、`RetryStrategy`（`Fixed` / `Linear` / `Exponential`）、
`TaskConfig`、`TaskWorkerConfig`、`ResourceType`、`SubmitTaskParams`、`SubmitBatchParams`、`TaskMessage`、
`RetryableTask`。

### 编译期注册

`TaskHandlerEntry` 与编译期注册表。`#[task_handler(...)]` 属性宏（来自 `alun-macros`，也再导出为
`alun::task_handler`）会把处理器记进去，`HandlerRegistry::from_discovered()` 一次性批量注册。

宏参数：`task_type`（必填）、`topic`、`priority`、`timeout_seconds`、`max_retries`、
`retry_strategy`、`retry_delay_seconds`、`max_retry_delay_seconds`、`description`、`dead_letter_topic`。

## 示例

定义处理器：

```rust,ignore
use alun_macros::task_handler;
use alun_task::TaskHandler;

#[task_handler(task_type = 1, topic = "export", timeout_seconds = 60, max_retries = 3)]
struct ExportHandler;

#[async_trait::async_trait]
impl TaskHandler for ExportHandler {
    fn task_type(&self) -> i16 { 1 }

    async fn execute(&self, payload: serde_json::Value) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "order_id": payload["order_id"] }))
    }
}
```

提交与消费：

```rust,ignore
use alun_task::{HandlerRegistry, SubmitTaskParams, TaskProducer, TaskWorker, TaskWorkerConfig};
use std::sync::Arc;

let mut registry = HandlerRegistry::new();
registry.from_discovered();

let producer = TaskProducer::new("localhost:9092", storage.clone(), registry.clone())?;
let task_id = producer
    .submit(SubmitTaskParams {
        task_type: 1,
        payload: serde_json::json!({ "order_id": "1001" }),
        priority: None,
        user_id: None,
        resource_id: None,
        resource_type: None,
    })
    .await?;

let worker = TaskWorker::new(
    TaskWorkerConfig { brokers: "localhost:9092".into(), ..Default::default() },
    storage,
    registry,
    Arc::new(alun_task::TaskMetrics::new()),
    &["export".to_string()],
)?;
worker.run().await?;
```

任务状态流转：`PENDING → PROCESSING → COMPLETED / FAILED →（重试）→ DEAD_LETTER`

## Feature

本 crate 不声明 cargo feature；`task` / `task-kafka` 是**门面 `alun`** 的 feature。

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)
- 速查参考：`skill-alun/references/task.md`

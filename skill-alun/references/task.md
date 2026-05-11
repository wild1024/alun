# Async Task Framework (`alun-task`)

Requires `features = ["task"]`. A **Kafka-driven distributed async task engine** with zero SQL in the framework itself — the business code provides persistence via `TaskStorage`.

## Architecture

- **TaskHandler trait**: Business logic for each task type
- **TaskStorage trait**: 8-method generic persistence interface. Framework owns zero SQL.
- **TaskProducer**: Sends tasks to Kafka + persists via storage
- **TaskWorker**: Consumes Kafka messages + dispatches to handlers
- **RetryScanner**: Periodically scans for retryable tasks and re-pushes to Kafka
- **TaskPlugin**: Implements `Plugin` trait for unified start/stop
- **`#[task_handler]` macro**: Compile-time auto-discovery via linkme distributed slice

## Define a Task Handler

```rust
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
        // ... execution logic ...
        Ok(json!({"url": "https://...", "file_id": file_id}))
    }
}
```

## Implement TaskStorage

```rust
struct DbTaskStorage;

#[async_trait]
impl TaskStorage for DbTaskStorage {
    async fn save_task_log(&self, task_id: &str, task_type: i16, priority: i16,
        config: &TaskConfig, params: &SubmitTaskParams) -> Result<(), String> {
        db().execute("INSERT INTO task_logs (task_id, task_type, ...) VALUES ($1, $2, ...)",
            &[task_id, &task_type.to_string(), ...])
            .await.map(|_| ()).map_err(|e| e.to_string())
    }
    // ... implement remaining 7 methods ...
}
```

## Startup Registration

```rust
#[tokio::main]
async fn main() {
    App::new().unwrap();  // Initialize global resources first

    let task_cfg: TaskWorkerConfig = cfg().custom.get("task")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    App::new().unwrap()
        .plugin(TaskPlugin::new(
            task_cfg,
            Arc::new(DbTaskStorage),
            HandlerRegistry::new().from_discovered(),
        ).unwrap())
        .scan()
        .start()
        .await
        .unwrap();
}
```

## Submit Tasks from Handlers

```rust
let producer = TaskProducer::new(
    &brokers,
    Arc::new(DbTaskStorage),
    HandlerRegistry::new().from_discovered(),
).map_err(|e| ApiError::internal(e))?;

let task_id = producer.submit(SubmitTaskParams {
    task_type: 1,
    payload: json!({"file_id": "f1"}),
    priority: Some(TaskPriority::High),
    user_id: Some("u1".into()),
    resource_id: None,
    resource_type: None,
}).await.map_err(|e| ApiError::internal(e))?;
```

## Retry Strategies

| Strategy | Formula | Note |
|----------|---------|------|
| **Fixed** | `base` | Constant delay |
| **Linear** | `base × (attempt + 1)` | Linear backoff |
| **Exponential** | `base × 2^attempt` | Capped at `max_retry_delay_seconds` |

## Task Configuration

```toml
[task]
brokers = "localhost:9092"
group_id = "my-app-task-worker"
scan_interval_secs = 30
max_batch_size = 100
max_message_age_secs = 3600
auto_create_topics = false
topic_partitions = 1
topic_replication = 1
```

## Task Statuses

`TaskStatus::Pending=1`, `Processing=2`, `Completed=3`, `Failed=4`, `Cancelled=5`, `Scheduled=6`, `DeadLetter=7`

---

## Kafka Integration (`alun-kafka`)

Requires `features = ["kafka"]`:

```rust
use alun_kafka::{KafkaProducer, KafkaConsumer, KafkaPlugin};

let producer = KafkaProducer::new("localhost:9092")?;
producer.send("order-events", "order.created", &json_value).await?;

let consumer = KafkaConsumer::new("localhost:9092", "group-id", &["topic"]);
consumer.start(|msg| async move {
    tracing::info!("Received: {:?}", msg);
    Ok(())
}).await?;

let plugin = KafkaPlugin::from_config(&config);
```

---

## File System (`alun-fs`)

Requires `features = ["fs"]`:

```rust
use alun_fs::{LocalFs, FsPlugin, FileMeta};

let fs = LocalFs::new("uploads");

// Write
let meta = FileMeta { name: "report.pdf".into(), size: 102400, content_type: "application/pdf".into() };
fs.store("reports/2024/report.pdf", &data).await?;

// Read
let file = fs.read("reports/2024/report.pdf").await?;

// Delete
fs.delete("temp/old.txt").await?;

// Existence check
let exists = fs.exists("path/to/file").await?;
```
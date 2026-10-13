# Async Task Framework (`alun-task`)

Requires `features = ["task-kafka"]`（完整能力）或 `features = ["task"]`（仅任务模型）。A
**Kafka-driven distributed async task engine** with zero SQL in the framework itself — the business code
provides persistence via `TaskStorage`.

> **feature 分层（0.3.0 起）**：`alun-task` 已拆成两个 feature，**`task` 不再自带 Kafka**：
>
> | feature | 门面 | 内容 | 外部依赖 |
> |---|---|---|---|
> | `task` | `alun = { features = ["task"] }` | `#[task_handler]` 收集、存储抽象、任务类型与生命周期、重试延迟策略 | **无** |
> | `task-kafka` | `alun = { features = ["task-kafka"] }` | 上面全部 + Kafka 生产/消费 worker、RetryScanner、`TaskPlugin` | 传递 `rdkafka` |
>
> `task-kafka` 隐含 `task`，通常直接写 `features = ["task-kafka"]`。**为什么默认不带 Kafka**：
> `rdkafka` 需要 CMake + C 工具链构建 librdkafka，在纯 Windows 工具链下无法构建，因此不能进默认。
> 若你只需要任务模型（自己实现 `TaskStorage` 落库、业务侧有现成队列），写 `features = ["task"]`
> 即可避开 C 工具链。
>
> `kafka` feature（`= ["dep:alun-kafka"]`）是**另一件事**：它是本章末尾那套通用的
> `KafkaProducer` / `KafkaConsumer` / `KafkaPlugin` 门面，与任务框架无关。
>
> 门面路径：`alun::alun_task::…`（`alun/src/lib.rs:1168-1169`）、
> `alun::alun_kafka::…`（`:1165-1166`）、`alun::alun_fs::…`（`:1171-1172`），
> 因此**不需要**在 `Cargo.toml` 里单独声明 `alun-task` / `alun-kafka` / `alun-fs`。
>
> **关于 linkme**：0.3 起收集机制已整体换成 `inventory`，`linkme` 不再是任何宏展开的
> 目标 crate。`#[task_handler]` 生成的是 `::alun::inventory::submit! { … }`
> （`alun-macros/src/task_handler.rs:164`），路由 / 权限 / 免鉴权宏同理
> （`alun-macros/src/route.rs:110/143/169`）。**业务工程不需要**在 `Cargo.toml` 里
> 声明 `linkme` 或 `inventory` —— 两者都由门面 `pub use` 转发。

## Architecture

- **TaskHandler trait**: Business logic for each task type
- **TaskStorage trait**: **8-method** generic persistence interface. Framework owns zero SQL.
- **TaskProducer**: Sends tasks to Kafka + persists via storage
- **TaskWorker**: Consumes Kafka messages + dispatches to handlers
- **RetryScanner**: Periodically scans for retryable tasks and re-pushes to Kafka
- **TaskPlugin**: Implements `Plugin` trait for unified start/stop
- **`#[task_handler]` macro**: 编译期经 `inventory` 自动注册到 handler 收集器

派发链路是**纯 Kafka**：`TaskProducer::submit` → `[task].brokers` 上的 topic →
`TaskWorker`（消费组 `group_id`）→ `HandlerRegistry` 按 `task_type` 找 handler。
框架**没有**“本地直连执行”模式，也不提供进程内队列；跨进程/多实例消费由 Kafka
消费组语义保证（`alun-task/src/producer.rs:58-105`、`alun-task/src/worker.rs:89-107`）。

## 与其它“异步/定时”能力的边界

| 能力 | 形态 | 持久化/重试 | 跨进程 |
|------|------|-------------|--------|
| `TaskPlugin`（`alun-task`） | Kafka 生产/消费 + 重试扫描 + 死信 | 由业务 `TaskStorage` 决定 | 是（Kafka 消费组） |
| `AsyncTaskPlugin`（`alun-plugin`） | 进程内 semaphore 并发池，`submit(fut)` 直接 `tokio::spawn` | 无（失败即丢） | 否 |
| `SchedulerPlugin`（`alun-plugin`） | 进程内 cron 循环，到点调用 runner | 无（不落库、不重试） | 否，且每个实例各自触发 |

三者**互不依赖**：`TaskPlugin::depends_on()` 返回 `&[]`（`alun-task/src/plugin.rs:115-117`），
不会与 `scheduler` / `async-task` 建立顺序。需要“定时 + 可重试 + 分布式”时，应把
SchedulerPlugin 当作触发器，在 runner 里调用 `TaskProducer::submit`，而不是在 runner 里
直接跑重业务。

## Define a Task Handler

```rust,no_run
use alun::prelude::*;
use alun::alun_task::TaskHandler;
use serde_json::{json, Value};

#[alun::task_handler(
    task_type = 1,
    topic = "export_tasks",
    timeout_seconds = 60,
    max_retries = 3,
    retry_strategy = "Exponential",
    retry_delay_seconds = 10,
    max_retry_delay_seconds = 120,
    priority = "High",
    description = "数据导出任务",
    dead_letter_topic = "export_dlq"
)]
struct ExportHandler;

#[async_trait::async_trait]
impl TaskHandler for ExportHandler {
    fn task_type(&self) -> i16 { 1 }

    // 注意：prelude 会把 `Result` 解析成 `alun::Result`（单泛型），
    // 这里必须写全 `std::result::Result<_, String>`
    async fn execute(&self, payload: Value) -> std::result::Result<Value, String> {
        let file_id = payload["file_id"].as_str().unwrap_or("");
        // ... execution logic ...
        Ok(json!({"url": "https://...", "file_id": file_id}))
    }
}
```

`#[task_handler]` 的可选参数与默认值（`alun-macros/src/task_handler.rs:10-19`）：

| 参数 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `task_type` | i16 | **必填** | 任务类型标识，唯一 |
| `topic` | &str | `"task_{task_type}"` | Kafka topic |
| `priority` | `Normal`/`High`/`Low`/`Critical` | `Normal` | 任务优先级 |
| `timeout_seconds` | u64 | `300` | 执行超时（秒），`0` 表示不限时 |
| `max_retries` | u32 | `3` | 最大重试次数 |
| `retry_strategy` | 字符串字面量 `"Fixed"`/`"Linear"`/`"Exponential"` | `"Linear"` | 重试策略 |
| `retry_delay_seconds` | u64 | `30` | 基础重试延迟（秒） |
| `max_retry_delay_seconds` | u64 | `300` | 最大重试延迟（秒） |
| `description` | &str | `""` | 任务描述 |
| `dead_letter_topic` | Option<&str> | `None` | 死信队列 topic，`None` 则不启用 DLQ |

宏把 `TaskHandlerEntry { task_type, handler_fn, config_fn }` 塞进
`alun_task::TASK_HANDLERS` 分布式切片（`alun-task/src/lib.rs:59-73`），运行时由
`HandlerRegistry::from_discovered()` 批量取出。trait 本体只有两个方法
（`alun-task/src/handler.rs:25-34`）：`fn task_type(&self) -> i16` 与
`async fn execute(&self, payload: Value) -> Result<Value, String>`。

## Implement TaskStorage

框架不持有任何 SQL：8 个方法全部由业务方实现（表名、字段、SQL 自定）
（`alun-task/src/storage.rs:31-97`）。

| 方法 | 调用时机 |
|------|----------|
| `save_task_log(task_id, task_type, priority, config, params)` | `TaskProducer::submit` |
| `save_task_queue(task_id, topic, priority)` | `TaskProducer::submit` |
| `update_task_status(task_id, status)` | Worker 拾取 → `Processing`；成功 → `Completed`；失败 → `Failed` / `DeadLetter` |
| `get_retry_count(task_id) -> i64` | Worker 失败分支（判断是否超限） |
| `update_retry(task_id, retry_count)` | Worker 失败且未超限（递增后等 RetryScanner 重推） |
| `save_task_result(task_id, output)` | 成功时保存结果；无 DLQ 且超限时保存错误 JSON |
| `log_execution(task_id, status, error, elapsed_ms)` | 每次执行（含重试）都记一条 |
| `scan_retryable_tasks(task_types, limit) -> Vec<RetryableTask>` | `RetryScanner` 定期调用 |

`RetryableTask { task_id, task_type, retry_count, max_retries, payload }` 是扫描返回项
（`alun-task/src/storage.rs:14-28`）。

```rust,ignore
struct DbTaskStorage;

#[async_trait::async_trait]
impl TaskStorage for DbTaskStorage {
    async fn save_task_log(&self, task_id: &str, task_type: i16, priority: i16,
        config: &TaskConfig, params: &SubmitTaskParams) -> Result<(), String> {
        // 注意：Db::execute 不带参数，带参请用 execute_with_params（参数是 Vec<Value>）
        db().execute_with_params(
            "INSERT INTO task_logs (task_id, task_type) VALUES ($1, $2)",
            vec![task_id.into(), (task_type as i64).into()],
        ).await.map(|_| ()).map_err(|e| e.to_string())
    }
    // ... 其余 7 个方法见上表
}
```

## Startup Registration

`HandlerRegistry::from_discovered()` 接收 `&mut self`，**不能**链式塞进构造函数；
`register(handler, config)` 也是 `&mut self`（`alun-task/src/registry.rs:26-56`）。

```rust,ignore
#[tokio::main]
async fn main() {
    // [task] 段没有专用结构字段，落在 AppConfig.custom 里（HashMap<String, Value>）
    let task_cfg: TaskWorkerConfig = cfg().custom.get("task")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let mut registry = HandlerRegistry::new();
    registry.from_discovered();           // 批量注册 #[task_handler] 发现的处理器

    App::new()
        .expect("初始化失败")
        .plugin(TaskPlugin::new(task_cfg, Arc::new(DbTaskStorage), registry).unwrap())
        .scan()
        .start()                          // 全局资源在此初始化，随后按拓扑序启动插件
        .await
        .unwrap();
}
```

`TaskPlugin::new(config, storage, registry) -> Result<Self, String>`：从 registry 推导
去重后的 topic 列表；`start()` 在后台 `tokio::spawn` 起 Worker + RetryScanner（任一创建失败
只记 `error!` 并让该后台任务退出，**不会**让插件 `start()` 失败），`stop()` 发停止信号并
await 两个后台任务（`alun-task/src/plugin.rs:58-66`、`:120-218`）。
`plugin.topics()` / `plugin.metrics()` 可查 topic 列表与指标（`:85-92`）。

## Submit Tasks from Handlers

```rust,ignore
let mut registry = HandlerRegistry::new();
registry.from_discovered();

let producer = TaskProducer::new(&brokers, Arc::new(DbTaskStorage), registry)
    .map_err(ApiError::internal)?;

let task_id = producer.submit(SubmitTaskParams {
    task_type: 1,
    payload: json!({"file_id": "f1"}),
    priority: Some(TaskPriority::High),
    user_id: Some("u1".into()),
    resource_id: None,
    resource_type: None,
}).await.map_err(ApiError::internal)?;
```

- `TaskProducer::new(brokers: &str, storage, registry) -> Result<Self, String>`
  （`alun-task/src/producer.rs:36-51`）。
- `submit(params) -> Result<String, String>`：生成 `task_id` → `save_task_log` +
  `save_task_queue` → 发 Kafka 消息（5s 超时），返回 `task_id`
  （`alun-task/src/producer.rs:58-105`）。
- `submit_batch(SubmitBatchParams { tasks }) -> (成功数, Vec<(index, 错误)>)`：逐个提交、
  部分失败不影响其他任务（`:107-126`）。
- `send_to_dlq(msg, topic, reason)`：把 `TaskMessage` 转发到 DLQ 并更新状态为 `DeadLetter`
  （`:128-151`）。

`SubmitTaskParams` 字段：`task_type` / `payload` / `priority: Option<TaskPriority>` /
`user_id` / `resource_id` / `resource_type: Option<ResourceType>`
（`alun-task/src/types.rs:191-206`、`ResourceType` 在 `:157-189`）。

## Retry Strategies

`compute_retry_delay(strategy, base, max, attempt)` 的计算口径
（`alun-task/src/retry.rs:149-157`）：

| Strategy | Formula | Note |
|----------|---------|------|
| **Fixed** | `base` | 恒定延迟，**不受 `max` 约束** |
| **Linear** | `base × (attempt + 1)` | 线性退避，上限 `max` |
| **Exponential** | `base × 2^attempt` | 指数退避，上限 `max` |

`attempt` 传的是**当前已重试次数**（`task.retry_count`）。`RetryScanner` 每
`[task].scan_interval_secs` 扫一轮：`scan_retryable_tasks` → 逐条算延迟、`update_retry`、
重新推入 Kafka（`alun-task/src/retry.rs:58-146`）。

> 实现细节（能力缺口）：算出的 `delay` 目前**只写进日志**，`scan()` 里没有 sleep 或延迟
> 队列，重推发生在同一轮扫描里；实际生效的“重试间隔”≈ 扫描周期
> （`alun-task/src/retry.rs:99-146`）。

### 死信队列

Worker 失败分支（`alun-task/src/worker.rs:221-267`）：`attempt = 已重试次数 + 1`，

- `attempt > max_retries` 且配了 `dead_letter_topic` → 发送 DLQ + 状态置 `DeadLetter`；
- `attempt > max_retries` 但没有 DLQ topic → 状态置 `Failed` 并把
  `{"error", "retries"}` 存进 `save_task_result`；
- 否则 `update_retry(attempt)` 等扫描器重推。

超时由 Worker 用 `tokio::time::timeout(config.timeout_seconds, execute(..))` 强制施加
（`timeout_seconds = 0` 则不施加）；随后 `commit_message(CommitMode::Async)` 提交 offset
（未过期但 handler 未找到的消息也会提交并跳过）
（`alun-task/src/worker.rs:114-219`）。

## Task Configuration

`TaskWorkerConfig` 从 **`[task]` 段**反序列化。注意 `AppConfig` 没有 `task` 字段，
该段落在 `AppConfig.custom`（`HashMap<String, serde_json::Value>`，
`alun-config/src/lib.rs:83`），所以要用 `cfg().custom.get("task")` 取（见上文注册示例）。

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

字段与默认值（`alun-task/src/types.rs:120-155`）：`brokers`（`"localhost:9092"`）、
`group_id`（`"alun-task-worker"`）、`scan_interval_secs`（30）、`max_batch_size`（100）、
`max_message_age_secs`（3600，Worker 收到超龄消息直接跳过并提交 offset）、
`auto_create_topics`（false）、`topic_partitions`（1）、`topic_replication`（1）。

## Task Statuses

`TaskStatus`（`alun-task/src/types.rs:8-36`，`#[repr(i16)]`）：

| 值 | 变体 | 说明 |
|----|------|------|
| 1 | `Pending` | 待处理 |
| 2 | `Processing` | 处理中 |
| 3 | `Completed` | 已完成（终态） |
| 4 | `Failed` | 失败 |
| 5 | `Cancelled` | 已取消（终态） |
| 6 | `Scheduled` | 已调度 |
| 7 | `DeadLetter` | 死信（终态） |

辅助：`from_i16` / `to_i16` / `is_terminal()`（`Completed` / `Cancelled` / `DeadLetter`）。
`TaskPriority`：`Low=1`、`Normal=2`、`High=3`、`Critical=4`（`:54-81`）。

指标：`TaskPlugin::metrics() -> Arc<TaskMetrics>`，`TaskMetrics::snapshot()` 返回 JSON
（`total`/`pending`/`running`/`completed`/`failed`/`cancelled`/`retried`）。注意 Worker 目前
只递增 `total` / `completed` / `failed`，其余计数器保持 0（`alun-task/src/worker.rs:114-267`、
`alun-task/src/metrics.rs:27-38`）。

---

## Kafka Integration (`alun-kafka`)

Requires `features = ["kafka"]`。注意 `KafkaProducer::new` / `KafkaConsumer::new` / `KafkaPlugin::new`
都**不返回 `Result`**（构造即惰性，真正连接在发送/消费时建立）；`KafkaConsumer` 只暴露
`new` / `with_config` / `inner()`（拿原生 `rdkafka::consumer::StreamConsumer` 自行消费），
且其 `new` / `with_config` 内部 `subscribe` 用 `expect`，配置非法会 **panic**
（`alun-kafka/src/consumer.rs:19-64`）。门面路径 `alun::alun_kafka::…`。

```rust,no_run
use alun::alun_kafka::{KafkaConsumer, KafkaError, KafkaPlugin, KafkaProducer};
use serde_json::json;

# async fn demo() -> Result<(), KafkaError> {
let producer = KafkaProducer::new("localhost:9092");
producer.send("order-events", "order.created", &json!({"id": 1})).await?;

let consumer = KafkaConsumer::new("localhost:9092", "group-id", &["order-events"]);
let _native = consumer.inner();       // &rdkafka::consumer::StreamConsumer

let plugin = KafkaPlugin::new("localhost:9092");
let _ = plugin.brokers();
# Ok(())
# }
```

- `KafkaProducer::send<T: Serialize + Debug>(topic, key, &payload) -> Result<(), KafkaError>`；
  `send_bytes(topic, key, &[u8])` 适用于二进制载荷（`alun-kafka/src/producer.rs:29-40`）。
- `KafkaPlugin` 实现 `Plugin`（`name() = "kafka"`），`publish(topic, key, &payload) ->
  alun::Result<()>`；`start()` 只检查 producer 是否初始化成功（失败也只记日志），
  初始化失败时 `publish()` 返回 `Err`（`alun-kafka/src/plugin.rs:28-78`）。
- `KafkaConsumer` 默认 `enable.auto.commit = true`、`auto.offset.reset = earliest`，与
  `TaskWorker` 自建的消费者（`enable.auto.commit = false` + 手动 commit）不同
  （`alun-kafka/src/consumer.rs:23-28`、`alun-task/src/worker.rs:52-64`）。

> `KafkaProducer::send` / `send_bytes` 的错误类型是 `alun_kafka::KafkaError`
> （已从 crate 根再导出，实现 `std::error::Error`），可直接 `?` 上抛或 `map_err` 转换。

---

## File System (`alun-fs`)

Requires `features = ["fs"]`（门面路径 `alun::alun_fs::…`）。后端统一实现 `StorageBackend`
（`write` / `write_at` / `read` / `delete` / `exists`），本地实现是 `LocalFs`；
`exists` 返回 `bool`（不是 `Result`）。

```rust,no_run
use alun::alun_fs::{LocalFs, StorageBackend};

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
let fs = LocalFs::new("uploads");

// Write（original_name → 自动生成 stored_path）
let meta = fs.write("report.pdf", b"binary data").await.map_err(|e| e.to_string())?;
let _ = (&meta.file_id, &meta.original_name, &meta.stored_path, meta.size, &meta.content_type);

// 精确路径写入 / 读取 / 删除
fs.write_at("reports/2024/report.pdf", b"binary data").await.map_err(|e| e.to_string())?;
let _data = fs.read("reports/2024/report.pdf").await.map_err(|e| e.to_string())?;
fs.delete("reports/2024/report.pdf").await.map_err(|e| e.to_string())?;

// 存在性检查（bool）
let _exists: bool = fs.exists("reports/2024/report.pdf").await;
# Ok(())
# }
```

多后端注册（`BackendRegistry` 在 `alun_fs::registry`，配置类型在 `alun_fs::types`）：

```rust,no_run
use alun::alun_fs::registry::BackendRegistry;
use alun::alun_fs::types::BackendConfig;
use alun::alun_fs::{LocalFs, StorageBackend};

# fn demo() -> Option<BackendRegistry> {
let mut registry = BackendRegistry::new();
registry.register(LocalFs::new("uploads"), BackendConfig::default());
registry.with_default("local");
Some(registry)
# }
```

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [启动钩子 / 自定义中间件 / WebPlugin / 异步 Hook](extension-points.md) | [SSE 实时推送](realtime-sse.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

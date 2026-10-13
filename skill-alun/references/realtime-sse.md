# 实时推送（SSE）

> 适用于：`features = ["sse"]`（门面 `full` 预设已含）。
> 一句话：只需要「服务端 → 客户端」单向推送时，用 SSE；需要双向时再用 axum 的 WebSocket。

## 什么时候用 SSE

| 场景 | 是否适合 |
|---|---|
| 站内信 / 待办红点提醒 | ✅ 最适合 |
| 大文件导入导出进度 | ✅ 最适合 |
| 监控大屏、日志 tail | ✅ 最适合 |
| 多人协同光标、在线白板 | ❌ 用 WebSocket |

理由（SSE 相对 WebSocket 的确定性优势）：

- 走**普通 HTTP**，浏览器自动带 Cookie、也能写 `Authorization` 头 —— 直接复用框架的 `AuthLayer`；
  WebSocket 无法带请求头，得靠 query token / subprotocol 变通；
- **浏览器 `EventSource` 自带断线重连**；
- 服务端只需一个广播器 + 一个 handler，约 20 行。

## 三步上手

### 1. 建一个进程级广播器

```rust,no_run
use std::sync::OnceLock;
use alun::prelude::*;

static NOTIFY: OnceLock<Broadcaster<String>> = OnceLock::new();

fn notify() -> &'static Broadcaster<String> {
    NOTIFY.get_or_init(|| Broadcaster::new(1024))
}
```

> `Broadcaster::new(capacity)` 里 `capacity` 是**每个订阅端**的缓冲上限（内部 `tokio::sync::broadcast`，
> 至少取 1）。放到 `OnceLock` 只是为了全局可达；也可以用启动钩子 `App::on_startup(|| async { .. })`
> 初始化并存进 `OnceLock` / `LazyLock`。

### 2. 开一个订阅端点

```rust,no_run
use alun::prelude::*;

#[alun::get("/events")]
async fn events() -> impl axum::response::IntoResponse {
    sse_json(notify().subscribe(), "message")
}
```

`sse_json(rx, event_name)` 已内置 **15 秒一条注释行**的保活（防反向代理按空闲超时切断连接）。

### 3. 在需要的地方推送

```rust,ignore
use alun::prelude::*;

#[alun::post("/orders")]
async fn create_order(ValidatedJson(req): ValidatedJson<serde_json::Value>) -> ResResult<()> {
    // …落库…
    notify().send(json!({ "type": "order_created", "id": 1 }).to_string());
    Ok(Res::ok_empty())
}
```

前端：

```js
const es = new EventSource("/api/events");          // 若开了鉴权，别忘带 header / cookie
es.addEventListener("message", (e) => console.log(e.data));
```

## API

| API | 说明 |
|---|---|
| `Broadcaster::new(capacity)` | 建广播器，有界缓冲 |
| `.send(msg) -> usize` | 推送一条，返回**送达的订阅端数**；没人订阅也正常返回 0 |
| `.subscribe() -> broadcast::Receiver<T>` | 取订阅端，交给 `sse_json` |
| `.receiver_count() -> usize` | 当前连接数（健康检查 / 限流可用） |

`Broadcaster<T>` 要求 `T: Clone + Send + 'static`。

## 三条必须知道的约束

1. **积压不报错、只丢最旧消息**：订阅端消费不及（`.recv()` 返回 `Lagged`）时丢弃最旧的若干条，
   但**连接不断**。这是「提醒」类消息的正确取舍 —— 宁可漏一条旧提醒，也不断开长连接。
   若消息不能丢（比如订单状态变更），改用 Redis Pub/Sub 或 `alun-kafka` 做可靠扇出。
2. **广播器是进程内的**：多实例部署时 A 实例推送，B 实例的订阅者收不到。跨进程扇出请换 Redis
   Pub/Sub / Kafka。
3. **推送端不受 `AuthLayer` 保护**：`send` 只是往 channel 里塞一条 —— 谁该收到什么由业务自己过滤
   （例如把 `tenant_id` 编进消息，订阅端按租户过滤，或者给每个租户一个独立的 Broadcaster）。

## 与 WebSocket 的边界

框架**没有内置 WebSocket**（SSE 已覆盖单向推送场景）。需要双向通信时直接用
`axum::extract::ws`，鉴权走 subprotocol 或握手头；不要为了「看起来统一」硬把 SSE 改成 WebSocket。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [异步任务 / 定时任务 / Kafka](task.md) | [可观测 / Prometheus / 安全指标](observability.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

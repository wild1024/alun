//! Server-Sent Events（SSE）—— 单向实时推送的轻量辅助。
//!
//! # 适用场景
//!
//! 站内信实时提醒、审批待办红点、大文件导入/导出进度、监控大屏。
//! 这些场景**只需服务端 → 客户端**单向推送，用 SSE 即可，
//! 无需引入 WebSocket 的握手、心跳、连接注册表与鉴权复杂度。
//!
//! # 为什么用 SSE 而不是 WebSocket
//!
//! - 走普通 HTTP，**浏览器自动携带 Cookie / 可用 `Authorization` 头**，
//!   因此**天然复用框架的 `AuthLayer`**——WebSocket 无法带 `Authorization`
//!   头，需要额外的 query token / subprotocol 方案；
//! - 内置断线重连（浏览器 `EventSource` 自动重连）；
//! - 实现成本极低。
//!
//! # 示例
//!
//! ```ignore
//! use std::sync::OnceLock;
//! use alun::sse::{Broadcaster, sse_json};
//!
//! // 进程级广播器（放进全局单例 / 启动钩子初始化）
//! static NOTIFY: OnceLock<Broadcaster<String>> = OnceLock::new();
//! fn notify() -> &'static Broadcaster<String> {
//!     NOTIFY.get_or_init(|| Broadcaster::new(1024))
//! }
//!
//! // 推送端（任意 handler 内）
//! notify().send(r#"{"type":"todo","id":7}"#.to_string());
//!
//! // 订阅端（GET /events）—— 返回类型由 sse_json 推断即可
//! async fn events() -> impl axum::response::IntoResponse {
//!     sse_json(notify().subscribe(), "message")
//! }
//! ```

use std::convert::Infallible;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::Stream;
use futures_util::stream::unfold;
use serde::Serialize;
use tokio::sync::broadcast;

/// 广播器：一个发送端，扇出到任意多个 SSE 订阅端。
///
/// 内部为 `tokio::sync::broadcast`，**无锁**、有界缓冲。订阅端消费不及导致
/// 积压时，最旧的若干条会被丢弃（`Lagged`），但**连接保持不断**——这对
/// 「提醒」类消息是正确取舍（宁可漏一条旧提醒，也不断开长连接）。
pub struct Broadcaster<T> {
    tx: broadcast::Sender<T>,
}

impl<T: Clone + Send + 'static> Broadcaster<T> {
    /// 创建广播器。`capacity` 为每个订阅端的缓冲上限（至少 1）。
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(1));
        Self { tx }
    }

    /// 订阅一个新的接收流（每次 `GET /events` 调用一次）。
    pub fn subscribe(&self) -> broadcast::Receiver<T> {
        self.tx.subscribe()
    }

    /// 广播一条消息。返回成功入队的订阅端数量（当前无订阅端时为 0）。
    pub fn send(&self, msg: T) -> usize {
        self.tx.send(msg).unwrap_or(0)
    }

    /// 当前活跃订阅端数量（可用于健康检查 / 观测）。
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

/// 默认保活间隔。
const KEEP_ALIVE_SECS: u64 = 15;

/// 把 `broadcast::Receiver<T>` 适配为「JSON 序列化 → SSE Event」的流。
///
/// 用 `futures_util::stream::unfold` 驱动 `recv().await`（而非手写
/// `poll_recv`）：tokio 1.53 的 `broadcast::Receiver` 未公开 `poll_recv`，
/// 而 `unfold` 按值持有 `Receiver`，无需自引用即可把异步接收转为 `Stream`。
fn json_sse_stream<T>(
    rx: broadcast::Receiver<T>,
    event: &'static str,
) -> impl Stream<Item = Result<Event, Infallible>>
where
    T: Serialize + Clone + Send + 'static,
{
    unfold(rx, move |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(value) => {
                    let event = match serde_json::to_string(&value) {
                        Ok(json) => Event::default().event(event).data(json),
                        Err(e) => {
                            // 单条序列化失败不应中断长连接：以 error 事件告知前端后继续。
                            tracing::warn!("SSE 消息序列化失败: {e}");
                            Event::default()
                                .event("error")
                                .data("{\"error\":\"serialize failed\"}")
                        }
                    };
                    return Some((Ok(event), rx));
                }
                // 订阅端消费不及、丢弃了 `n` 条：跳过继续，**不断开连接**。
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("SSE 订阅端落后 {n} 条消息，已跳过（连接保持）");
                    continue;
                }
                // 发送端已全部释放或通道关闭 → 正常结束流。
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
}

/// 构造 SSE 响应：把广播消息以 JSON 形式推送给客户端。
///
/// - `rx`：来自 [`Broadcaster::subscribe`]
/// - `event_name`：SSE 事件名（前端 `addEventListener("<name>", ...)`）；
///   约定用 `"message"` 作为默认事件名。
///
/// 自动附加保活（每 15s 一条注释行），防止反向代理按空闲超时切断连接。
pub fn sse_json<T>(
    rx: broadcast::Receiver<T>,
    event_name: &'static str,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
where
    T: Serialize + Clone + Send + 'static,
{
    Sse::new(json_sse_stream(rx, event_name)).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(KEEP_ALIVE_SECS))
            .text("keep-alive"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    #[tokio::test]
    async fn broadcaster_fans_out_to_all_subscribers() {
        let bus: Broadcaster<i64> = Broadcaster::new(8);
        assert_eq!(bus.receiver_count(), 0);

        let mut a = bus.subscribe();
        let mut b = bus.subscribe();
        assert_eq!(bus.receiver_count(), 2);

        assert_eq!(bus.send(7), 2, "两个订阅端都应收到");
        assert_eq!(a.recv().await.unwrap(), 7);
        assert_eq!(b.recv().await.unwrap(), 7);
    }

    #[tokio::test]
    async fn sse_stream_serializes_to_json_events() {
        let bus: Broadcaster<serde_json::Value> = Broadcaster::new(8);
        let rx = bus.subscribe();
        let mut stream = Box::pin(json_sse_stream(rx, "message"));

        bus.send(serde_json::json!({"type": "todo", "id": 7}));
        let ev = stream.next().await.unwrap().unwrap();
        // Event 无公开访问器：断言其 Debug 含有事件名与 JSON 载荷
        let dbg = format!("{ev:?}");
        assert!(dbg.contains("message"), "事件名应为 message：{dbg}");
        assert!(dbg.contains("todo"), "载荷应含 type=todo：{dbg}");
    }

    #[tokio::test]
    async fn stream_ends_when_sender_dropped() {
        let bus: Broadcaster<i64> = Broadcaster::new(4);
        let rx = bus.subscribe();
        let mut stream = Box::pin(json_sse_stream(rx, "message"));
        drop(bus); // 发送端全部释放 → 流应正常结束
        assert!(stream.next().await.is_none());
    }
}

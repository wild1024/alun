//! 异步任务框架
//!
//! 基于 Kafka 消息队列的任务分发与处理框架。
//! 插件本身**不持有任何 SQL 或表结构**——所有持久化通过 `TaskStorage` trait 委托给业务方实现。
//!
//! ## 核心能力
//!
//! - 任务提交（Kafka 消息 + 通过 `TaskStorage` 持久化）
//! - 处理器注册（按 `task_type` 分发到不同业务 Handler）
//! - 重试机制（固定延迟/线性增长/指数退避）+ 死信队列
//! - 任务生命周期标记（PENDING → PROCESSING → COMPLETED/FAILED/CANCELLED/DEAD_LETTER）
//! - `alun_core::Plugin` 集成（通过 `PluginManager` 统一启动/停止）
//! - `#[task_handler]` 宏注解自动发现 Handler（编译期 linkme 收集）
//! - **配置从配置文件 `[task]` section 读取**（`TaskWorkerConfig` 实现 Deserialize）
//!
//! ## 使用方式
//!
//! ```ignore
//! // 1. 实现 TaskStorage（内部通过 db() 全局函数操作数据库）
//! struct DbTaskStorage;
//! impl TaskStorage for DbTaskStorage { ... }
//!
//! // 2. 从配置文件读取配置，创建插件 → 注册到 App
//! let task_cfg: TaskWorkerConfig = cfg().custom.get("task")...unwrap_or_default();
//! let plugin = TaskPlugin::new(
//!     task_cfg,
//!     Arc::new(DbTaskStorage),
//!     HandlerRegistry::new().from_discovered(),
//! )?;
//! app.plugin(plugin).scan().start().await
//! ```

mod types;
mod storage;
mod handler;
mod registry;
mod producer;
mod worker;
mod retry;
mod metrics;
mod plugin;

pub use types::*;
pub use storage::{TaskStorage, RetryableTask};
pub use handler::TaskHandler;
pub use registry::HandlerRegistry;
pub use producer::TaskProducer;
pub use worker::TaskWorker;
pub use retry::{RetryScanner, compute_retry_delay};
pub use metrics::TaskMetrics;
pub use plugin::TaskPlugin;

/// 任务处理器注册条目 —— 编译期由 `#[task_handler]` 宏收集
///
/// 与 linkme distributed_slice 配合使用，实现编译期自动发现。
#[derive(Debug, Clone)]
pub struct TaskHandlerEntry {
    /// 任务类型标识
    pub task_type: i16,
    /// 处理器构建函数（返回 Box<dyn TaskHandler>）
    pub handler_fn: fn() -> Box<dyn TaskHandler>,
    /// 任务配置
    pub config_fn: fn() -> TaskConfig,
}

/// 任务处理器分布式切片 —— `#[task_handler]` 宏注解的 handler 在此汇集
///
/// 使用 linkme 在链接期自动收集所有被注解的 handler，
/// 启动时通过 `HandlerRegistry::from_discovered()` 一键注册。
#[linkme::distributed_slice]
pub static TASK_HANDLERS: [TaskHandlerEntry] = [..];
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
//! - `#[task_handler]` 宏注解自动发现 Handler（编译期 `inventory` 收集）
//! - **配置从配置文件 `[task]` section 读取**（`TaskWorkerConfig` 实现 Deserialize）
//!
//! ## 使用方式
//!
//! ```ignore
//! // 1. 实现 TaskStorage（内部通过 db() 全局函数操作数据库）
//! struct DbTaskStorage;
//! #[async_trait::async_trait]
//! impl TaskStorage for DbTaskStorage { /* 见 storage.rs 的 8 个方法 */ }
//!
//! // 2. 处理器注册表：先建后批量注册 `#[task_handler]` 宏发现的处理器
//! //    （`from_discovered` 接收 `&mut self`，不能链式塞进构造函数）
//! let mut registry = HandlerRegistry::new();
//! registry.from_discovered();
//!
//! // 3. 从配置文件读取 `[task]` 段，创建插件 → 注册到 App
//! let task_cfg: TaskWorkerConfig = cfg().custom.get("task")...unwrap_or_default();
//! let plugin = TaskPlugin::new(task_cfg, Arc::new(DbTaskStorage), registry)?;
//! App::new()?.plugin(plugin).scan().start().await?;
//! ```

mod handler;
mod metrics;
mod registry;
mod storage;
mod types;

// ── Kafka 后端（可选）──────────────────────────────────────────
// 仅在 `kafka` feature 开启时编译。该后端依赖 `rdkafka`，构建时
// 需要 CMake + C 工具链（librdkafka 自带 `configure` 脚本，在纯
// Windows 工具链下无法构建）。
//
// 关闭 `kafka` 时，本 crate 仍提供：任务处理器注册（`HandlerRegistry`
// + `#[task_handler]` 宏收集）、`TaskStorage` 持久化抽象、任务类型与
// 生命周期定义、重试延迟策略（`compute_retry_delay`）——即"任务模型"
// 部分可用，只缺 Kafka 的收发与后台 worker。
#[cfg(feature = "kafka")]
mod plugin;
#[cfg(feature = "kafka")]
mod producer;
#[cfg(feature = "kafka")]
mod retry;
#[cfg(feature = "kafka")]
mod worker;

pub use handler::TaskHandler;
pub use metrics::TaskMetrics;
pub use registry::HandlerRegistry;
pub use storage::{RetryableTask, TaskStorage};
pub use types::*;

#[cfg(feature = "kafka")]
pub use plugin::TaskPlugin;
#[cfg(feature = "kafka")]
pub use producer::TaskProducer;
#[cfg(feature = "kafka")]
pub use retry::RetryScanner;
#[cfg(feature = "kafka")]
pub use worker::TaskWorker;

/// 任务处理器注册条目 —— 编译期由 `#[task_handler]` 宏收集
///
/// 通过 `inventory` 收集（**不依赖链接器自定义段**，见 [`crate::TASK_HANDLERS`]）。
#[derive(Debug, Clone)]
pub struct TaskHandlerEntry {
    /// 任务类型标识
    pub task_type: i16,
    /// 处理器构建函数（返回 Box<dyn TaskHandler>）
    pub handler_fn: fn() -> Box<dyn TaskHandler>,
    /// 任务配置
    pub config_fn: fn() -> TaskConfig,
}

inventory::collect!(TaskHandlerEntry);

/// 遍历编译期注册的全部任务处理器（由 `#[alun::task_handler]` 宏提交）。
///
/// # 为什么用 `inventory` 而不是 `linkme`
///
/// `linkme::distributed_slice` 依赖链接器保留自定义段；在 `windows-gnu`
/// 等工具链的 `--gc-sections` 下会被**整段回收**，切片长度为 0 且**链接无报错**，
/// 表现为 handler 静默全部丢失。`inventory` 不依赖自定义段，任何工具链都成立。
pub fn discovered_task_handlers() -> impl Iterator<Item = &'static TaskHandlerEntry> {
    inventory::iter::<TaskHandlerEntry>.into_iter()
}

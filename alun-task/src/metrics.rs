//! 任务指标（原子计数器，并发安全）

use std::sync::atomic::AtomicU64;

/// 任务执行指标 —— 原子计数器，并发安全
///
/// 工作线程通过 AtomicU64 递增，避免锁开销。
pub struct TaskMetrics {
    /// 提交任务总数
    pub total: AtomicU64,
    /// 待处理任务数
    pub pending: AtomicU64,
    /// 执行中任务数
    pub running: AtomicU64,
    /// 已完成任务数
    pub completed: AtomicU64,
    /// 失败任务数
    pub failed: AtomicU64,
    /// 已取消任务数
    pub cancelled: AtomicU64,
    /// 重试任务数
    pub retried: AtomicU64,
}

impl TaskMetrics {
    /// 创建零初始化的指标实例
    pub fn new() -> Self {
        Self {
            total: AtomicU64::new(0u64),
            pending: AtomicU64::new(0u64),
            running: AtomicU64::new(0u64),
            completed: AtomicU64::new(0u64),
            failed: AtomicU64::new(0u64),
            cancelled: AtomicU64::new(0u64),
            retried: AtomicU64::new(0u64),
        }
    }

    /// 获取指标快照（JSON 格式，用于监控/API 接口）
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "total": self.total.load(std::sync::atomic::Ordering::Relaxed),
            "pending": self.pending.load(std::sync::atomic::Ordering::Relaxed),
            "running": self.running.load(std::sync::atomic::Ordering::Relaxed),
            "completed": self.completed.load(std::sync::atomic::Ordering::Relaxed),
            "failed": self.failed.load(std::sync::atomic::Ordering::Relaxed),
            "cancelled": self.cancelled.load(std::sync::atomic::Ordering::Relaxed),
            "retried": self.retried.load(std::sync::atomic::Ordering::Relaxed),
        })
    }
}

impl Default for TaskMetrics {
    fn default() -> Self { Self::new() }
}

/// AtomicU64 原子递增扩展 trait
pub(crate) trait AtomicInc {
    /// 原子递增并返回递增后的值
    fn inc(&self) -> u64;
}

impl AtomicInc for AtomicU64 {
    fn inc(&self) -> u64 {
        self.fetch_add(1u64, std::sync::atomic::Ordering::Relaxed) + 1u64
    }
}
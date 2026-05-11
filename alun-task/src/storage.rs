//! 任务持久化 trait —— 由业务方实现的自定义存储接口
//!
//! 插件不持有任何 SQL 语句、表名、字段名。所有持久化逻辑完全由调用方通过实现此 trait 控制。
//! 业务方可以自由选择存储后端（PostgreSQL / MySQL / MongoDB / 文件 ……）。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::types::{TaskConfig, TaskStatus, SubmitTaskParams};

/// 扫描到的可重试任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryableTask {
    /// 任务 ID
    pub task_id: String,
    /// 任务类型标识
    pub task_type: i16,
    /// 当前已重试次数
    pub retry_count: i64,
    /// 最大允许重试次数
    pub max_retries: i64,
    /// 任务载荷（JSON 格式）
    pub payload: serde_json::Value,
}

/// 任务持久化接口
///
/// 业务方实现此 trait，定义任务数据的存储方式。
/// 所有方法均返回 `Result<(), String>`，失败原因通过 `Err(String)` 传递。
#[async_trait]
pub trait TaskStorage: Send + Sync {
    /// 持久化任务日志（提交任务时调用）
    ///
    /// 接收完整的任务提交参数与配置，由实现方决定如何存储（表名、字段映射）。
    async fn save_task_log(
        &self,
        task_id: &str,
        task_type: i16,
        priority: i16,
        config: &TaskConfig,
        params: &SubmitTaskParams,
    ) -> Result<(), String>;

    /// 持久化任务队列记录（提交任务时调用）
    ///
    /// 记录任务已入队，供外部查询队列状态。
    async fn save_task_queue(
        &self,
        task_id: &str,
        topic: &str,
        priority: i16,
    ) -> Result<(), String>;

    /// 更新任务状态
    ///
    /// 状态变更时机：提交后 Pending → Worker 拾取 Processing → 成功 Completed / 失败 Failed / DLQ DeadLetter
    async fn update_task_status(
        &self,
        task_id: &str,
        status: TaskStatus,
    ) -> Result<(), String>;

    /// 获取当前重试次数
    async fn get_retry_count(&self, task_id: &str) -> Result<i64, String>;

    /// 更新重试信息
    ///
    /// 失败后递增 retry_count，同时将状态重置为 Pending 等待 RetryScanner 重新推送。
    async fn update_retry(&self, task_id: &str, retry_count: i64) -> Result<(), String>;

    /// 保存任务执行结果（成功或失败后的输出）
    async fn save_task_result(
        &self,
        task_id: &str,
        output: &serde_json::Value,
    ) -> Result<(), String>;

    /// 记录执行日志
    ///
    /// 每次执行（含重试）都记录一条，供审计和排障。
    async fn log_execution(
        &self,
        task_id: &str,
        status: TaskStatus,
        error: Option<&str>,
        elapsed_ms: i64,
    ) -> Result<(), String>;

    /// 扫描需要重试的任务
    ///
    /// 返回当前所有 status=Failed 且 retry_count < max_retries 的任务。
    /// RetryScanner 定期调用此方法获取待重试列表。
    async fn scan_retryable_tasks(
        &self,
        task_types: &[i16],
        limit: usize,
    ) -> Result<Vec<RetryableTask>, String>;
}
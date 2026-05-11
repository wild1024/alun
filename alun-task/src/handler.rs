use async_trait::async_trait;
use serde_json::Value;

/// 任务处理器特质
///
/// 每个业务模块实现此特质以处理特定类型的异步任务。
///
/// ## 示例
///
/// ```ignore
/// struct ExportHandler;
///
/// #[async_trait]
/// impl TaskHandler for ExportHandler {
///     fn task_type(&self) -> i16 { 1 }
///
///     async fn execute(&self, payload: Value) -> Result<Value, String> {
///         let file_id = payload["file_id"].as_str().unwrap();
///         // 执行导出逻辑 ...
///         Ok(serde_json::json!({"url": "https://..."}))
///     }
/// }
/// ```
#[async_trait]
pub trait TaskHandler: Send + Sync {
    /// 返回该处理器对应的 task_type
    fn task_type(&self) -> i16;

    /// 执行任务，返回结果 JSON
    ///
    /// - `payload`: 任务携带的业务数据
    /// - 成功时返回 `Ok(Value)`，结果写入 `task_results` 表
    /// - 失败时返回 `Err(String)`，框架根据重试策略决定后续处理
    async fn execute(&self, payload: Value) -> Result<Value, String>;
}
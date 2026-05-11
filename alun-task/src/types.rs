//! 任务类型、状态、优先级、重试策略等枚举和配置

use serde::{Deserialize, Serialize};

/// 任务状态枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i16)]
pub enum TaskStatus {
    /// 待处理
    Pending = 1,
    /// 处理中
    Processing = 2,
    /// 已完成
    Completed = 3,
    /// 失败
    Failed = 4,
    /// 已取消
    Cancelled = 5,
    /// 已调度（定时任务）
    Scheduled = 6,
    /// 死信队列 —— 超过最大重试次数后转入此状态
    DeadLetter = 7,
}

impl TaskStatus {
    /// 从 i16 值转换为 TaskStatus 枚举
    pub fn from_i16(v: i16) -> Option<Self> {
        match v {
            1 => Some(Self::Pending),
            2 => Some(Self::Processing),
            3 => Some(Self::Completed),
            4 => Some(Self::Failed),
            5 => Some(Self::Cancelled),
            6 => Some(Self::Scheduled),
            7 => Some(Self::DeadLetter),
            _ => None,
        }
    }

    /// 将 TaskStatus 枚举转换为 i16 值
    pub fn to_i16(self) -> i16 {
        self as i16
    }

    /// 判断是否为终态（不会再流转）
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::DeadLetter)
    }
}

/// 任务优先级
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i16)]
pub enum TaskPriority {
    /// 低优先级
    Low = 1,
    /// 普通优先级
    Normal = 2,
    /// 高优先级
    High = 3,
    /// 紧急优先级
    Critical = 4,
}

impl TaskPriority {
    /// 从 i16 值转换为 TaskPriority 枚举
    pub fn from_i16(v: i16) -> Self {
        match v {
            3 => Self::High,
            4 => Self::Critical,
            1 => Self::Low,
            _ => Self::Normal,
        }
    }

    /// 将 TaskPriority 枚举转换为 i16 值
    pub fn to_i16(self) -> i16 {
        self as i16
    }
}

/// 重试策略
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RetryStrategy {
    /// 固定延迟重试
    Fixed,
    /// 线性递增延迟重试
    Linear,
    /// 指数退避延迟重试
    Exponential,
}

/// 任务配置 —— 每种 task_type 对应一份配置
#[derive(Debug, Clone)]
pub struct TaskConfig {
    /// 任务类型标识
    pub task_type: i16,
    /// 任务优先级
    pub priority: TaskPriority,
    /// Kafka topic 名称
    pub topic: String,
    /// 任务执行超时时间（秒）
    pub timeout_seconds: u64,
    /// 最大重试次数
    pub max_retries: u32,
    /// 重试策略
    pub retry_strategy: RetryStrategy,
    /// 重试延迟基础时间（秒）
    pub retry_delay_seconds: u64,
    /// 最大重试延迟时间（秒）
    pub max_retry_delay_seconds: u64,
    /// 任务描述
    pub description: &'static str,
    /// 死信队列 topic —— 超过 max_retries 后转发到此 topic（None 则不启用）
    pub dead_letter_topic: Option<String>,
}

/// TaskWorker 运行时配置（可从配置文件 `[task]` section 反序列化）
#[derive(Debug, Clone, Deserialize)]
pub struct TaskWorkerConfig {
    /// Kafka broker 地址
    pub brokers: String,
    /// 消费组 ID
    pub group_id: String,
    /// 重试扫描间隔（秒）
    pub scan_interval_secs: u64,
    /// 每批次扫描最大任务数
    pub max_batch_size: usize,
    /// 消息最大时效（秒），超过此时间的消息将被跳过
    pub max_message_age_secs: u64,
    /// 是否在启动时自动创建 topic
    pub auto_create_topics: bool,
    /// topic 分区数（auto_create_topics 时使用）
    pub topic_partitions: i32,
    /// topic 副本数（auto_create_topics 时使用）
    pub topic_replication: i16,
}

impl Default for TaskWorkerConfig {
    fn default() -> Self {
        Self {
            brokers: "localhost:9092".into(),
            group_id: "alun-task-worker".into(),
            scan_interval_secs: 30,
            max_batch_size: 100,
            max_message_age_secs: 3600,
            auto_create_topics: false,
            topic_partitions: 1,
            topic_replication: 1,
        }
    }
}

/// 资源类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i16)]
pub enum ResourceType {
    /// 用户资源
    User = 1,
    /// 订单资源
    Order = 2,
    /// 商品资源
    Product = 3,
    /// 门店资源
    Store = 4,
    /// 文件资源
    File = 5,
}

impl ResourceType {
    /// 从 i16 值转换为 ResourceType 枚举
    pub fn from_i16(v: i16) -> Option<Self> {
        match v {
            1 => Some(Self::User),
            2 => Some(Self::Order),
            3 => Some(Self::Product),
            4 => Some(Self::Store),
            5 => Some(Self::File),
            _ => None,
        }
    }

    /// 将 ResourceType 枚举转换为 i16 值
    pub fn to_i16(self) -> i16 {
        self as i16
    }
}

/// 任务提交参数
#[derive(Debug, Clone)]
pub struct SubmitTaskParams {
    /// 任务类型标识
    pub task_type: i16,
    /// 任务载荷（JSON 格式的业务数据）
    pub payload: serde_json::Value,
    /// 优先级（None 使用配置默认值）
    pub priority: Option<TaskPriority>,
    /// 提交用户 ID（可选，用于审计）
    pub user_id: Option<String>,
    /// 关联资源 ID（可选）
    pub resource_id: Option<String>,
    /// 关联资源类型（可选）
    pub resource_type: Option<ResourceType>,
}

/// 批量任务提交参数
#[derive(Debug, Clone)]
pub struct SubmitBatchParams {
    /// 任务列表
    pub tasks: Vec<SubmitTaskParams>,
}

/// 从 Kafka 消费到的任务消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMessage {
    /// 任务 ID
    pub task_id: String,
    /// 任务类型标识
    pub task_type: i16,
    /// 任务载荷（JSON 格式的业务数据）
    pub payload: serde_json::Value,
    /// 任务优先级数值
    pub priority: i16,
    /// 提交用户 ID（可选）
    pub user_id: Option<String>,
    /// 关联资源 ID（可选）
    pub resource_id: Option<String>,
    /// 关联资源类型（可选）
    pub resource_type: Option<i16>,
    /// 任务提交时间（ISO 8601 格式）
    pub submitted_at: String,
}
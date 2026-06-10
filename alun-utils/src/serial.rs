//! 业务单号生成器：基于规则配置的分布式单号引擎
//!
//! # 设计理念
//!
//! 将"格式模板 + 循环周期 + 计数策略"抽象为可配置的 `SerialRule`，
//! 通过 `SerialGenerator` trait 统一三种后端实现（内存锁/Redis锁/PG锁）。
//!
//! # 格式语法
//!
//! ```text
//! {YYYY}    - 4 位年份
//! {YY}      - 2 位年份
//! {MM}      - 2 位月份
//! {DD}      - 2 位日期
//! {SEQ:n}   - 定长顺序号（n 位，不足补零）
//! {RAND:n}  - 定长随机数（n 位）
//! {TS}      - Unix 时间戳（秒）
//! {TSMS}    - Unix 时间戳（毫秒）
//! ```
//!
//! # 示例
//!
//! ```ignore
//! let rule = SerialRule::new("order", "ORD{YYYY}{MM}{DD}{SEQ:8}")
//!     .with_cycle(CyclePeriod::Daily);
//! backend.register_rule(rule).await?;
//! let no = backend.generate("order").await?; // ORD2024052400000001
//! ```

use std::collections::HashMap;
use std::fmt;
use async_trait::async_trait;
use chrono::{Local, Datelike};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

// ──── 错误类型 ────────────────────────────────────

/// 单号生成器错误
#[derive(Debug, Clone)]
pub enum SerialError {
    /// 规则未找到
    RuleNotFound(String),
    /// 规则已禁用
    RuleDisabled(String),
    /// 格式解析错误
    FormatError(String),
    /// 计数器溢出
    CounterOverflow(String),
    /// 后端存储错误
    StorageError(String),
}

impl fmt::Display for SerialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuleNotFound(k) => write!(f, "单号规则未找到: {}", k),
            Self::RuleDisabled(k) => write!(f, "单号规则已禁用: {}", k),
            Self::FormatError(msg) => write!(f, "格式错误: {}", msg),
            Self::CounterOverflow(msg) => write!(f, "计数器溢出: {}", msg),
            Self::StorageError(msg) => write!(f, "存储错误: {}", msg),
        }
    }
}

impl std::error::Error for SerialError {}

// ──── 周期类型 ────────────────────────────────────

/// 循环周期：计数器何时重置
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CyclePeriod {
    /// 不循环——计数器终生递增
    #[default]
    NoCycle,
    /// 按天循环（YYYYMMDD）
    Daily,
    /// 按月循环（YYYYMM）
    Monthly,
    /// 按年循环（YYYY）
    Yearly,
}

impl CyclePeriod {
    /// 计算当前周期值：Daily→20240524，Monthly→202405，Yearly→2024，NoCycle→空
    pub fn current_value(&self) -> String {
        let now = Local::now();
        match self {
            Self::NoCycle => String::new(),
            Self::Daily => format!("{:04}{:02}{:02}", now.year(), now.month(), now.day()),
            Self::Monthly => format!("{:04}{:02}", now.year(), now.month()),
            Self::Yearly => format!("{:04}", now.year()),
        }
    }
}

// ──── 增量策略 ────────────────────────────────────

/// 增量方式
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncrementStrategy {
    /// 顺序递增（步长 1）
    Sequential,
    /// 随机增量——在 `[1, max]` 范围内随机跳动
    Random { max: u64 },
}

impl Default for IncrementStrategy {
    fn default() -> Self { Self::Sequential }
}

// ──── 单号规则 ────────────────────────────────────

/// 业务单号规则定义
///
/// 一条规则 = 格式模板 + 循环周期 + 计数策略 + 启用状态。
/// 可通过静态配置（`config.toml`）或运行时 API（`register_rule`）注册。
/// 禁用后的规则调用 `generate()` 将返回 `SerialError::RuleDisabled`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialRule {
    /// 规则唯一标识，如 "order"、"contract"、"purchase"
    pub key: String,
    /// 单号格式，如 "ORD{YYYY}{MM}{DD}{SEQ:8}"
    pub format: String,
    /// 循环周期
    #[serde(default)]
    pub cycle: CyclePeriod,
    /// 计数器初始值（每次周期重置后从此值开始）
    #[serde(default = "default_initial_value")]
    pub initial_value: u64,
    /// 增量策略
    #[serde(default)]
    pub step: IncrementStrategy,
    /// 是否启用（默认 true）。禁用后 `generate()` 返回错误。
    #[serde(default = "default_true")]
    pub is_enabled: bool,
}

fn default_initial_value() -> u64 { 1 }
fn default_true() -> bool { true }

impl SerialRule {
    /// 创建新规则（默认启用）
    pub fn new(key: impl Into<String>, format: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            format: format.into(),
            cycle: CyclePeriod::NoCycle,
            initial_value: 1,
            step: IncrementStrategy::Sequential,
            is_enabled: true,
        }
    }

    /// 设置循环周期
    pub fn with_cycle(mut self, cycle: CyclePeriod) -> Self {
        self.cycle = cycle;
        self
    }

    /// 设置初始值
    pub fn with_initial_value(mut self, val: u64) -> Self {
        self.initial_value = val;
        self
    }

    /// 设置增量策略
    pub fn with_step(mut self, step: IncrementStrategy) -> Self {
        self.step = step;
        self
    }
}

// ──── 生成记录 ────────────────────────────────────

/// 单号生成记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialRecord {
    /// 规则 key
    pub rule_key: String,
    /// 生成的单号
    pub serial_no: String,
    /// 当前计数器值
    pub counter: u64,
    /// 周期值（Daily→YYYYMMDD，NoCycle→空）
    pub cycle_value: String,
    /// 生成时间（RFC3339）
    pub created_at: String,
}

// ──── 格式引擎 ────────────────────────────────────

/// 格式片段
#[derive(Debug, Clone, PartialEq, Eq)]
enum FormatSegment {
    /// 原始文本
    Literal(String),
    /// {YYYY}（false）或 {YY}（true）
    Year(bool),
    /// {MM}
    Month,
    /// {DD}
    Day,
    /// {SEQ:n}
    Seq(u32),
    /// {RAND:n}
    Random(u32),
    /// {TS}（false）或 {TSMS}（true）
    Timestamp(bool),
}

/// 格式引擎：解析并渲染格式字符串
///
/// 预编译格式字符串为片段列表，生成单号时填充日期和序列号。
#[derive(Debug, Clone)]
pub struct FormatEngine {
    segments: Vec<FormatSegment>,
}

impl FormatEngine {
    /// 从格式字符串编译格式引擎
    ///
    /// # 错误
    ///
    /// 格式字符串含未知占位符或语法错误时返回 `SerialError::FormatError`。
    pub fn compile(format: &str) -> Result<Self, SerialError> {
        let mut segments = Vec::new();
        let chars: Vec<char> = format.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            if chars[i] == '{' {
                let mut j = i + 1;
                while j < len && chars[j] != '}' {
                    j += 1;
                }
                if j >= len {
                    return Err(SerialError::FormatError(format!(
                        "未闭合的 '{{': 位置 {} 格式 '{}'", i, format
                    )));
                }
                let token: String = chars[i + 1..j].iter().collect();
                segments.push(Self::parse_token(&token, format)?);
                i = j + 1;
            } else {
                let start = i;
                while i < len && chars[i] != '{' {
                    i += 1;
                }
                let lit: String = chars[start..i].iter().collect();
                if !lit.is_empty() {
                    segments.push(FormatSegment::Literal(lit));
                }
            }
        }

        Ok(Self { segments })
    }

    fn parse_token(token: &str, full_format: &str) -> Result<FormatSegment, SerialError> {
        match token {
            "YYYY" => Ok(FormatSegment::Year(false)),
            "YY" => Ok(FormatSegment::Year(true)),
            "MM" => Ok(FormatSegment::Month),
            "DD" => Ok(FormatSegment::Day),
            "TS" => Ok(FormatSegment::Timestamp(false)),
            "TSMS" => Ok(FormatSegment::Timestamp(true)),
            _ => {
                if let Some(w) = token.strip_prefix("SEQ:") {
                    let width: u32 = w.parse().map_err(|_| {
                        SerialError::FormatError(format!("SEQ 位数无效: '{}'", token))
                    })?;
                    if width == 0 {
                        return Err(SerialError::FormatError("SEQ 位数必须大于 0".into()));
                    }
                    Ok(FormatSegment::Seq(width))
                } else if let Some(w) = token.strip_prefix("RAND:") {
                    let width: u32 = w.parse().map_err(|_| {
                        SerialError::FormatError(format!("RAND 位数无效: '{}'", token))
                    })?;
                    if width == 0 {
                        return Err(SerialError::FormatError("RAND 位数必须大于 0".into()));
                    }
                    Ok(FormatSegment::Random(width))
                } else {
                    Err(SerialError::FormatError(format!(
                        "未知占位符: '{{{}}}' 在格式 '{}'", token, full_format
                    )))
                }
            }
        }
    }

    /// 渲染最终单号
    ///
    /// - `seq`: 序列号值
    /// - `ts`: 可选的时间戳（秒），无则使用当前时间
    pub fn render(&self, seq: u64, ts: Option<i64>) -> String {
        let now = ts.and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .unwrap_or_else(|| {
                Local::now().naive_local().and_utc()
            });

        let mut result = String::new();
        for seg in &self.segments {
            match seg {
                FormatSegment::Literal(s) => result.push_str(s),
                FormatSegment::Year(short) => {
                    let y = now.year();
                    if *short {
                        result.push_str(&format!("{:02}", y % 100));
                    } else {
                        result.push_str(&format!("{:04}", y));
                    }
                }
                FormatSegment::Month => result.push_str(&format!("{:02}", now.month())),
                FormatSegment::Day => result.push_str(&format!("{:02}", now.day())),
                FormatSegment::Seq(w) => {
                    result.push_str(&format!("{:0width$}", seq, width = *w as usize));
                }
                FormatSegment::Random(w) => {
                    let max = 10u64.pow(*w);
                    let rand_val: u64 = rand::random::<u64>() % max;
                    result.push_str(&format!("{:0width$}", rand_val, width = *w as usize));
                }
                FormatSegment::Timestamp(ms) => {
                    if *ms {
                        result.push_str(&format!("{}", now.timestamp_millis()));
                    } else {
                        result.push_str(&format!("{}", now.timestamp()));
                    }
                }
            }
        }
        result
    }
}

/// 检测序列号部分的宽度（用于溢出检查）
fn detect_seq_width(segments: &[FormatSegment]) -> u32 {
    for seg in segments {
        if let FormatSegment::Seq(w) = seg {
            return *w;
        }
    }
    20 // 默认 20 位
}

// ──── SerialGenerator Trait ───────────────────────

/// 单号生成器抽象 trait
///
/// 三种后端实现共享此接口，遵循 `TaskStorage` 设计模式：
/// - 业务方实现自己的后端（或使用内置的 Memory/Redis/PG 后端）
/// - 通过 `Arc<dyn SerialGenerator>` 注入到 `SerialPlugin`
///
/// # 示例
///
/// ```ignore
/// let backend = Arc::new(MemorySerialBackend::new());
/// backend.register_rule(rule).await?;
/// let order_no = backend.generate("order").await?;
/// ```
#[async_trait]
pub trait SerialGenerator: Send + Sync {
    /// 生成下一个单号
    async fn generate(&self, rule_key: &str) -> Result<String, SerialError>;

    /// 批量生成单号
    async fn batch_generate(
        &self, rule_key: &str, count: u32,
    ) -> Result<Vec<String>, SerialError>;

    /// 预览下一个单号（不消耗计数器）
    async fn peek(&self, rule_key: &str) -> Result<String, SerialError>;

    /// 运行时注册/更新一条规则
    async fn register_rule(&self, rule: SerialRule) -> Result<(), SerialError>;

    /// 运行时删除一条规则
    async fn remove_rule(&self, rule_key: &str) -> Result<(), SerialError>;

    /// 运行时启用一条规则
    async fn enable_rule(&self, rule_key: &str) -> Result<(), SerialError>;

    /// 运行时禁用一条规则
    async fn disable_rule(&self, rule_key: &str) -> Result<(), SerialError>;

    /// 查询生成记录（分页），返回 (记录列表, 总数)
    async fn query_records(
        &self, rule_key: &str, page: u64, page_size: u64,
    ) -> Result<(Vec<SerialRecord>, u64), SerialError>;

    /// 获取所有已注册的规则
    async fn list_rules(&self) -> Result<Vec<SerialRule>, SerialError>;
}

// ──── MemorySerialBackend ─────────────────────────

/// 计数器状态
#[derive(Debug, Clone)]
struct CounterState {
    value: u64,
    cycle: String,
}

/// 基于内存 + `tokio::sync::Mutex` 的单号生成后端
///
/// 适用于单机部署。计数器按 `{rule_key}:{cycle_value}` 维度分组存储。
///
/// # 示例
///
/// ```ignore
/// let backend = MemorySerialBackend::new();
/// backend.register_rule(SerialRule::new("order", "ORD{SEQ:6}")).await?;
/// let no = backend.generate("order").await?;
/// ```
pub struct MemorySerialBackend {
    rules: Mutex<HashMap<String, SerialRule>>,
    counters: Mutex<HashMap<String, CounterState>>,
    engines: Mutex<HashMap<String, FormatEngine>>,
    records: Mutex<Vec<SerialRecord>>,
    max_records: usize,
}

impl MemorySerialBackend {
    /// 创建内存后端（最多保留 10000 条记录）
    pub fn new() -> Self {
        Self {
            rules: Mutex::new(HashMap::new()),
            counters: Mutex::new(HashMap::new()),
            engines: Mutex::new(HashMap::new()),
            records: Mutex::new(Vec::new()),
            max_records: 10000,
        }
    }

    /// 设置最大记录数
    pub fn with_max_records(mut self, max: usize) -> Self {
        self.max_records = max;
        self
    }

    fn counter_key(rule_key: &str, cycle_value: &str) -> String {
        if cycle_value.is_empty() {
            rule_key.to_string()
        } else {
            format!("{}:{}", rule_key, cycle_value)
        }
    }

    async fn get_or_init_counter(
        &self, rule: &SerialRule,
    ) -> Result<(String, u64), SerialError> {
        let cycle_val = rule.cycle.current_value();
        let ck = Self::counter_key(&rule.key, &cycle_val);

        let mut counters = self.counters.lock().await;
        if let Some(state) = counters.get(&ck) {
            if state.cycle == cycle_val {
                return Ok((ck, state.value));
            }
        }

        let init = rule.initial_value;
        counters.insert(
            ck.clone(),
            CounterState { value: init, cycle: cycle_val },
        );
        Ok((ck, init))
    }
}

impl Default for MemorySerialBackend {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl SerialGenerator for MemorySerialBackend {
    async fn generate(&self, rule_key: &str) -> Result<String, SerialError> {
        let rules = self.rules.lock().await;
        let rule = rules
            .get(rule_key)
            .ok_or_else(|| SerialError::RuleNotFound(rule_key.to_string()))?
            .clone();
        drop(rules);

        // 检查规则是否启用
        if !rule.is_enabled {
            return Err(SerialError::RuleDisabled(rule_key.to_string()));
        }

        let engine = {
            let mut engines = self.engines.lock().await;
            if !engines.contains_key(rule_key) {
                let eng = FormatEngine::compile(&rule.format)?;
                engines.insert(rule_key.to_string(), eng);
            }
            engines.get(rule_key).unwrap().clone()
        };

        let (ck, current_val) = self.get_or_init_counter(&rule).await?;

        let next_val = match &rule.step {
            IncrementStrategy::Sequential => current_val + 1,
            IncrementStrategy::Random { max } => {
                let max = *max;
                let step: u64 = rand::random::<u64>() % if max > 0 { max } else { 1 } + 1;
                current_val + step
            }
        };

        let seq_width = detect_seq_width(&engine.segments);
        let max_val = 10u64.pow(seq_width).saturating_sub(1);
        if next_val > max_val {
            return Err(SerialError::CounterOverflow(format!(
                "规则 '{}' 序列号溢出: {} > 最大值 {}", rule_key, next_val, max_val
            )));
        }

        let serial_no = engine.render(current_val, None);

        {
            let mut counters = self.counters.lock().await;
            if let Some(state) = counters.get_mut(&ck) {
                state.value = next_val;
            }
        }

        {
            let mut records = self.records.lock().await;
            records.push(SerialRecord {
                rule_key: rule_key.to_string(),
                serial_no: serial_no.clone(),
                counter: current_val,
                cycle_value: rule.cycle.current_value(),
                created_at: Local::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
            });
            while records.len() > self.max_records {
                records.remove(0);
            }
        }

        Ok(serial_no)
    }

    async fn batch_generate(
        &self, rule_key: &str, count: u32,
    ) -> Result<Vec<String>, SerialError> {
        let mut result = Vec::with_capacity(count as usize);
        for _ in 0..count {
            result.push(self.generate(rule_key).await?);
        }
        Ok(result)
    }

    async fn peek(&self, rule_key: &str) -> Result<String, SerialError> {
        let rules = self.rules.lock().await;
        let rule = rules
            .get(rule_key)
            .ok_or_else(|| SerialError::RuleNotFound(rule_key.to_string()))?
            .clone();
        drop(rules);

        let engine = {
            let mut engines = self.engines.lock().await;
            if !engines.contains_key(rule_key) {
                let eng = FormatEngine::compile(&rule.format)?;
                engines.insert(rule_key.to_string(), eng);
            }
            engines.get(rule_key).unwrap().clone()
        };

        let cycle_val = rule.cycle.current_value();
        let ck = Self::counter_key(rule_key, &cycle_val);

        let current_val = {
            let counters = self.counters.lock().await;
            counters.get(&ck).map(|s| s.value).unwrap_or(rule.initial_value)
        };

        Ok(engine.render(current_val, None))
    }

    async fn register_rule(&self, rule: SerialRule) -> Result<(), SerialError> {
        FormatEngine::compile(&rule.format)?;
        let key = rule.key.clone();
        self.rules.lock().await.insert(key.clone(), rule);
        self.engines.lock().await.remove(&key);
        Ok(())
    }

    async fn remove_rule(&self, rule_key: &str) -> Result<(), SerialError> {
        self.rules.lock().await.remove(rule_key);
        self.engines.lock().await.remove(rule_key);
        Ok(())
    }

    async fn enable_rule(&self, rule_key: &str) -> Result<(), SerialError> {
        let mut rules = self.rules.lock().await;
        let rule = rules
            .get_mut(rule_key)
            .ok_or_else(|| SerialError::RuleNotFound(rule_key.to_string()))?;
        rule.is_enabled = true;
        Ok(())
    }

    async fn disable_rule(&self, rule_key: &str) -> Result<(), SerialError> {
        let mut rules = self.rules.lock().await;
        let rule = rules
            .get_mut(rule_key)
            .ok_or_else(|| SerialError::RuleNotFound(rule_key.to_string()))?;
        rule.is_enabled = false;
        Ok(())
    }

    async fn query_records(
        &self, rule_key: &str, page: u64, page_size: u64,
    ) -> Result<(Vec<SerialRecord>, u64), SerialError> {
        let records = self.records.lock().await;
        let filtered: Vec<&SerialRecord> = records
            .iter()
            .filter(|r| r.rule_key == rule_key)
            .collect();
        let total = filtered.len() as u64;
        let offset = ((page.saturating_sub(1)) * page_size) as usize;
        let data: Vec<SerialRecord> = filtered
            .into_iter().rev()
            .skip(offset).take(page_size as usize).cloned().collect();
        Ok((data, total))
    }

    async fn list_rules(&self) -> Result<Vec<SerialRule>, SerialError> {
        Ok(self.rules.lock().await.values().cloned().collect())
    }
}

// ──── 测试 ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_compile_basic() {
        let eng = FormatEngine::compile("ORD{YYYY}{MM}{DD}{SEQ:8}").unwrap();
        assert_eq!(eng.segments.len(), 5);
    }

    #[test]
    fn test_format_compile_all_types() {
        let eng = FormatEngine::compile("P{YY}{MM}{DD}-{RAND:4}{SEQ:6}").unwrap();
        assert_eq!(eng.segments.len(), 7);
    }

    #[test]
    fn test_format_render() {
        let eng = FormatEngine::compile("TST{YYYY}{MM}{DD}{SEQ:4}").unwrap();
        let ts = chrono::NaiveDate::from_ymd_opt(2024, 5, 24)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(eng.render(1, Some(ts)), "TST202405240001");
    }

    #[test]
    fn test_format_render_padding() {
        let eng = FormatEngine::compile("SEQ:{SEQ:6}").unwrap();
        assert_eq!(eng.render(123, None), "SEQ:000123");
    }

    #[test]
    fn test_format_parse_error() {
        assert!(FormatEngine::compile("X{UNKNOWN}").is_err());
    }

    #[tokio::test]
    async fn test_memory_generate_simple() {
        let backend = MemorySerialBackend::new();
        backend.register_rule(SerialRule::new("t", "T{SEQ:4}").with_initial_value(1)).await.unwrap();
        assert_eq!(backend.generate("t").await.unwrap(), "T0001");
        assert_eq!(backend.generate("t").await.unwrap(), "T0002");
    }

    #[tokio::test]
    async fn test_memory_daily_cycle() {
        let backend = MemorySerialBackend::new();
        let rule = SerialRule::new("d", "{SEQ:3}").with_cycle(CyclePeriod::Daily).with_initial_value(1);
        backend.register_rule(rule).await.unwrap();
        assert_eq!(backend.generate("d").await.unwrap(), "001");
        assert_eq!(backend.generate("d").await.unwrap(), "002");
    }

    #[tokio::test]
    async fn test_memory_peek() {
        let backend = MemorySerialBackend::new();
        backend.register_rule(SerialRule::new("p", "{SEQ:3}").with_initial_value(100)).await.unwrap();
        assert_eq!(backend.peek("p").await.unwrap(), "100");
        assert_eq!(backend.generate("p").await.unwrap(), "100");
    }

    #[tokio::test]
    async fn test_memory_register_remove() {
        let backend = MemorySerialBackend::new();
        backend.register_rule(SerialRule::new("tmp", "{SEQ:2}")).await.unwrap();
        assert!(backend.list_rules().await.unwrap().len() == 1);
        backend.remove_rule("tmp").await.unwrap();
        assert!(backend.list_rules().await.unwrap().is_empty());
        assert!(backend.generate("tmp").await.is_err());
    }
}
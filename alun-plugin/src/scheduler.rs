//! 定时任务插件：cron 表达式解析 + 到点执行
//!
//! 设计：
//! 1. `CronSchedule` 为纯解析/推算核心（无 IO、无时钟依赖），便于单测；
//! 2. `SchedulerPlugin` 在 `start()` 时按注册项派生调度循环，到点调用处理函数；
//! 3. `stop()` / `Drop` 通过 `watch` 通道发出关闭信号，等待调度循环退出（优雅关闭，
//!    与 `alun-web` 的关闭流程解耦）。

use alun_core::{Plugin, Result};
use async_trait::async_trait;
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// 任务处理函数：启动一次执行并返回其句柄
///
/// 用 `Arc` 包裹以便取出后**在锁外**调用（处理函数可能再次访问插件，避免死锁）。
type JobFn = Arc<dyn Fn() -> tokio::task::JoinHandle<()> + Send + Sync>;

/// 时间源：默认取系统时间（UTC），测试可注入受控时钟
pub type ClockFn = Arc<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// cron 表达式解析错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronError {
    expression: String,
    message: String,
}

impl CronError {
    /// 出错的整条表达式
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// 具体原因（含出错字段）
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CronError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cron 表达式 `{}` 非法: {}",
            self.expression, self.message
        )
    }
}

impl std::error::Error for CronError {}

/// 单个 cron 字段的取值集合（位图 + 该字段的合法区间）
#[derive(Debug, Clone, Copy)]
struct Field {
    bits: u64,
    min: u32,
    max: u32,
}

impl Field {
    fn empty(min: u32, max: u32) -> Self {
        Self { bits: 0, min, max }
    }

    fn insert(&mut self, v: u32) {
        debug_assert!((self.min..=self.max).contains(&v), "字段取值越界: {v}");
        self.bits |= 1u64 << v;
    }

    fn contains(&self, v: u32) -> bool {
        v < 64 && self.bits & (1u64 << v) != 0
    }

    /// 是否覆盖整个合法区间（用于日/周字段的 AND / OR 语义判定）
    fn is_full(&self) -> bool {
        (self.min..=self.max).all(|v| self.contains(v))
    }

    #[cfg(test)]
    fn values(&self) -> Vec<u32> {
        (self.min..=self.max)
            .filter(|v| self.contains(*v))
            .collect()
    }
}

/// cron 调度表达式（解析后的可推算形式）
///
/// 支持两种段数：
/// - **5 段**：`分 时 日 月 周`（分钟级精度，秒固定为 0）；
/// - **6 段**：`秒 分 时 日 月 周`（秒级精度，便于秒级任务与测试）。
///
/// 每段支持 `*`、`a`、`a,b`、`a-b`、`*/n`、`a-b/n`（另容忍 `a/n`，等价 `a-max/n`）。
/// 周日可用 `0` 或 `7`。日/周同时受限时按标准 cron 语义取“或”。
#[derive(Debug, Clone)]
pub struct CronSchedule {
    expression: String,
    seconds: Field,
    minutes: Field,
    hours: Field,
    days: Field,
    months: Field,
    weekdays: Field,
}

impl CronSchedule {
    /// 解析 cron 表达式；非法表达式返回 [`CronError`]
    pub fn parse(expr: &str) -> std::result::Result<Self, CronError> {
        let err = |msg: String| CronError {
            expression: expr.to_string(),
            message: msg,
        };

        let fields: Vec<&str> = expr.split_whitespace().collect();
        let (sec_spec, rest) = match fields.len() {
            5 => (None, &fields[..]),
            6 => (Some(fields[0]), &fields[1..]),
            n => {
                return Err(err(format!(
                    "需要 5 段（分 时 日 月 周）或 6 段（秒 分 时 日 月 周），实际 {} 段",
                    n
                )));
            }
        };

        let seconds = match sec_spec {
            Some(s) => parse_field(s, 0, 59).map_err(|m| err(format!("秒字段 {m}")))?,
            None => {
                let mut f = Field::empty(0, 59);
                f.insert(0);
                f
            }
        };

        Ok(Self {
            expression: expr.to_string(),
            seconds,
            minutes: parse_field(rest[0], 0, 59).map_err(|m| err(format!("分字段 {m}")))?,
            hours: parse_field(rest[1], 0, 23).map_err(|m| err(format!("时字段 {m}")))?,
            days: parse_field(rest[2], 1, 31).map_err(|m| err(format!("日字段 {m}")))?,
            months: parse_field(rest[3], 1, 12).map_err(|m| err(format!("月字段 {m}")))?,
            weekdays: parse_weekday(rest[4]).map_err(|m| err(format!("周字段 {m}")))?,
        })
    }

    /// 原始表达式
    pub fn expression(&self) -> &str {
        &self.expression
    }

    /// 严格晚于 `after` 的下一个触发时刻；无解（如 `0 0 30 2 *`）返回 `None`
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        // 起点：严格大于 after 的整秒时刻
        let mut t = after
            .with_nanosecond(0)?
            .checked_add_signed(chrono::Duration::seconds(1))?;
        // 搜索窗上限：闰年 2/29 等最长空档远小于 8 年
        let deadline = t.checked_add_signed(chrono::Duration::days(366 * 8))?;

        while t <= deadline {
            if !self.months.contains(t.month()) {
                t = next_month_start(t)?;
                continue;
            }
            if !self.day_matches(&t) {
                t = next_day_start(t)?;
                continue;
            }
            if !self.hours.contains(t.hour()) {
                t = t.checked_add_signed(chrono::Duration::seconds(
                    3600 - (t.minute() * 60 + t.second()) as i64,
                ))?;
                continue;
            }
            if !self.minutes.contains(t.minute()) {
                t = t.checked_add_signed(chrono::Duration::seconds(60 - t.second() as i64))?;
                continue;
            }
            if !self.seconds.contains(t.second()) {
                t = t.checked_add_signed(chrono::Duration::seconds(1))?;
                continue;
            }
            return Some(t);
        }
        None
    }

    /// 日匹配：日月同时受限时取“或”（标准 cron 语义），否则取“与”
    fn day_matches(&self, t: &DateTime<Utc>) -> bool {
        let dom = self.days.contains(t.day());
        let dow = self.weekdays.contains(t.weekday().num_days_from_sunday());
        match (self.days.is_full(), self.weekdays.is_full()) {
            (true, true) => true,
            (true, false) => dow,
            (false, true) => dom,
            (false, false) => dom || dow,
        }
    }
}

/// 解析单段：`*`、`a`、`a,b`、`a-b`、`*/n`、`a-b/n`
fn parse_field(spec: &str, min: u32, max: u32) -> std::result::Result<Field, String> {
    parse_field_wrapped(spec, min, max, None)
}

/// 解析周字段：允许 `7` 代表周日（回绕到 0）
fn parse_weekday(spec: &str) -> std::result::Result<Field, String> {
    parse_field_wrapped(spec, 0, 6, Some(7))
}

/// `wrap_at`：取值达到该值时回绕为 0（仅周字段的 `7 → 0` 使用）
fn parse_field_wrapped(
    spec: &str,
    min: u32,
    max: u32,
    wrap_at: Option<u32>,
) -> std::result::Result<Field, String> {
    if spec.is_empty() {
        return Err("为空".to_string());
    }
    let hard_max = wrap_at.unwrap_or(max);
    let mut field = Field::empty(min, max);

    for item in spec.split(',') {
        if item.is_empty() {
            return Err(format!("`{spec}` 含空列表项"));
        }
        let (range_part, step, has_step) = match item.split_once('/') {
            Some((r, s)) => {
                if s.contains('/') {
                    return Err(format!("`{item}` 步长写法非法"));
                }
                let n: u32 = s.parse().map_err(|_| format!("`{item}` 步长非法: `{s}`"))?;
                if n == 0 {
                    return Err(format!("`{item}` 步长不能为 0"));
                }
                (r, n, true)
            }
            None => (item, 1, false),
        };

        let (start, end) = if range_part == "*" {
            (min, hard_max)
        } else if let Some((a, b)) = range_part.split_once('-') {
            (
                bound(a, min, hard_max, item)?,
                bound(b, min, hard_max, item)?,
            )
        } else {
            let v = bound(range_part, min, hard_max, item)?;
            // `a/n`：与主流实现一致，视为 `a-max/n`；裸 `a` 即单值
            if has_step { (v, hard_max) } else { (v, v) }
        };
        if start > end {
            return Err(format!("`{item}` 范围起止颠倒"));
        }

        let mut v = start as u64;
        while v <= end as u64 {
            field.insert(match wrap_at {
                Some(w) if v >= w as u64 => (v - w as u64) as u32,
                _ => v as u32,
            });
            v += step as u64;
        }
    }
    Ok(field)
}

fn bound(token: &str, min: u32, max: u32, item: &str) -> std::result::Result<u32, String> {
    let v: u32 = token
        .parse()
        .map_err(|_| format!("`{item}` 取值非法: `{token}`"))?;
    if v < min || v > max {
        return Err(format!("`{item}` 取值 {v} 越界（合法区间 {min}-{max}）"));
    }
    Ok(v)
}

fn next_day_start(t: DateTime<Utc>) -> Option<DateTime<Utc>> {
    Some(t.date_naive().succ_opt()?.and_hms_opt(0, 0, 0)?.and_utc())
}

fn next_month_start(t: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let (year, month) = if t.month() == 12 {
        (t.year() + 1, 1)
    } else {
        (t.year(), t.month() + 1)
    };
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0).single()
}

struct ScheduledJob {
    cron: String,
    description: String,
    /// 处理函数：允许为空（只登记 cron 未绑定处理函数时到点跳过，不 panic）
    runner: Mutex<Option<JobFn>>,
}

/// 调度运行态（`start()` 后有效）
#[derive(Default)]
struct RuntimeState {
    running: bool,
    /// 关闭信号：真实值 = 已请求关闭
    shutdown: Option<tokio::sync::watch::Sender<bool>>,
    /// 各调度循环句柄（含任务名，便于日志）
    handles: Vec<(String, tokio::task::JoinHandle<()>)>,
}

/// 定时任务插件：cron 表达式调度
///
/// 支持注册/移除/手动触发/列举任务；`start()` 后按注册项派生调度循环，到点调用处理函数。
///
/// # 示例
///
/// ```ignore
/// let scheduler = SchedulerPlugin::new();
/// scheduler.register("cleanup", "0 */5 * * * *", "每5分钟清理", || {
///     tokio::spawn(async { cleanup_task().await })
/// });
/// // 交给 PluginManager：start_all() 后自动到点触发，stop_all() 时优雅关闭
/// ```
pub struct SchedulerPlugin {
    /// 任务注册表（name → ScheduledJob）
    jobs: Arc<RwLock<HashMap<String, Arc<ScheduledJob>>>>,
    /// 时间源（默认系统时钟）
    clock: ClockFn,
    /// 调度运行态
    state: Arc<Mutex<RuntimeState>>,
}

impl SchedulerPlugin {
    /// 创建空的调度器（系统时钟）
    pub fn new() -> Self {
        Self::with_clock(Arc::new(Utc::now))
    }

    /// 创建空的调度器并注入时间源（测试/受控时钟）
    pub fn with_clock(clock: ClockFn) -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            clock,
            state: Arc::new(Mutex::new(RuntimeState::default())),
        }
    }

    /// 注册定时任务（登记 cron 并绑定处理函数）
    ///
    /// `cron` 为 5 段（分 时 日 月 周）或 6 段（秒 分 时 日 月 周）表达式，
    /// 非法表达式在 `start()` 时返回错误。
    ///
    /// ```ignore
    /// scheduler.register("cleanup", "0 */5 * * * *", "每5分钟清理", || {
    ///     tokio::spawn(async { cleanup_task().await })
    /// });
    /// ```
    pub fn register<F>(&self, name: &str, cron: &str, desc: &str, runner: F)
    where
        F: Fn() -> tokio::task::JoinHandle<()> + Send + Sync + 'static,
    {
        self.insert(name, cron, desc, Some(Arc::new(runner)));
    }

    /// 只登记调度表达式，稍后可用 [`SchedulerPlugin::attach`] 绑定处理函数
    ///
    /// 未绑定处理函数的注册项仍会按期触发，但只记录告警（不 panic）。
    pub fn register_scheduled(&self, name: &str, cron: &str, desc: &str) {
        self.insert(name, cron, desc, None);
    }

    /// 为已登记的调度项绑定/替换处理函数；名字不存在时返回 `false`
    pub fn attach<F>(&self, name: &str, runner: F) -> bool
    where
        F: Fn() -> tokio::task::JoinHandle<()> + Send + Sync + 'static,
    {
        match self.jobs.read().get(name) {
            Some(job) => {
                *job.runner.lock() = Some(Arc::new(runner));
                true
            }
            None => false,
        }
    }

    fn insert(&self, name: &str, cron: &str, desc: &str, runner: Option<JobFn>) {
        self.jobs.write().insert(
            name.to_string(),
            Arc::new(ScheduledJob {
                cron: cron.to_string(),
                description: desc.to_string(),
                runner: Mutex::new(runner),
            }),
        );
    }

    /// 移除定时任务（运行中会停止其调度循环）
    pub fn remove(&self, name: &str) {
        self.jobs.write().remove(name);
    }

    /// 列出所有注册的任务
    pub fn list(&self) -> Vec<(String, String, String)> {
        self.jobs
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), v.cron.clone(), v.description.clone()))
            .collect()
    }

    /// 手动触发指定的定时任务（未注册或未绑定处理函数返回 `None`）
    pub fn trigger(&self, name: &str) -> Option<tokio::task::JoinHandle<()>> {
        let job = self.jobs.read().get(name).cloned()?;
        let runner = job.runner.lock().clone();
        runner.map(|f| f())
    }

    /// 插件是否处于调度中（`start()` 之后、`stop()` 之前）
    pub fn is_running(&self) -> bool {
        self.state.lock().running
    }
}

#[async_trait]
impl Plugin for SchedulerPlugin {
    fn name(&self) -> &str {
        "scheduler"
    }

    /// 解析全部注册项并派生调度循环；任一表达式非法则返回错误且不启动任何调度
    async fn start(&self) -> Result<()> {
        if self.state.lock().running {
            return Ok(());
        }
        // 先整体校验表达式，避免半启动
        let compiled: Vec<(String, Arc<ScheduledJob>, CronSchedule)> = {
            let jobs = self.jobs.read();
            let mut out = Vec::with_capacity(jobs.len());
            for (name, job) in jobs.iter() {
                let schedule = CronSchedule::parse(&job.cron)
                    .map_err(|e| alun_core::Error::Config(format!("定时任务 `{name}` 的 {e}")))?;
                out.push((name.clone(), Arc::clone(job), schedule));
            }
            out
        };

        let mut state = self.state.lock();
        if state.running {
            return Ok(());
        }
        let (tx, _) = tokio::sync::watch::channel(false);
        let mut handles = Vec::with_capacity(compiled.len());

        for (name, job, schedule) in compiled {
            if schedule.next_after((self.clock)()).is_none() {
                tracing::warn!("定时任务 {} [{}] 永不触发，跳过调度", name, job.cron);
                continue;
            }
            let rx = tx.subscribe();
            let clock = Arc::clone(&self.clock);
            let jobs = Arc::clone(&self.jobs);
            let task_name = name.clone();
            tracing::info!("定时任务 {} [{}] 已排程", name, schedule.expression());
            let handle = tokio::spawn(async move {
                run_job(task_name, job, schedule, jobs, clock, rx).await;
            });
            handles.push((name, handle));
        }

        state.running = true;
        state.shutdown = Some(tx);
        state.handles = handles;
        tracing::info!("定时任务插件: {} 个任务已启动", self.jobs.read().len());
        Ok(())
    }

    /// 优雅关闭：发信号并等待全部调度循环退出
    async fn stop(&self) -> Result<()> {
        let (tx, handles) = {
            let mut state = self.state.lock();
            if !state.running && state.shutdown.is_none() {
                return Ok(());
            }
            state.running = false;
            (state.shutdown.take(), std::mem::take(&mut state.handles))
        };
        if let Some(tx) = tx {
            let _ = tx.send(true);
        }
        for (name, handle) in handles {
            if handle.await.is_err() {
                tracing::warn!("定时任务 {} 的调度循环异常退出", name);
            }
        }
        tracing::info!("定时任务插件: 已停止");
        Ok(())
    }
}

/// 单个任务的调度循环：睡到下一个触发时刻 → 调用处理函数 → 继续
async fn run_job(
    name: String,
    job: Arc<ScheduledJob>,
    schedule: CronSchedule,
    jobs: Arc<RwLock<HashMap<String, Arc<ScheduledJob>>>>,
    clock: ClockFn,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    loop {
        let now = clock();
        let Some(next) = schedule.next_after(now) else {
            tracing::warn!(
                "定时任务 {} [{}] 无后续触发时刻，调度循环退出",
                name,
                job.cron
            );
            return;
        };
        let wait = (next - now).to_std().unwrap_or(Duration::ZERO);

        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = shutdown.changed() => break,
        }
        if *shutdown.borrow() {
            break;
        }
        // 等待期间被 remove：不再触发
        if !jobs.read().contains_key(&name) {
            tracing::info!("定时任务 {} 已移除，调度循环退出", name);
            break;
        }

        let runner = job.runner.lock().clone();
        match runner {
            Some(runner) => {
                tracing::debug!("定时任务 {} 触发", name);
                // fire-and-forget：runner 内部通常已 `tokio::spawn`，此处显式 drop
                // 返回的 JoinHandle 使任务脱离本循环继续执行
                std::mem::drop(runner());
            }
            None => tracing::warn!("定时任务 {} 未绑定处理函数，跳过本次触发", name),
        }
    }
    tracing::info!("定时任务 {} 调度循环已停止", name);
}

impl Drop for SchedulerPlugin {
    /// 与外部关闭流程解耦：即使未调用 `stop()`，drop 时也发出关闭信号并终止调度循环
    fn drop(&mut self) {
        let mut state = self.state.lock();
        state.running = false;
        if let Some(tx) = state.shutdown.take() {
            let _ = tx.send(true);
        }
        for (_, handle) in state.handles.drain(..) {
            handle.abort();
        }
    }
}

impl Default for SchedulerPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn ts(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    /// 轮询间隔
    const POLL: Duration = Duration::from_millis(25);

    /// 可注入时钟：基于 `tokio::time::Instant`（便于与暂停/推进的时间轴对齐），
    /// 默认偏移取当前系统时间
    fn virtual_clock() -> ClockFn {
        let t0 = tokio::time::Instant::now();
        let base = Utc::now();
        Arc::new(move || {
            let d = tokio::time::Instant::now().saturating_duration_since(t0);
            base + chrono::Duration::from_std(d).unwrap_or_else(|_| chrono::Duration::zero())
        })
    }

    /// 冻结时钟（永远停在同一时刻）
    fn frozen_clock(at: DateTime<Utc>) -> ClockFn {
        Arc::new(move || at)
    }

    async fn wait_for(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        while tokio::time::Instant::now() < deadline {
            if cond() {
                return true;
            }
            tokio::time::sleep(POLL).await;
        }
        cond()
    }

    fn values(spec: &str, min: u32, max: u32) -> Vec<u32> {
        parse_field(spec, min, max).unwrap().values()
    }

    #[test]
    fn parses_star_comma_range_and_step() {
        assert_eq!(values("*", 0, 59), (0..=59).collect::<Vec<_>>());
        assert_eq!(
            values("*/5", 0, 59),
            vec![0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55]
        );
        assert_eq!(values("1-5", 0, 59), vec![1, 2, 3, 4, 5]);
        assert_eq!(values("0,30", 0, 59), vec![0, 30]);
        assert_eq!(values("10-20/5", 0, 59), vec![10, 15, 20]);
        assert_eq!(values("0/15", 0, 59), vec![0, 15, 30, 45]);
        // 周字段：7 回绕为周日 0
        assert_eq!(parse_weekday("7").unwrap().values(), vec![0]);
        assert_eq!(parse_weekday("0,7").unwrap().values(), vec![0]);
        assert_eq!(parse_weekday("5-7").unwrap().values(), vec![0, 5, 6]);
    }

    #[test]
    fn schedule_parses_five_and_six_fields() {
        let five = CronSchedule::parse("*/5 1-5 * * 1,3").unwrap();
        assert_eq!(five.expression(), "*/5 1-5 * * 1,3");
        // 5 段：秒固定为 0
        assert_eq!(five.seconds.values(), vec![0]);
        assert_eq!(
            five.minutes.values(),
            vec![0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55]
        );
        assert_eq!(five.hours.values(), vec![1, 2, 3, 4, 5]);
        assert_eq!(five.weekdays.values(), vec![1, 3]);
        assert_eq!(five.days.values(), (1..=31).collect::<Vec<_>>());

        // 6 段：秒级
        let six = CronSchedule::parse("0 */5 * * * *").unwrap();
        assert_eq!(six.seconds.values(), vec![0]);
        assert_eq!(
            six.minutes.values(),
            vec![0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55]
        );
        assert_eq!(six.hours.values(), (0..=23).collect::<Vec<_>>());
    }

    #[test]
    fn rejects_invalid_expressions() {
        for bad in [
            "",
            "   ",
            "* * * *",
            "* * * * * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * 32 * *",
            "* * * 13 *",
            "* * * * 8",
            "*/0 * * * *",
            "5-1 * * * *",
            "a * * * *",
            "1,,2 * * * *",
            "1-2-3 * * * *",
            "* * * * *,",
            "* * * * -1",
        ] {
            let e = CronSchedule::parse(bad).unwrap_err();
            assert_eq!(e.expression(), bad, "表达式应原样回填: {bad}");
            assert!(!e.message().is_empty(), "应说明原因: {bad}");
            assert!(e.to_string().contains("非法"), "Display 应可读: {e}");
        }
    }

    #[test]
    fn next_after_step_range_and_daily() {
        // */5：从 00:05:30 到 00:10:00
        let s = CronSchedule::parse("*/5 * * * *").unwrap();
        assert_eq!(
            s.next_after(ts(2026, 1, 1, 0, 5, 30)),
            Some(ts(2026, 1, 1, 0, 10, 0))
        );
        // 边界：刚好落在触发时刻时取“严格之后”
        assert_eq!(
            s.next_after(ts(2026, 1, 1, 0, 55, 0)),
            Some(ts(2026, 1, 1, 1, 0, 0))
        );

        // 1-5：跨小时/跨日
        let r = CronSchedule::parse("1-5 * * * *").unwrap();
        assert_eq!(
            r.next_after(ts(2026, 1, 1, 0, 0, 0)),
            Some(ts(2026, 1, 1, 0, 1, 0))
        );
        assert_eq!(
            r.next_after(ts(2026, 1, 1, 0, 6, 0)),
            Some(ts(2026, 1, 1, 1, 1, 0))
        );
        assert_eq!(
            r.next_after(ts(2026, 1, 1, 23, 6, 0)),
            Some(ts(2026, 1, 2, 0, 1, 0))
        );

        // 0 3 * * *：每天 03:00
        let d = CronSchedule::parse("0 3 * * *").unwrap();
        assert_eq!(
            d.next_after(ts(2026, 1, 1, 4, 0, 0)),
            Some(ts(2026, 1, 2, 3, 0, 0))
        );
        assert_eq!(
            d.next_after(ts(2026, 12, 31, 23, 59, 59)),
            Some(ts(2027, 1, 1, 3, 0, 0))
        );

        // 跨月
        let m = CronSchedule::parse("0 0 1 * *").unwrap();
        assert_eq!(
            m.next_after(ts(2026, 1, 15, 0, 0, 0)),
            Some(ts(2026, 2, 1, 0, 0, 0))
        );

        // 闰年 2/29：2026-03-01 之后是 2028-02-29
        let leap = CronSchedule::parse("0 0 29 2 *").unwrap();
        assert_eq!(
            leap.next_after(ts(2026, 3, 1, 0, 0, 0)),
            Some(ts(2028, 2, 29, 0, 0, 0))
        );
    }

    #[test]
    fn next_after_seconds_weekday_and_impossible() {
        // 6 段：秒级
        let s = CronSchedule::parse("* * * * * *").unwrap();
        assert_eq!(
            s.next_after(ts(2026, 1, 1, 0, 0, 0)),
            Some(ts(2026, 1, 1, 0, 0, 1))
        );
        let q = CronSchedule::parse("*/15 * * * * *").unwrap();
        assert_eq!(
            q.next_after(ts(2026, 1, 1, 0, 0, 7)),
            Some(ts(2026, 1, 1, 0, 0, 15))
        );

        // 周字段：2026-01-01 是周四 → 下一个周一为 01-05
        let mon = CronSchedule::parse("0 0 * * 1").unwrap();
        assert_eq!(
            mon.next_after(ts(2026, 1, 1, 0, 0, 0)),
            Some(ts(2026, 1, 5, 0, 0, 0))
        );
        // 7 == 周日
        let sun = CronSchedule::parse("0 0 * * 7").unwrap();
        assert_eq!(
            sun.next_after(ts(2026, 1, 1, 0, 0, 0)),
            Some(ts(2026, 1, 4, 0, 0, 0))
        );
        // 日/周同时受限 → 取“或”：01-05（周一）早于 01-13
        let either = CronSchedule::parse("0 0 13 * 1").unwrap();
        assert_eq!(
            either.next_after(ts(2026, 1, 1, 0, 0, 0)),
            Some(ts(2026, 1, 5, 0, 0, 0))
        );

        // 永不触发：2 月没有 30 日
        let never = CronSchedule::parse("0 0 30 2 *").unwrap();
        assert_eq!(never.next_after(ts(2026, 1, 1, 0, 0, 0)), None);
    }

    #[tokio::test]
    async fn fires_at_due_time_and_stop_prevents_further_fires() {
        let hits = Arc::new(AtomicUsize::new(0));
        // 秒级表达式（6 段）+ 可注入时钟：无需等待分钟级周期即可验证真实调度
        let plugin = SchedulerPlugin::with_clock(virtual_clock());
        let h = Arc::clone(&hits);
        plugin.register("tick", "* * * * * *", "每秒", move || {
            h.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async {})
        });

        plugin.start().await.expect("启动失败");
        assert!(plugin.is_running());
        assert!(
            wait_for(|| hits.load(Ordering::SeqCst) > 0, Duration::from_secs(5)).await,
            "应在到点后触发处理函数"
        );

        plugin.stop().await.expect("停止失败");
        assert!(!plugin.is_running());
        let n = hits.load(Ordering::SeqCst);
        // 停止后跨越 2 个触发周期
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert_eq!(hits.load(Ordering::SeqCst), n, "stop 后不应再触发");
    }

    #[tokio::test]
    async fn frozen_clock_does_not_fire_before_due_time() {
        let hits = Arc::new(AtomicUsize::new(0));
        let plugin = SchedulerPlugin::with_clock(frozen_clock(ts(2026, 1, 1, 0, 0, 0)));
        let h = Arc::clone(&hits);
        plugin.register("hourly", "0 * * * *", "每小时", move || {
            h.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async {})
        });

        plugin.start().await.expect("启动失败");
        // 冻结时钟永远到不了下一个整点 → 不应触发
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(hits.load(Ordering::SeqCst), 0, "未到点不应触发");
        // 手动触发不受时钟影响
        assert!(plugin.trigger("hourly").is_some());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        plugin.stop().await.expect("停止失败");
    }

    #[tokio::test]
    async fn drop_without_stop_cancels_scheduling() {
        let hits = Arc::new(AtomicUsize::new(0));
        let plugin = SchedulerPlugin::with_clock(virtual_clock());
        let h = Arc::clone(&hits);
        plugin.register("tick", "* * * * * *", "每秒", move || {
            h.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async {})
        });
        plugin.start().await.expect("启动失败");
        assert!(
            wait_for(|| hits.load(Ordering::SeqCst) > 0, Duration::from_secs(5)).await,
            "应在到点后触发处理函数"
        );

        // 不显式 stop()，仅依赖 Drop 钩子（与外部关闭流程解耦）
        drop(plugin);
        let n = hits.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert_eq!(hits.load(Ordering::SeqCst), n, "Drop 后不应再触发");
    }

    #[tokio::test]
    async fn start_rejects_invalid_cron_and_schedules_nothing() {
        let hits = Arc::new(AtomicUsize::new(0));
        let plugin = SchedulerPlugin::new();
        let h = Arc::clone(&hits);
        plugin.register("ok", "* * * * *", "合法", move || {
            h.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async {})
        });
        plugin.register("bad", "61 * * * *", "非法", || tokio::spawn(async {}));

        let err = plugin.start().await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("bad") && msg.contains("61"),
            "错误应指出任务名与越界取值: {msg}"
        );
        assert!(!plugin.is_running(), "非法表达式时不应进入运行态");
        assert_eq!(hits.load(Ordering::SeqCst), 0, "不应半启动合法任务");
    }

    #[tokio::test]
    async fn registration_without_handler_does_not_panic() {
        let plugin = SchedulerPlugin::with_clock(virtual_clock());
        plugin.register_scheduled("orphan", "* * * * * *", "仅登记 cron");
        assert_eq!(
            plugin.list(),
            vec![(
                "orphan".to_string(),
                "* * * * * *".to_string(),
                "仅登记 cron".to_string()
            )]
        );

        plugin.start().await.expect("启动失败");
        // 至少经历一次到点触发（无处理函数时仅告警，不得 panic）
        tokio::time::sleep(Duration::from_millis(1200)).await;

        // 未注册名字：不 panic，返回 None / false
        assert!(plugin.trigger("missing").is_none());
        assert!(!plugin.attach("missing", || tokio::spawn(async {})));
        assert!(
            plugin.trigger("orphan").is_none(),
            "未绑定处理函数时手动触发返回 None"
        );

        // 运行中补绑处理函数后即可手动触发
        let hits = Arc::new(AtomicUsize::new(0));
        let h = Arc::clone(&hits);
        assert!(plugin.attach("orphan", move || {
            h.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async {})
        }));
        assert!(plugin.trigger("orphan").is_some());
        assert!(
            hits.load(Ordering::SeqCst) >= 1,
            "补绑后手动触发应执行处理函数"
        );

        plugin.stop().await.expect("停止失败");
    }

    #[tokio::test]
    async fn remove_stops_scheduling_and_list_keeps_api() {
        let hits = Arc::new(AtomicUsize::new(0));
        let plugin = SchedulerPlugin::new();
        let h = Arc::clone(&hits);
        plugin.register("nightly", "0 3 * * *", "每日 3 点", move || {
            h.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async {})
        });
        plugin.register("weekly", "0 0 * * 1", "每周一", || tokio::spawn(async {}));
        assert_eq!(plugin.list().len(), 2);

        plugin.start().await.expect("启动失败");
        // 每日 3 点：真实时钟下不会自触发，仅手动触发一次
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        assert!(plugin.trigger("nightly").is_some());
        assert!(wait_for(|| hits.load(Ordering::SeqCst) == 1, Duration::from_secs(1)).await);

        plugin.remove("weekly");
        assert_eq!(plugin.list().len(), 1);
        assert!(plugin.trigger("weekly").is_none());

        plugin.stop().await.expect("停止失败");
        // 重复 stop / 未启动时 stop 均为幂等成功
        plugin.stop().await.expect("重复停止应成功");
    }
}

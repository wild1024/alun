//! F10 · 缓存侧可观测性埋点
//!
//! 只在 `metrics` feature 下编译。埋点挂在 [`SharedCache`](crate::SharedCache)
//! 上——它是框架**唯一的缓存入口**（`LocalCache` / `RedisCache` 都经它分派），
//! 因此一处埋点即可覆盖全部实现，无需在每个后端各写一遍。
//!
//! # 指标清单
//!
//! | 指标 | 类型 | 标签 | 含义 |
//! |---|---|---|---|
//! | `alun_cache_operations_total` | counter | `op` `result` | 操作计数；`result` ∈ `hit` / `miss` / `ok` / `error` |
//! | `alun_cache_operation_duration_seconds` | histogram | `op` | 操作耗时（秒） |
//!
//! # 为什么 `hit` / `miss` 只在 `get` 上分
//!
//! 命中率是唯一有业务含义的缓存指标，且只有读操作能定义"命中"。把
//! `set` / `del` 的结果也塞进同一个 `result` 标签（值取 `ok` / `error`）
//! 是为了**共用一条时间序列**——否则命中率的分母要跨两条序列相除，
//! 在 PromQL 里既别扭又容易写错。

use alun_core::Result;
use std::time::Instant;

/// 操作计数
pub const CACHE_OPERATIONS_TOTAL: &str = "alun_cache_operations_total";
/// 操作耗时（秒，直方图）
pub const CACHE_OPERATION_DURATION_SECONDS: &str = "alun_cache_operation_duration_seconds";

/// 注册指标元数据（单位 + 说明），让 Prometheus 的文本输出带上 `# HELP`。
///
/// 幂等；由 [`observe`] 在首次调用时自动触发。
pub fn describe() {
    metrics::describe_counter!(
        CACHE_OPERATIONS_TOTAL,
        metrics::Unit::Count,
        "缓存操作次数；result=hit/miss 仅出现在读操作上，据此可算命中率"
    );
    metrics::describe_histogram!(
        CACHE_OPERATION_DURATION_SECONDS,
        metrics::Unit::Seconds,
        "缓存操作耗时（秒）"
    );
}

/// 记录一次缓存操作的结果与耗时。
pub fn observe(op: &'static str, start: Instant, result: &'static str) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(describe);
    metrics::counter!(CACHE_OPERATIONS_TOTAL, "op" => op, "result" => result).increment(1);
    metrics::histogram!(CACHE_OPERATION_DURATION_SECONDS, "op" => op)
        .record(start.elapsed().as_secs_f64());
}

/// 记录一次**读**操作，自动判定命中 / 未命中 / 失败。
pub fn observe_get<T>(start: Instant, out: &Result<Option<T>>) {
    let result = match out {
        Ok(Some(_)) => "hit",
        Ok(None) => "miss",
        Err(_) => "error",
    };
    observe("get", start, result);
}

/// 记录一次非读操作的结果（成功 `ok` / 失败 `error`）。
pub fn observe_result<E>(op: &'static str, start: Instant, out: &Result<E>) {
    observe(op, start, if out.is_ok() { "ok" } else { "error" });
}

//! F10 · DB 侧可观测性埋点
//!
//! 只在 `metrics` feature 下编译；未安装 recorder 时 [`metrics`] 的全部宏是
//! **no-op**（空实现，无分配），因此埋点可以常驻代码而不必再套一层开关。
//!
//! # 指标清单
//!
//! | 指标 | 类型 | 标签 | 含义 |
//! |---|---|---|---|
//! | `alun_db_queries_total` | counter | `op` `table` `status` | 语句计数（按成功/失败拆分） |
//! | `alun_db_query_duration_seconds` | histogram | `op` `table` | 语句耗时（秒） |
//! | `alun_tenant_scope_missing_total` | counter | `table` `op` | **安全指标**：租户模型在无租户上下文时被访问（fail-closed 拦截次数） |
//! | `alun_raw_sql_tenant_unbound_total` | counter | `table` | **安全指标**：`Scoped` 下原生 SQL 未通过「占位 + 显式承诺」守卫 |
//! | `alun_db_raw_sql_total` | counter | `kind` | 受信任原始 SQL 的调用次数（定义在 `alun-db-core`，与 F15 联动） |
//!
//! # 为什么安全指标是**一等公民**
//!
//! 前两个是"系统健不健康"，后两个是"隔离有没有被绕过"。后者在单测里永远是
//! 绿的（fail-closed 拦截了就是正确行为），只有靠**计数**才能在生产上发现
//! "某个接口每次请求都在撞守卫"——那通常意味着一处漏配的 `tenant_scope`，
//! 或一个本该 `cross_tenant` 却忘了标注的模型。

use std::future::Future;
use std::time::Instant;

use alun_db_core::error::Result;

/// 语句计数
pub const DB_QUERIES_TOTAL: &str = "alun_db_queries_total";
/// 语句耗时（秒，直方图）
pub const DB_QUERY_DURATION_SECONDS: &str = "alun_db_query_duration_seconds";
/// 安全指标：租户模型在无租户上下文时被访问（fail-closed）
pub const TENANT_SCOPE_MISSING_TOTAL: &str = "alun_tenant_scope_missing_total";
/// 安全指标：`Scoped` 上下文下原生 SQL 未通过租户绑定守卫
pub const RAW_SQL_TENANT_UNBOUND_TOTAL: &str = "alun_raw_sql_tenant_unbound_total";

/// 无表名可用的场景（原生 SQL 通道）使用的占位标签值。
///
/// 刻意用显式占位而非空串：空 label 在 Prometheus 里容易被误读成"缺失"。
pub const TABLE_RAW: &str = "raw";

/// 注册指标元数据（单位 + 说明），让 Prometheus 的文本输出带上 `# HELP`。
///
/// **幂等**，且由各 `observe_*` 在首次调用时自动触发（见 [`describe_once`]）；
/// 需要提前注册（比如进程启动即暴露 `/metrics`）时也可显式调用。
pub fn describe() {
    // `alun-db-core` 的 `alun_db_raw_sql_total`（F15 逃生舱计数）一并注册
    alun_db_core::metrics::describe();
    metrics::describe_counter!(
        DB_QUERIES_TOTAL,
        metrics::Unit::Count,
        "数据库语句执行次数，按 op/table/status 拆分"
    );
    metrics::describe_histogram!(
        DB_QUERY_DURATION_SECONDS,
        metrics::Unit::Seconds,
        "数据库语句执行耗时（秒）"
    );
    metrics::describe_counter!(
        TENANT_SCOPE_MISSING_TOTAL,
        metrics::Unit::Count,
        "安全指标：租户模型在无租户上下文时被访问（已被 fail-closed 拦截）"
    );
    metrics::describe_counter!(
        RAW_SQL_TENANT_UNBOUND_TOTAL,
        metrics::Unit::Count,
        "安全指标：Scoped 上下文下原生 SQL 未通过租户绑定守卫"
    );
}

/// 首次埋点时注册元数据；之后只剩一次原子读，不影响热路径。
#[inline]
fn describe_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(describe);
}

/// 记录一次语句执行的结果与耗时。
///
/// - `op`：`query` / `exec` / `script` / `select` / `tx_query` / `tx_exec`
/// - `table`：模型表名；无法判定时传 [`TABLE_RAW`]。
///
/// 标签值一律取 `&'static str`（与 `metrics` 的 `SharedString` 对齐）：表名来
/// 自 `Model::TABLE` 常量，天然是静态的，因此埋点**不会**为每个标签分配字符串。
pub fn observe_query(op: &'static str, table: &'static str, start: Instant, ok: bool) {
    describe_once();
    let status = if ok { "ok" } else { "error" };
    metrics::counter!(DB_QUERIES_TOTAL, "op" => op, "table" => table, "status" => status)
        .increment(1);
    metrics::histogram!(DB_QUERY_DURATION_SECONDS, "op" => op, "table" => table)
        .record(start.elapsed().as_secs_f64());
}

/// 安全指标：租户模型在无租户上下文时被访问（被 fail-closed 拦截）。
///
/// 在 `scope_for` / `join_scope_for` / `ensure_raw_allowed` 的 `None` 分支调用。
pub fn observe_tenant_scope_missing(table: &'static str, op: &'static str) {
    describe_once();
    metrics::counter!(TENANT_SCOPE_MISSING_TOTAL, "table" => table, "op" => op).increment(1);
}

/// 安全指标：`Scoped` 上下文下原生 SQL 未通过「占位 + 显式承诺完整性」守卫。
pub fn observe_raw_sql_unbound(table: &'static str) {
    describe_once();
    metrics::counter!(RAW_SQL_TENANT_UNBOUND_TOTAL, "table" => table).increment(1);
}

/// 包一层 future，执行结束后自动记录耗时与成败。
///
/// 供 ORM 层（`ModelQuery`）使用——那里拿得到模型表名，而 `Db` 门面层只有
/// SQL 文本。
pub async fn instrument<T>(
    op: &'static str,
    table: &'static str,
    fut: impl Future<Output = Result<T>>,
) -> Result<T> {
    let start = Instant::now();
    let out = fut.await;
    observe_query(op, table, start, out.is_ok());
    out
}

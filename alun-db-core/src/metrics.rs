//! F10 · `alun-db-core` 侧指标元数据
//!
//! 本 crate 只有一个埋点：受信任原始 SQL 的**调用计数**
//! （`alun_db_raw_sql_total`，见 [`Expr::raw_trusted`](crate::expr::Expr::raw_trusted)）。
//! 这里只负责给它补上单位与说明，让 Prometheus 的 `/metrics` 输出带 `# HELP`。

/// 受信任原始 SQL 的调用计数
pub const RAW_SQL_TOTAL: &str = "alun_db_raw_sql_total";

/// 注册指标元数据（单位 + 说明）。
///
/// 幂等；由 `raw_trusted*` 在首次调用时自动触发。
pub fn describe() {
    metrics::describe_counter!(
        RAW_SQL_TOTAL,
        metrics::Unit::Count,
        "受信任原始 SQL 的调用次数；从 0 变正即意味着有人绕过了列引用与值绑定"
    );
}

/// 首次埋点时注册元数据；之后只剩一次原子读。
#[inline]
pub fn describe_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(describe);
}

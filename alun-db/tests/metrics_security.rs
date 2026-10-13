//! F10 · **安全指标**验证：多租户 fail-closed 与原生 SQL 守卫是否被真实计数。
//!
//! 这些指标的价值不在"能不能加一"，而在**生产上可告警**：
//!
//! - `alun_tenant_scope_missing_total` 持续增长 → 某处漏配 `tenant_scope`；
//! - `alun_raw_sql_tenant_unbound_total` 出现 → 有人在 `Scoped` 上下文里写了
//!   框架无法校验的原生 SQL（应改结构化联查）。
//!
//! 二者在单测里都表现为"正确拒绝了"，只有计数能把"偶发一次"和"每请求都在撞"
//! 区分开，因此这里锁定的是**计数链路本身**（守卫语义由 `multi_tenant.rs` 覆盖）。
//!
//! 仅在 `metrics` feature 下编译：未启用时埋点代码不存在，指标也不该被断言。
#![cfg(feature = "metrics")]

mod common;

use alun_db::prelude::*;
use alun_db::tenant::{TenantContext, set_tenant_enabled, tenant_scope};
use common::*;

/// 参与租户隔离的模型（守卫触发对象）。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("mt_notes")]
pub struct MtNote {
    /// 主键
    #[key]
    pub id: i64,
    /// 标题
    pub title: String,
    /// 租户列
    #[tenant]
    pub tenant_id: i64,
}

/// 安装 Prometheus recorder 并取渲染句柄。
///
/// 全局 recorder 只能装一次，多测试共享同一个句柄（各用例只做**增量**断言，
/// 因此共享安全）。
fn handle() -> metrics_exporter_prometheus::PrometheusHandle {
    static HANDLE: std::sync::OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
        std::sync::OnceLock::new();
    HANDLE
        .get_or_init(|| {
            let h = metrics_exporter_prometheus::PrometheusBuilder::new()
                .install_recorder()
                .expect("install prometheus recorder");
            // recorder 就位后再注册元数据（`describe_*` 无 recorder 时会被丢弃）
            alun_db::metrics::describe();
            h
        })
        .clone()
}

/// 从渲染文本中取出某指标（含给定标签）的当前值。
fn metric_value(rendered: &str, name: &str, label: &str) -> f64 {
    rendered
        .lines()
        .filter(|l| l.starts_with(name) && l.contains(label))
        .filter_map(|l| l.rsplit(' ').next())
        .filter_map(|v| v.parse::<f64>().ok())
        .sum()
}

#[tokio::test]
async fn tenant_fail_closed_and_raw_sql_guard_are_counted() {
    let h = handle();
    let db = setup_db().await;
    db.execute(
        "CREATE TABLE mt_notes (
            id INTEGER PRIMARY KEY,
            title TEXT NOT NULL,
            tenant_id INTEGER NOT NULL
        )",
    )
    .await
    .expect("create mt_notes");

    // 全局开关：仅在 `enabled` 时租户守卫才生效（与生产一致）
    set_tenant_enabled(true);

    // ── 1. 无租户上下文访问隔离模型 → fail-closed + 计数 ──
    let before = metric_value(&h.render(), "alun_tenant_scope_missing_total", "mt_notes");
    let err = MtNote::query().find(&db).await;
    assert!(err.is_err(), "无租户上下文必须 fail-closed");
    let after = metric_value(&h.render(), "alun_tenant_scope_missing_total", "mt_notes");
    assert!(
        after > before,
        "fail-closed 拦截应计入 alun_tenant_scope_missing_total（before={before} after={after}）"
    );

    // ── 2. Scoped 上下文 + 未标注的原生 SQL → 守卫拒绝 + 计数 ──
    let before_raw = metric_value(&h.render(), "alun_raw_sql_tenant_unbound_total", "mt_notes");
    let scoped = tenant_scope(TenantContext::Scoped(Value::I64(1)), async {
        MtNote::sql(sql!("SELECT * FROM mt_notes WHERE 1 = 1"))
            .find(&db)
            .await
    })
    .await;
    assert!(
        scoped.is_err(),
        "Scoped 下未标注的原生 SQL 必须被守卫拒绝（F2）"
    );
    let after_raw = metric_value(&h.render(), "alun_raw_sql_tenant_unbound_total", "mt_notes");
    assert!(
        after_raw > before_raw,
        "原生 SQL 守卫拦截应计入 alun_raw_sql_tenant_unbound_total（before={before_raw} after={after_raw}）"
    );

    // ── 3. DB 通用埋点确实在工作（读路径计数 + 直方图） ──
    let rendered = h.render();
    assert!(
        rendered.contains("alun_db_queries_total"),
        "应有 DB 语句计数（埋点未生效？）：\n{rendered}"
    );
    // 元数据：指标必须自带 `# HELP`，否则运维面对裸序列无从下手
    assert!(
        rendered.contains("# HELP alun_db_queries_total"),
        "导出文本应带 # HELP 说明行：\n{rendered}"
    );
    // ORM 层按**表名**归类：`Scoped` 下再跑一次合法查询即可产出带表名的序列
    let ok = tenant_scope(TenantContext::Cross, async {
        MtNote::query().find(&db).await
    })
    .await;
    assert!(ok.is_ok(), "Cross 上下文应放行：{ok:?}");
    let rendered = h.render();
    assert!(
        rendered.contains(r#"table="mt_notes""#),
        "ORM 读路径应按模型表名归类：\n{rendered}"
    );

    set_tenant_enabled(false);
}

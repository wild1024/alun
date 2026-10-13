//! M3 订阅计费（用例 SUB-001..008）。
//!
//! 目标方言由 `ALUN_LEDGER_DB` 选择（默认 `sqlite-mem`）。需要其它方言的
//! 分支显式门控并打印跳过原因。

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use alun_core::Plugin;
use alun_db::prelude::*;
use alun_db_core::query::SqlQuery;
use alun_db_core::stmt::Value;
use chrono::{DateTime, Datelike, Duration, FixedOffset, TimeZone, Utc};
use rust_decimal::Decimal;

use alun_ledger::models::{Invoice, InvoiceStatus, Subscription, SubscriptionStatus};
use alun_ledger::services::{billing, rowx};

use common::assertions::*;
use common::fixtures::*;
use common::harness::{Dialect, ctx};

// ==================== 辅助 ====================

/// 构造一张订阅计费发票（id 显式指定，便于断言幂等语义）。
fn sub_invoice(id: i64, tenant_id: i64, customer_id: i64, created_at: DateTime<Utc>) -> Invoice {
    Invoice {
        id,
        tenant_id,
        customer_id,
        status: InvoiceStatus::Draft,
        total: money(299, 0),
        metadata: Some(serde_json::json!({"source": "subscription"})),
        issued_at: None,
        paid_at: None,
        created_at,
        note: None,
    }
}

/// 空白归一：被省略的 `sql!` 片段只留下字面量前导空白，比对时忽略多空格。
fn norm(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ==================== SUB-001 ====================

/// 到期扫描边界：`period_end <= now` 为**左闭**；`status = 'active'` 过滤。
#[tokio::test]
async fn sub_001_due_scan_boundary_is_left_closed() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let now = t0();
    // `period_start` 各不相同：uq_subscriptions_period(tenant_id, customer_id, period_start)
    seed_subscription(&c, TENANT_T1, 101, at(-1_000_101), now, "active")
        .await
        .unwrap();
    seed_subscription(
        &c,
        TENANT_T1,
        102,
        at(-1_000_102),
        now + Duration::microseconds(1),
        "active",
    )
    .await
    .unwrap();
    seed_subscription(
        &c,
        TENANT_T1,
        103,
        at(-1_000_103),
        now - Duration::microseconds(1),
        "active",
    )
    .await
    .unwrap();
    seed_subscription(
        &c,
        TENANT_T1,
        104,
        at(-1_000_104),
        now - Duration::microseconds(1),
        "canceled",
    )
    .await
    .unwrap();

    let due = billing::due_subscriptions(c.db(), now)
        .await
        .expect("due scan");
    let ours = due.iter().filter(|s| s.tenant_id == TENANT_T1);
    let ids: BTreeSet<i64> = ours.clone().map(|s| s.id).collect();
    assert_eq!(
        ids,
        [101_i64, 103].into_iter().collect::<BTreeSet<_>>(),
        "period_end == now 命中；now+1µs 不命中；now-1µs 命中；canceled 被 status 过滤"
    );
    assert!(
        ours.clone().all(billing::is_active),
        "返回行必须全部为 active"
    );
    assert!(!ids.contains(&102), "晚于 now 1µs 的订阅不得命中");
    assert!(!ids.contains(&104), "canceled 订阅不得命中");
}

// ==================== SUB-002 ====================

/// 跨时区归月：UTC 存储 + UTC 归月；与 Asia/Shanghai 本地归月不同。
#[tokio::test]
async fn sub_002_utc_storage_versus_shanghai_month_attribution() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    // 2026-01-31T23:00:00Z == 2026-02-01 07:00 +08:00（Asia/Shanghai 全年 UTC+8）
    let jan_utc = Utc.with_ymd_and_hms(2026, 1, 31, 23, 0, 0).unwrap();
    let cst = FixedOffset::east_opt(8 * 3600).unwrap();
    let local = jan_utc.with_timezone(&cst);
    assert_eq!(
        (local.year(), local.month(), local.day()),
        (2026, 2, 1),
        "同一瞬间在 Asia/Shanghai 已是次日/次月"
    );

    // 存储与读回都是 UTC 同一瞬间
    seed_subscription(
        &c,
        TENANT_T1,
        201,
        jan_utc - Duration::days(30),
        jan_utc,
        "active",
    )
    .await
    .unwrap();
    let stored = billing::due_subscriptions(c.db(), jan_utc)
        .await
        .unwrap()
        .into_iter()
        .find(|s| s.id == 201)
        .expect("seeded subscription");
    assert_eq!(stored.period_end, jan_utc, "时间戳按 UTC 往返");

    // period_key / month_bounds 以 UTC 为准
    assert_eq!(billing::period_key(jan_utc), "2026-01");
    let (ms, me) = billing::month_bounds(jan_utc);
    assert_eq!(
        (ms, me),
        (
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap(),
        )
    );
    // 上海本地月初 2026-02-01T00:00+08:00 == 2026-01-31T16:00Z，UTC 归月仍为 2026-01
    let local_month_start_utc = Utc.with_ymd_and_hms(2026, 1, 31, 16, 0, 0).unwrap();
    assert_eq!(billing::period_key(local_month_start_utc), "2026-01");

    // 用量归组：相隔 1 小时、同一上海日期，却落在两个不同的 UTC 月份组
    seed_usage(&c, 201, 2010, jan_utc, 5).await.unwrap();
    seed_usage(&c, 201, 2011, jan_utc + Duration::hours(1), 7)
        .await
        .unwrap();
    let rows = billing::usage_rows(c.db(), 201).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].usage_date, jan_utc);
    assert_eq!(rows[1].usage_date, jan_utc + Duration::hours(1));

    let utc_keys: BTreeSet<String> = rows
        .iter()
        .map(|r| billing::period_key(r.usage_date))
        .collect();
    assert_eq!(
        utc_keys,
        ["2026-01".to_string(), "2026-02".to_string()]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        "UTC 归月把同一上海日期的两条记录拆到 1 月 / 2 月"
    );
    let local_keys: BTreeSet<String> = rows
        .iter()
        .map(|r| {
            let l = r.usage_date.with_timezone(&cst);
            format!("{:04}-{:02}", l.year(), l.month())
        })
        .collect();
    assert_eq!(local_keys.len(), 1, "按 Asia/Shanghai 归月则同属 2026-02");
}

// ==================== SUB-003 ====================

/// 用量汇总：`GROUP BY + HAVING + COUNT(DISTINCT)`（raw SQL 路径）。
#[tokio::test]
async fn sub_003_usage_aggregate_group_having_count_distinct() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let d0 = at(0);
    seed_subscription(&c, TENANT_T1, 301, at(-30), at(30), "active")
        .await
        .unwrap();
    // 4 条记录 / 3 个不同 usage_date / Σ = 18
    seed_usage(&c, 301, 3010, d0, 3).await.unwrap();
    seed_usage(&c, 301, 3011, d0, 4).await.unwrap();
    seed_usage(&c, 301, 3012, d0 + Duration::days(1), 5)
        .await
        .unwrap();
    seed_usage(&c, 301, 3013, d0 + Duration::days(2), 6)
        .await
        .unwrap();

    let agg = billing::aggregate_usage(c.db(), 301).await.unwrap();
    assert_eq!(
        agg,
        billing::UsageAgg {
            days: 3,
            total: Decimal::from(18)
        },
        "COUNT(DISTINCT usage_date) 与 SUM(quantity)"
    );

    // HAVING SUM(quantity) > 0：和为负 → 整组被过滤（未过滤时 days 会是 2）
    seed_subscription(&c, TENANT_T1, 302, at(-31), at(31), "active")
        .await
        .unwrap();
    seed_usage(&c, 302, 3020, d0, 5).await.unwrap();
    seed_usage(&c, 302, 3021, d0 + Duration::days(1), -10)
        .await
        .unwrap();
    assert_eq!(
        billing::aggregate_usage(c.db(), 302).await.unwrap(),
        billing::UsageAgg {
            days: 0,
            total: Decimal::ZERO
        },
        "HAVING 过滤负和组"
    );

    // 和恰为 0 也被过滤
    seed_subscription(&c, TENANT_T1, 303, at(-32), at(32), "active")
        .await
        .unwrap();
    seed_usage(&c, 303, 3030, d0, 5).await.unwrap();
    seed_usage(&c, 303, 3031, d0 + Duration::days(1), -5)
        .await
        .unwrap();
    assert_eq!(
        billing::aggregate_usage(c.db(), 303).await.unwrap(),
        billing::UsageAgg {
            days: 0,
            total: Decimal::ZERO
        },
        "HAVING 过滤零和组"
    );

    // 无记录
    assert_eq!(
        billing::aggregate_usage(c.db(), 999_999).await.unwrap(),
        billing::UsageAgg {
            days: 0,
            total: Decimal::ZERO
        }
    );
    // 说明：`sql!` 指令表只有 #where/#and/#or/#orderBy/#groupBy/#limit，
    // **没有 #having**（`alun-db-macro/src/sql_macro.rs::expand_directive`），
    // 故本用例只能走 raw SQL；AST 层 `SelectStmt.having` 存在但无面向用户的入口。
}

// ==================== SUB-004 ====================

/// `sql!` 动态条件片段：`#where/#and` 在 `None` 时整段省略（含参数）。
///
/// `#and` 指令要求参数变量为 `Option<T: TypeMapper>`（`&str` 未实现
/// `TypeMapper`，仅实现了 `IntoValue`），故这里用 `String`。
fn dynamic_subscriptions(tenant_filter: Option<i64>, status_filter: Option<String>) -> SqlQuery {
    sql!(
        "SELECT id FROM subscriptions WHERE 1=1",
        " #and(tenant_id, '=', tenant_filter)",
        " #and(status, '=', status_filter)",
        " #orderBy(id asc)",
    )
}

/// `#where` 变体：`None` 时连 `WHERE` 关键字一并省略。
fn dynamic_by_tenant_only(tenant_filter: Option<i64>) -> SqlQuery {
    sql!(
        "SELECT id FROM subscriptions",
        " #where(tenant_id, '=', tenant_filter)"
    )
}

#[tokio::test]
async fn sub_004_sql_macro_dynamic_conditions_drop_none() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_subscription(&c, TENANT_T1, 401, at(-1_000_401), at(10), "active")
        .await
        .unwrap();
    seed_tenant(&c, TENANT_T2, "T2").await.unwrap();
    seed_subscription(&c, TENANT_T2, 402, at(-1_000_402), at(10), "active")
        .await
        .unwrap();
    seed_subscription(&c, TENANT_T2, 403, at(-1_000_403), at(10), "canceled")
        .await
        .unwrap();

    // 文本 + 参数个数：None 的 #and 整段省略
    let q = dynamic_subscriptions(None, Some("active".to_string()));
    assert!(
        !q.sql().contains("tenant_id"),
        "None 条件必须整段省略：{}",
        q.sql()
    );
    assert!(q.sql().contains("AND status = ?"), "{}", q.sql());
    assert!(q.sql().ends_with("ORDER BY id ASC"), "{}", q.sql());
    assert_eq!(q.params().len(), 1, "仅 status 一个绑定参数");

    let ids = |rows: Vec<Row>| -> BTreeSet<i64> {
        rows.iter()
            .map(|r| rowx::i64_of(r, "id").unwrap())
            .collect()
    };
    let all_active = ids(dynamic_subscriptions(None, Some("active".to_string()))
        .find(c.db())
        .await
        .unwrap());
    assert!(all_active.contains(&401) && all_active.contains(&402));
    assert!(!all_active.contains(&403), "status 过滤生效");

    let t1_only = ids(dynamic_subscriptions(Some(TENANT_T1), None)
        .find(c.db())
        .await
        .unwrap());
    assert!(t1_only.contains(&401));
    assert!(!t1_only.contains(&402), "tenant 过滤生效");

    let q = dynamic_subscriptions(Some(TENANT_T2), Some("canceled".to_string()));
    assert_eq!(q.params().len(), 2);
    assert!(q.sql().contains("AND tenant_id = ?"));
    assert!(q.sql().contains("AND status = ?"));
    let canceled = ids(
        dynamic_subscriptions(Some(TENANT_T2), Some("canceled".to_string()))
            .find(c.db())
            .await
            .unwrap(),
    );
    assert_eq!(canceled, [403_i64].into_iter().collect::<BTreeSet<_>>());

    // 全 None：无条件片段，仍为合法 SQL
    // （被省略的 `#and` 只留下字面量里的前导空白，不产生条件与参数）
    let q = dynamic_subscriptions(None, None);
    assert_eq!(q.params().len(), 0);
    assert_eq!(
        norm(q.sql()),
        "SELECT id FROM subscriptions WHERE 1=1 ORDER BY id ASC"
    );

    // #where 的整段省略
    let q = dynamic_by_tenant_only(None);
    assert_eq!(norm(q.sql()), "SELECT id FROM subscriptions");
    assert_eq!(q.params().len(), 0);
    let q = dynamic_by_tenant_only(Some(TENANT_T2));
    assert_eq!(
        norm(q.sql()),
        "SELECT id FROM subscriptions WHERE tenant_id = ?"
    );
    assert_eq!(q.params().len(), 1);
    assert_eq!(
        ids(dynamic_by_tenant_only(Some(TENANT_T2))
            .find(c.db())
            .await
            .unwrap()),
        [402_i64, 403].into_iter().collect::<BTreeSet<_>>()
    );
}

// ==================== SUB-005 ====================

/// `insert_many` 的 Hook 探针（进程内计数）。
static SCRATCH_HOOK_CALLS: AtomicUsize = AtomicUsize::new(0);

struct BatchProbeHook;

impl Hook for BatchProbeHook {
    fn before_insert(&self, _row: &mut Row) -> alun_db_core::error::Result<()> {
        SCRATCH_HOOK_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Model, Debug, Clone)]
#[table("sub005_scratch")]
#[hook(BatchProbeHook)]
struct ScratchRow {
    #[key]
    id: i64,
    label: String,
}

/// 自增主键探针（仅 sqlite 侧建表，见测试内门控）。
#[derive(Model, Debug, Clone)]
#[table("sub005_auto")]
struct AutoRow {
    #[key]
    #[auto]
    id: i64,
    label: String,
}

/// 1000 张发票单批写入：`insert_many` 不触发 Hook、不回填自增主键（G16）。
#[tokio::test]
async fn sub_005_batch_invoice_skips_hooks_and_auto_ids() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let batch: Vec<Invoice> = (0..1000)
        .map(|i| sub_invoice(5_000 + i, TENANT_T1, TENANT_T1 * 100 + 7, at(0)))
        .collect();
    billing::issue_invoices_batch(c.db(), &batch)
        .await
        .expect("batch issue");
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE tenant_id = ?",
            vec![Value::I64(TENANT_T1)]
        )
        .await,
        1000,
        "1000 张发票全部落库"
    );
    assert_eq!(batch[0].id, 5_000, "非自增主键原样写入");

    // Hook 语义：insert_many 不触发；save 触发（对照）
    c.db()
        .execute("CREATE TABLE IF NOT EXISTS sub005_scratch (id BIGINT NOT NULL PRIMARY KEY, label VARCHAR(64) NOT NULL)")
        .await
        .unwrap();
    c.db().execute("DELETE FROM sub005_scratch").await.unwrap();

    let rows = vec![
        ScratchRow {
            id: 1,
            label: "a".into(),
        },
        ScratchRow {
            id: 2,
            label: "b".into(),
        },
    ];
    ScratchRow::insert_many(&rows, c.db())
        .await
        .expect("scratch batch");
    assert_eq!(
        SCRATCH_HOOK_CALLS.load(Ordering::SeqCst),
        0,
        "insert_many 不触发 before_insert（G16）"
    );
    assert_eq!(
        count_by(c.db(), "SELECT COUNT(*) FROM sub005_scratch", vec![]).await,
        2
    );

    ScratchRow {
        id: 3,
        label: "c".into(),
    }
    .insert_or_update(c.db())
    .await
    .expect("save");
    assert_eq!(
        SCRATCH_HOOK_CALLS.load(Ordering::SeqCst),
        1,
        "对照：save() 触发 before_insert"
    );

    // 自增主键：insert_many 不回填、save 回填
    if c.dialect.is_sqlite() {
        c.db()
            .execute(
                "CREATE TABLE IF NOT EXISTS sub005_auto (id INTEGER PRIMARY KEY AUTOINCREMENT, label VARCHAR(64) NOT NULL)",
            )
            .await
            .unwrap();
        c.db().execute("DELETE FROM sub005_auto").await.unwrap();

        let rows = vec![
            AutoRow {
                id: 0,
                label: "x".into(),
            },
            AutoRow {
                id: 0,
                label: "y".into(),
            },
        ];
        AutoRow::insert_many(&rows, c.db())
            .await
            .expect("auto batch");
        let db_ids: Vec<i64> = c
            .db()
            .find_with_params("SELECT id FROM sub005_auto ORDER BY id", vec![])
            .await
            .unwrap()
            .iter()
            .map(|r| rowx::i64_of(r, "id").unwrap())
            .collect();
        assert_eq!(db_ids.len(), 2, "数据库生成 2 个自增主键");
        assert!(db_ids[0] < db_ids[1], "自增主键递增：{db_ids:?}");
        assert_eq!(
            rows[0].id, 0,
            "insert_many 不回填自增主键到调用方结构体（G16）"
        );

        let mut one = AutoRow {
            id: 0,
            label: "z".into(),
        };
        one.insert_or_update(c.db()).await.expect("save auto");
        assert!(one.id > 0, "对照：save() 回填自增主键，实际 id={}", one.id);
    } else {
        eprintln!(
            "[skip] 自增主键语义仅在 sqlite 验证：{} 需要方言化 DDL（AUTOINCREMENT/SERIAL）",
            c.dialect.name()
        );
    }
}

// ==================== SUB-006 ====================

/// 幂等开票 + 订阅 upsert（MySQL 无 `on_conflict` → `UnsupportedFeature`）。
#[tokio::test]
async fn sub_006_invoice_once_and_subscription_upsert() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    let customer = TENANT_T1 * 100 + 7;

    // 同一自然月，第二次返回既有发票
    let (first, inserted) =
        billing::create_invoice_once(c.db(), sub_invoice(6_001, TENANT_T1, customer, at(60)))
            .await
            .expect("first");
    assert!(inserted, "首次应插入");
    assert_eq!(first.id, 6_001);

    let (second, inserted) =
        billing::create_invoice_once(c.db(), sub_invoice(6_002, TENANT_T1, customer, at(120)))
            .await
            .expect("second");
    assert!(!inserted, "同月第二次返回既有发票");
    assert_eq!(second.id, 6_001, "返回既有发票 id");
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE tenant_id = ? AND customer_id = ?",
            vec![Value::I64(TENANT_T1), Value::I64(customer)]
        )
        .await,
        1,
        "同（租户, 客户, 自然月）仅一张"
    );

    // 跨月 → 新发票
    let (third, inserted) = billing::create_invoice_once(
        c.db(),
        sub_invoice(6_003, TENANT_T1, customer, at(31 * 86_400)),
    )
    .await
    .expect("third");
    assert!(inserted, "次月应新增");
    assert_eq!(third.id, 6_003);
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE tenant_id = ? AND customer_id = ?",
            vec![Value::I64(TENANT_T1), Value::I64(customer)]
        )
        .await,
        2
    );

    // upsert_subscription
    let mut sub = Subscription {
        id: 6_010,
        tenant_id: TENANT_T1,
        customer_id: customer,
        plan: "pro".into(),
        period_start: at(0),
        period_end: at(86_400),
        status: SubscriptionStatus::Active,
    };
    if c.dialect == Dialect::Mysql {
        let err = billing::upsert_subscription(c.db(), &mut sub)
            .await
            .expect_err("MySQL 无 on_conflict 能力，应报 UnsupportedFeature");
        assert!(
            matches!(err, DbError::UnsupportedFeature { .. }),
            "期望 UnsupportedFeature，实际 {err:?}"
        );
    } else {
        billing::upsert_subscription(c.db(), &mut sub)
            .await
            .expect("upsert insert");
        assert_eq!(
            count_by(
                c.db(),
                "SELECT COUNT(*) FROM subscriptions WHERE tenant_id = ?",
                vec![Value::I64(TENANT_T1)]
            )
            .await,
            1
        );

        sub.plan = "enterprise".into();
        billing::upsert_subscription(c.db(), &mut sub)
            .await
            .expect("upsert update");
        assert_eq!(
            count_by(
                c.db(),
                "SELECT COUNT(*) FROM subscriptions WHERE tenant_id = ?",
                vec![Value::I64(TENANT_T1)]
            )
            .await,
            1,
            "冲突时更新，不新增行"
        );
        let row = c
            .db()
            .find_with_params(
                "SELECT plan FROM subscriptions WHERE id = ?",
                vec![Value::I64(6_010)],
            )
            .await
            .unwrap();
        assert_eq!(
            rowx::text_of(&row[0], "plan").unwrap(),
            "enterprise",
            "upsert 更新了冲突列"
        );
    }
}

// ==================== SUB-007 ====================

/// 批量进度：框架无进度 API，项目侧自行按批回调。
#[tokio::test]
async fn sub_007_batch_progress_is_project_side() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();

    let batch: Vec<Invoice> = (0..1000)
        .map(|i| sub_invoice(7_000 + i, TENANT_T1, TENANT_T1 * 100 + 7, at(0)))
        .collect();

    // 框架层面 `issue_invoices_batch` 无回调参数：项目侧按 100 条切片推进
    let mut progress: Vec<usize> = Vec::new();
    let mut done = 0usize;
    for part in batch.chunks(100) {
        billing::issue_invoices_batch(c.db(), part)
            .await
            .expect("chunk");
        done += part.len();
        progress.push(done);
    }
    assert_eq!(
        progress,
        (1..=10).map(|k| k * 100).collect::<Vec<_>>(),
        "每批（100 条）恰好一次进度事件"
    );
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE tenant_id = ?",
            vec![Value::I64(TENANT_T1)]
        )
        .await,
        1000
    );
}

// ==================== SUB-008 ====================

/// SUB-008：调度插件**真正执行**注册的定时任务（D027 已修复；旧实现只登记不执行）。
#[tokio::test]
async fn sub_008_scheduler_executes_registered_jobs() {
    use alun_plugin::scheduler::{CronSchedule, SchedulerPlugin};

    // ① cron 解析：5 段 / 6 段均可，非法表达式必须报错（不 panic）
    assert!(CronSchedule::parse("0 3 * * *").is_ok(), "5 段标准表达式");
    assert!(CronSchedule::parse("* * * * * *").is_ok(), "6 段秒级表达式");
    assert!(
        CronSchedule::parse("*/5 1-5 * * 1,3").is_ok(),
        "步长/区间/枚举"
    );
    for bad in ["61 * * * *", "0 3 * *", "not a cron", "0 0 * * 8"] {
        assert!(
            CronSchedule::parse(bad).is_err(),
            "非法表达式 `{bad}` 必须报错"
        );
    }

    // ② 到点执行：秒级表达式在 ~2.5s 内至少触发一次
    let hits = Arc::new(AtomicUsize::new(0));
    let plugin = SchedulerPlugin::new();
    let counter = hits.clone();
    plugin.register("sub-008-tick", "* * * * * *", "每秒探针", move || {
        let counter = counter.clone();
        tokio::spawn(async move {
            counter.fetch_add(1, Ordering::SeqCst);
        })
    });
    let listed: Vec<String> = plugin.list().into_iter().map(|(n, _, _)| n).collect();
    assert!(
        listed.iter().any(|n| n == "sub-008-tick"),
        "注册后应可在 list() 中查到：{listed:?}"
    );
    assert!(!plugin.is_running(), "未 start 前不应在运行");
    assert_eq!(hits.load(Ordering::SeqCst), 0, "注册本身不执行");

    Plugin::start(&plugin).await.expect("start");
    assert!(plugin.is_running(), "start 后应处于运行态");
    tokio::time::sleep(std::time::Duration::from_millis(2_500)).await;
    let after_start = hits.load(Ordering::SeqCst);
    assert!(
        after_start > 0,
        "秒级 cron 应在 ~2.5s 内至少触发一次，实际 {after_start}"
    );

    // ③ 停止后不再触发（stop 幂等）
    Plugin::stop(&plugin).await.expect("stop");
    assert!(!plugin.is_running(), "stop 后不应仍在运行");
    Plugin::stop(&plugin).await.expect("stop 幂等");
    tokio::time::sleep(std::time::Duration::from_millis(2_500)).await;
    assert_eq!(
        hits.load(Ordering::SeqCst),
        after_start,
        "stop 之后不得再触发"
    );

    // ④ 未注册项 trigger 不得 panic
    assert!(
        plugin.trigger("nope").is_none(),
        "未注册项 trigger 返回 None"
    );

    eprintln!(
        "[SUB-008] SchedulerPlugin 真实执行：cron 解析（5/6 段）通过，秒级任务触发 {after_start} 次，stop 后不再触发（D027 已修复）"
    );
}

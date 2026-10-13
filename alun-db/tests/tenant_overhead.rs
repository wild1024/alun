//! 租户隔离**开销回归测试**：宽松的计时护栏，只拦「明显退化」，不追求精确比值。
//!
//! ## 为什么是宽松阈值
//!
//! 基准（`benches/tenant_isolation.rs`）实测：SQLite 下开启行级租户隔离的**绝对**
//! 开销约 0.5 µs/次查询，来源是框架 scope 注入机制（非租户谓词本身，谓词仅 +2.6%）。
//! 对极短的 pk 点查（约 1.2 µs）相对占比可达 ~42%，对 keyset 分页约 +13%，
//! 对批量插入约 +2%。因此**不存在**一个统一的「<5%」相对阈值：
//!
//! - 用相对阈值会因操作时长差异巨大而失真；
//! - 计时本身在 CI 上有噪声。
//!
//! 故本测试按操作分别设定**带余量**的宽松上限（见各 `*_MAX_RATIO`），并在两次
//! 取最小值（min-of-N）以压低噪声。若某操作比值突破上限，说明隔离路径出现了
//! 数量级级别的退化，需要人工排查。
//!
//! ## 口径
//!
//! - 仅 SQLite（默认特性）；PG 的 `schema-per-tenant` 是连接级开销，不在此口径。
//! - 开关是**进程级**全局状态，故本文件只放一个测试函数，避免并行测例互相干扰。
//! - 使用 `#[test]`（非 `#[tokio::test]`）自建运行时，便于在同一进程内串行翻转开关。

use std::time::{Duration, Instant};

use alun_db::prelude::*;

/// 基准表行数（较 criterion 的 10 万行缩小，保证测试时长可控）。
const ROWS: usize = 50_000;

/// 点查次数。
const LOOKUPS: usize = 300;

/// 分页页大小。
const PAGE_SIZE: u64 = 1_000;

/// 单次批量插入行数。
const INSERT_ROWS: usize = 1_000;

/// 参与隔离的租户 ID。
const TENANT_ID: i64 = 1;

/// 每侧的重复次数（取最小值，压低噪声）。
const REPS: usize = 5;

/// 点查比值上限：实测约 1.42×，留足余量以容忍 debug 构建与 CI 抖动。
const LOOKUP_MAX_RATIO: f64 = 4.0;

/// 分页比值上限：实测约 1.13×。
const PAGINATION_MAX_RATIO: f64 = 2.0;

/// 批量插入比值上限：实测约 1.02×。
const INSERT_MAX_RATIO: f64 = 1.5;

/// 基准用租户模型。
#[derive(Model, Debug, Clone)]
#[table("bench_tenant_overhead_users")]
struct TenantUser {
    /// 自增主键。
    #[key]
    #[auto]
    id: i64,
    /// 名称。
    name: String,
    /// 租户列（行级隔离依据）。
    #[tenant]
    tenant_id: i64,
}

/// 建表 SQL。
const CREATE_TABLE: &str = "\
CREATE TABLE bench_tenant_overhead_users (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    name      TEXT    NOT NULL,
    tenant_id INTEGER NOT NULL
)";

/// 建库：建表、建租户列索引，并灌入 [`ROWS`] 行（全部属于 [`TENANT_ID`]）。
async fn new_read_db() -> Db {
    let db = Db::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    db.execute(CREATE_TABLE).await.expect("create table");
    db.execute(
        "CREATE INDEX idx_bench_overhead_users_tenant ON bench_tenant_overhead_users(tenant_id)",
    )
    .await
    .expect("create index");
    db.execute(&format!(
        "INSERT INTO bench_tenant_overhead_users (name, tenant_id)
         WITH RECURSIVE seq(n) AS (
             SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < {ROWS}
         )
         SELECT 'user' || n, {TENANT_ID} FROM seq"
    ))
    .await
    .expect("seed rows");
    db
}

/// 建空写库。
async fn new_write_db() -> Db {
    let db = Db::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    db.execute(CREATE_TABLE).await.expect("create table");
    db
}

/// 确定性点查主键序列。
fn lookup_ids() -> Vec<i64> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    (0..LOOKUPS)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            1 + (state % ROWS as u64) as i64
        })
        .collect()
}

/// 构造待插入模型。
fn make_insert_rows() -> Vec<TenantUser> {
    (0..INSERT_ROWS)
        .map(|i| TenantUser {
            id: 0,
            name: format!("new{i}"),
            tenant_id: TENANT_ID,
        })
        .collect()
}

/// 重复 [`REPS`] 次并取最短耗时。
///
/// 取最小值而非均值：计时分布右偏（偶发的调度/分配抖动只增不减），最小值
/// 最接近「无干扰」的真实成本，因而在 CI 上更稳定。
fn timed_min<F, Fut>(rt: &tokio::runtime::Runtime, mut op: F) -> Duration
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut best = Duration::MAX;
    for _ in 0..REPS {
        let started = Instant::now();
        rt.block_on(op());
        best = best.min(started.elapsed());
    }
    best
}

/// 比值（`on / off`），`off` 为零时返回正无穷。
fn ratio(on: Duration, off: Duration) -> f64 {
    let off = off.as_secs_f64();
    if off <= 0.0 {
        return f64::INFINITY;
    }
    on.as_secs_f64() / off
}

/// 断言比值不超过上限，超出时给出可读的失败信息。
fn assert_ratio(op: &str, on: Duration, off: Duration, max: f64) {
    let actual = ratio(on, off);
    println!(
        "{op}: off={:.2} ms, on={:.2} ms, ratio={actual:.2}× (上限 {max:.2}×)",
        off.as_secs_f64() * 1e3,
        on.as_secs_f64() * 1e3,
    );
    assert!(
        actual <= max,
        "租户隔离使 `{op}` 耗时增至 {actual:.2}×（上限 {max:.2}×），疑似隔离路径退化"
    );
}

/// 租户隔离开销回归护栏：点查 / 分页 / 批量插入的开启态不应显著慢于关闭态。
#[test]
fn tenant_isolation_overhead_stays_bounded() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build tokio runtime");

    let read_db = rt.block_on(new_read_db());
    let write_db = rt.block_on(new_write_db());
    let ids = lookup_ids();
    let insert_rows = make_insert_rows();

    // 预热：触发 SQLite 页缓存与首轮分配，避免把冷启动算进比值
    set_tenant_enabled(false);
    rt.block_on(async {
        let _ = TenantUser::find_or_none(&read_db, ids[0]).await;
        let _ = TenantUser::insert_many(&insert_rows, &write_db).await;
    });

    // ---------- 关闭隔离（存量单租户行为） ----------
    set_tenant_enabled(false);
    let lookup_off = timed_min(&rt, || async {
        for id in &ids {
            TenantUser::find_or_none(&read_db, *id)
                .await
                .expect("lookup off")
                .expect("row exists");
        }
    });
    let paginate_off = timed_min(&rt, || run_pagination(&read_db));
    let insert_off = timed_min(&rt, || async {
        TenantUser::insert_many(&insert_rows, &write_db)
            .await
            .expect("insert off");
    });

    // ---------- 开启隔离（自动追加租户条件） ----------
    set_tenant_enabled(true);
    let lookup_on = timed_min(&rt, || {
        tenant_scope(TenantContext::Scoped(Value::I64(TENANT_ID)), async {
            for id in &ids {
                TenantUser::find_or_none(&read_db, *id)
                    .await
                    .expect("lookup on")
                    .expect("row exists");
            }
        })
    });
    let paginate_on = timed_min(&rt, || {
        tenant_scope(TenantContext::Scoped(Value::I64(TENANT_ID)), async {
            run_pagination(&read_db).await;
        })
    });
    let insert_on = timed_min(&rt, || {
        tenant_scope(TenantContext::Scoped(Value::I64(TENANT_ID)), async {
            TenantUser::insert_many(&insert_rows, &write_db)
                .await
                .expect("insert on");
        })
    });

    // 恢复默认开关，避免影响同进程后续逻辑
    set_tenant_enabled(true);

    assert_ratio("点查", lookup_on, lookup_off, LOOKUP_MAX_RATIO);
    assert_ratio("分页", paginate_on, paginate_off, PAGINATION_MAX_RATIO);
    assert_ratio("批量插入", insert_on, insert_off, INSERT_MAX_RATIO);
}

/// keyset 游标分页：逐页向后遍历全表直到取尽。
async fn run_pagination(db: &Db) {
    let mut cursor: Option<Cursor> = None;
    loop {
        let rows = TenantUser::query()
            .order_by(OrderBy {
                expr: ColumnRef::Column("id"),
                direction: Order::Asc,
                nulls: None,
            })
            .page_after("id", "id", Order::Asc, cursor.clone())
            .limit(PAGE_SIZE)
            .find(db)
            .await
            .expect("keyset page");
        if rows.len() < PAGE_SIZE as usize {
            break;
        }
        let last = rows.last().expect("page is non-empty when full");
        cursor = Some(Cursor {
            key: Value::I64(last.id),
            pk: Value::I64(last.id),
        });
    }
}

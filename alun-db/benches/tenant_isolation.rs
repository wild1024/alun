//! 租户隔离**开销基准**：同一张 10 万行表，开 / 关行级租户隔离各跑
//! 点查、分页、批量插入，用 criterion 统计框架给出两侧耗时分布。
//!
//! ## 口径说明（读结果前必看）
//!
//! - 开关 `set_tenant_enabled` 是**进程级**全局状态，无法在一次基准中并行对比，
//!   因此本基准先注册「关闭隔离」的三项，再翻转开关注册「开启隔离」的三项。
//!   criterion 在 `bench_function` 注册时即执行测量，故两组不会交错。
//! - 「开启隔离」时点查 / 分页在 `tenant_scope(Scoped(1))` 内执行，模型查询
//!   自动追加 `WHERE tenant_id = ?`；「关闭隔离」时不追加。数据全部属于租户 1，
//!   两次扫描的行数相同，差额即隔离注入（谓词 + 绑参 + 上下文字段读取）的净开销。
//! - 分页采用 **keyset 游标**（`page_after`）而非 `LIMIT/OFFSET`：`OFFSET` 分页一旦
//!   带上任何 `WHERE`（含租户条件）就会失去 SQLite 的 rowid 跳页优化，退化为
//!   `O(n²/页大小)`，实测比无 `WHERE` 慢约 5 倍——这是 OFFSET 分页自身的病理，
//!   与租户隔离无关。keyset 两侧都走索引 seek，才能公平地量出隔离的净开销。
//! - 写入基准使用独立空库，避免与读基准的 10 万行表相互干扰。
//! - 仅覆盖 SQLite（默认特性）。PostgreSQL 的 schema-per-tenant 走 `SET search_path`，
//!   其开销是连接级而非语句级，不在此基准口径内。
//!
//! ## 实测结论（10 万行表，供回归对照）
//!
//! | 操作 | 关闭隔离 | 开启隔离 | 相对增幅 |
//! |---|---|---|---|
//! | 点查（1000 次 `find_or_none`） | 1.21 ms | 1.72 ms | +42% |
//! | keyset 分页（100 页 × 1000 行） | 21.3 ms | 24.0 ms | +13% |
//! | 批量插入（2000 行） | 488 µs | 497 µs | +1.7% |
//!
//! 诊断项显示：把同样的 SQL 手工多带一个 `tenant_id = ?` 谓词，仅 +2.6%
//! （790 µs → 813 µs）。因此点查的 +42% 几乎全部来自框架的 scope 注入机制
//! （`scope_for` + task_local 读取 + 渲染层多次 `format!`），与租户谓词无关。
//! 绝对开销约 0.5 µs/次，对极短的 pk 点查（1.2 µs）相对占比大，对批量插入
//! 这类长操作可忽略。回归阈值见 `tests/tenant_overhead.rs`。
//!
//! 运行：`cargo bench -p alun-db --bench tenant_isolation`

use std::time::Duration;

use alun_db::prelude::*;
use criterion::{Criterion, criterion_group, criterion_main};

/// 基准表行数。
const ROWS: usize = 100_000;

/// 点查次数（每次迭代内循环）。
const LOOKUPS: usize = 1_000;

/// 分页页大小。
const PAGE_SIZE: u64 = 1_000;

/// 单次批量插入的行数。
const INSERT_ROWS: usize = 2_000;

/// 参与隔离的租户 ID（数据全部属于该租户，保证两侧扫描行数一致）。
const TENANT_ID: i64 = 1;

/// 基准用租户模型：`tenant_id` 为隔离列。
#[derive(Model, Debug, Clone)]
#[table("bench_tenant_users")]
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

/// 建表语句。
const CREATE_TABLE: &str = "\
CREATE TABLE bench_tenant_users (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    name      TEXT    NOT NULL,
    tenant_id INTEGER NOT NULL
)";

/// 租户列索引：文档明确要求为租户列建索引，基准须反映这一最佳实践。
const CREATE_TENANT_INDEX: &str =
    "CREATE INDEX idx_bench_tenant_users_tenant ON bench_tenant_users(tenant_id)";

/// 建立只读基准库：建表、建租户列索引，并一次性灌入 [`ROWS`] 行（全部属于 [`TENANT_ID`]）。
async fn new_read_db() -> Db {
    let db = Db::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    db.execute(CREATE_TABLE).await.expect("create table");
    db.execute(CREATE_TENANT_INDEX).await.expect("create index");
    // 递归 CTE 一次性生成十万行：比逐行插入快，且不经模型写入路径
    db.execute(&format!(
        "INSERT INTO bench_tenant_users (name, tenant_id)
         WITH RECURSIVE seq(n) AS (
             SELECT 1 UNION ALL SELECT n + 1 FROM seq WHERE n < {ROWS}
         )
         SELECT 'user' || n, {TENANT_ID} FROM seq"
    ))
    .await
    .expect("seed rows");
    db
}

/// 建立写入基准库：仅建表。
async fn new_write_db() -> Db {
    let db = Db::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    db.execute(CREATE_TABLE).await.expect("create table");
    db
}

/// 构造 [`INSERT_ROWS`] 行待插入模型（在计时区之外预构造）。
fn make_insert_rows() -> Vec<TenantUser> {
    (0..INSERT_ROWS)
        .map(|i| TenantUser {
            id: 0,
            name: format!("new{i}"),
            tenant_id: TENANT_ID,
        })
        .collect()
}

/// 确定性的点查主键序列（线性同余，避免引入 rand 依赖）。
fn lookup_ids() -> Vec<i64> {
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    (0..LOOKUPS)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            1 + (state % ROWS as u64) as i64
        })
        .collect()
}

/// 点查：按主键逐个读取，断言命中。
async fn run_lookup(db: &Db, ids: &[i64]) {
    for id in ids {
        TenantUser::find_or_none(db, *id)
            .await
            .expect("lookup")
            .expect("row exists");
    }
}

/// keyset 游标分页：按主键逐页向后遍历全表直到取尽。
///
/// 两侧（开 / 关隔离）都使用 `page_after` 游标，SQL 形态仅差一个
/// `tenant_id = ?` 谓词，故耗时差额即隔离注入的净开销。
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

/// 批量插入：整批一次多行 `INSERT`（不额外开事务）。
async fn run_bulk_insert(db: &Db, rows: &[TenantUser]) {
    TenantUser::insert_many(rows, db)
        .await
        .expect("bulk insert");
}

/// 注册某一隔离模式下的三项基准。
///
/// `with_diagnostics = true` 时额外注册两个「裸 SQL 对照」基准，用于把
/// 「租户谓词本身」的开销从「框架 scope 注入机制」的开销中分离出来；
/// 只在其中一组注册即可（裸 SQL 不经 scope 注入，两组数值仅差运行时噪声）。
fn bench_mode(c: &mut Criterion, name: &str, rt: &tokio::runtime::Runtime, with_diagnostics: bool) {
    let read_db = rt.block_on(new_read_db());
    let write_db = rt.block_on(new_write_db());
    let ids = lookup_ids();
    let insert_rows = make_insert_rows();
    // 隔离开启时在租户上下文中执行，否则在默认（跨租户）上下文执行
    let is_scoped = name == "isolation_on";

    let mut group = c.benchmark_group(name);
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("point_lookup", |b| {
        let db = read_db.clone();
        b.to_async(rt).iter(|| {
            let db = db.clone();
            let ids = ids.clone();
            async move {
                if is_scoped {
                    tenant_scope(TenantContext::Scoped(Value::I64(TENANT_ID)), async {
                        run_lookup(&db, &ids).await;
                    })
                    .await;
                } else {
                    run_lookup(&db, &ids).await;
                }
            }
        })
    });

    // 诊断项：同样的原始 SQL，只多一个 `tenant_id = ?` 谓词，用于分离
    // 「谓词本身（SQL 解析 + 绑参 + 计划）」与「框架 scope 注入机制」的成本。
    if with_diagnostics {
        group.bench_function("point_lookup_raw_extra_predicate", |b| {
            let db = read_db.clone();
            b.to_async(rt).iter(|| {
                let db = db.clone();
                let ids = ids.clone();
                async move {
                    for id in &ids {
                        db.find_first_with_params(
                            "SELECT * FROM bench_tenant_users WHERE tenant_id = ? AND id = ? LIMIT 1",
                            vec![Value::I64(TENANT_ID), Value::I64(*id)],
                        )
                        .await
                        .expect("manual lookup")
                        .expect("row");
                    }
                }
            })
        });
        group.bench_function("point_lookup_raw_no_predicate", |b| {
            let db = read_db.clone();
            b.to_async(rt).iter(|| {
                let db = db.clone();
                let ids = ids.clone();
                async move {
                    for id in &ids {
                        db.find_first_with_params(
                            "SELECT * FROM bench_tenant_users WHERE id = ? LIMIT 1",
                            vec![Value::I64(*id)],
                        )
                        .await
                        .expect("manual lookup")
                        .expect("row");
                    }
                }
            })
        });
    }

    group.bench_function("pagination", |b| {
        let db = read_db.clone();
        b.to_async(rt).iter(|| {
            let db = db.clone();
            async move {
                if is_scoped {
                    tenant_scope(TenantContext::Scoped(Value::I64(TENANT_ID)), async {
                        run_pagination(&db).await;
                    })
                    .await;
                } else {
                    run_pagination(&db).await;
                }
            }
        })
    });

    group.bench_function("bulk_insert", |b| {
        let db = write_db.clone();
        b.to_async(rt).iter(|| {
            let db = db.clone();
            let rows = insert_rows.clone();
            async move {
                if is_scoped {
                    tenant_scope(TenantContext::Scoped(Value::I64(TENANT_ID)), async {
                        run_bulk_insert(&db, &rows).await;
                    })
                    .await;
                } else {
                    run_bulk_insert(&db, &rows).await;
                }
            }
        })
    });

    group.finish();
}

/// 基准入口：先关隔离，再开隔离（`bench_function` 注册即测量，顺序即结果分组）。
fn bench_tenant_isolation(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build tokio runtime");

    // 关闭：完全不注入租户条件（存量单租户行为）；在此组附带裸 SQL 对照项
    set_tenant_enabled(false);
    bench_mode(c, "isolation_off", &rt, true);

    // 开启：租户模型自动追加 `WHERE tenant_id = ?`
    set_tenant_enabled(true);
    bench_mode(c, "isolation_on", &rt, false);

    // 恢复默认（开启），避免影响同进程其他基准
    set_tenant_enabled(true);
}

criterion_group!(benches, bench_tenant_isolation);
criterion_main!(benches);

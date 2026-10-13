//! alun-db SQLite 基准：多量级下对比 **alun-db** 与 **rusqlite 裸驱动**。
//!
//! ## 为什么基线是 rusqlite
//!
//! `libsqlite3-sys` 是 `links` 类 crate，整个工作区只能存在**一个**版本
//! （当前 0.38.2）。sqlx / sea-orm / diesel 都要求 0.30~0.35 的 `libsqlite3-sys`，
//! 无法与本仓库共存，因此无法作为对照引入。`rusqlite` 既是 alun-db SQLite 驱动的
//! 底层库、又与之共用同一份 libsqlite3，是**最公平的下界基线**：alun-db 与它的差距
//! 就是框架抽象层（Model 映射 / 参数绑定 / 行解码）的真实开销。
//!
//! ## 测什么
//!
//! 每个量级覆盖四类操作，均为各自**惯用写法**：
//!
//! | 操作 | alun-db | rusqlite |
//! |---|---|---|
//! | 批量插入 | `insert_many`（自动多行 VALUES 分块，包在一个事务里） | 预处理语句逐行 + 事务 |
//! | 主键点查 | `find_or_none(db, id)` | `query_row` |
//! | 分页遍历 | `db.paginate_with_params`（COUNT + LIMIT/OFFSET） | COUNT + LIMIT/OFFSET |
//! | 全表聚合 | `db.find_with_params` 读 COUNT/SUM | `query_row` |
//!
//! ## 指标
//!
//! - **耗时**：重复 `RUNS` 次取中位数，降低抖动；
//! - **内存**：自定义 `GlobalAlloc` 统计本次操作在 **Rust 堆**上新分配的字节数与
//!   峰值增量。SQLite 的 C 侧内存（页缓存等）不在统计内，但两侧同源、开销一致，
//!   相对比较仍然成立。
//!
//! ## 口径说明（读结果前必看）
//!
//! - rusqlite 侧在循环外 `prepare` 一次并复用语句，alun-db 的 `Model` 查询当前
//!   **每次调用都重新准备语句**（驱动无语句缓存）。这是两者能力差异的真实体现，
//!   不是测量误差——也是点查差距的主要来源之一。
//! - rusqlite 的插入 / 聚合几乎不产生 Rust 堆分配（工作下沉到 C 侧 `sqlite3_*`），
//!   故其「内存」列接近 0；这不代表它总内存更小，只代表分配发生在 C 侧而未被统计。
//!   因此**内存列的绝对值不可跨库直接对比**，只适合看「同一侧随量级的增长」。
//!
//! 运行：`cargo bench -p alun-db --bench db_compare`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use alun_db::prelude::*;
use rusqlite::Connection;

// ==================== 计数分配器 ====================

/// 当前活跃（已分配未释放）字节数。
static LIVE: AtomicUsize = AtomicUsize::new(0);
/// 自上次重置以来的峰值活跃字节数。
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// 累计分配字节数。
static TOTAL: AtomicUsize = AtomicUsize::new(0);

/// 统计分配量的 `GlobalAlloc` 包装。
struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            TOTAL.fetch_add(layout.size(), Ordering::Relaxed);
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                let grew = new_size - layout.size();
                let live = LIVE.fetch_add(grew, Ordering::Relaxed) + grew;
                TOTAL.fetch_add(grew, Ordering::Relaxed);
                PEAK.fetch_max(live, Ordering::Relaxed);
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// 一次操作样本：耗时 + Rust 堆分配字节 + 峰值增量。
#[derive(Clone, Copy)]
struct Sample {
    /// 墙钟耗时。
    elapsed: Duration,
    /// 本次操作累计分配的字节数。
    alloc: usize,
    /// 本次操作期间活跃堆内存的峰值增量。
    peak: usize,
}

/// 运行一次并采样。
fn measure(run: &mut impl FnMut()) -> Sample {
    let total_before = TOTAL.load(Ordering::Relaxed);
    let live_before = LIVE.load(Ordering::Relaxed);
    PEAK.store(live_before, Ordering::Relaxed);

    let started = Instant::now();
    run();
    let elapsed = started.elapsed();

    Sample {
        elapsed,
        alloc: TOTAL.load(Ordering::Relaxed) - total_before,
        peak: PEAK.load(Ordering::Relaxed).saturating_sub(live_before),
    }
}

/// 重复 `RUNS` 次，取耗时中位数对应的样本（内存取同一次的值）。
fn bench(runs: usize, mut run: impl FnMut()) -> Sample {
    let mut samples: Vec<Sample> = (0..runs).map(|_| measure(&mut run)).collect();
    samples.sort_by_key(|s| s.elapsed);
    samples[samples.len() / 2]
}

// ==================== 被测模型与 SQL ====================

/// 基准用模型：`(id, name, email, age)`。
#[derive(Model, Debug, Clone)]
#[table("bench_users")]
struct BenchUser {
    /// 自增主键。
    #[key]
    #[auto]
    id: i64,
    /// 名称。
    name: String,
    /// 邮箱。
    email: String,
    /// 年龄。
    age: i32,
}

/// 建表语句（两侧共用，保证 schema 完全一致）。
const CREATE_TABLE: &str = "\
CREATE TABLE bench_users (
    id    INTEGER PRIMARY KEY AUTOINCREMENT,
    name  TEXT    NOT NULL,
    email TEXT    NOT NULL,
    age   INTEGER NOT NULL
)";

/// 全表查询（分页 / 聚合共用）。
const SELECT_ALL: &str = "SELECT id, name, email, age FROM bench_users ORDER BY id";

/// 聚合查询。
const AGGREGATE: &str = "SELECT COUNT(*) AS c, COALESCE(SUM(age), 0) AS s FROM bench_users";

/// 主键点查语句。
const SELECT_BY_ID: &str = "SELECT id, name, email, age FROM bench_users WHERE id = ?";

/// 单行插入语句。
const INSERT_ONE: &str = "INSERT INTO bench_users (name, email, age) VALUES (?, ?, ?)";

/// 基准量级（行数）。
const MAGNITUDES: [usize; 3] = [1_000, 10_000, 100_000];

/// 每个采样点的重复次数（取中位数）。
const RUNS: usize = 3;

/// 主键点查次数。
const LOOKUPS: usize = 1_000;

/// 分页页大小。
const PAGE_SIZE: u64 = 1_000;

/// 构造 `n` 行模型（在计时区之外预构造，避免把构造开销算进数据库操作）。
fn make_rows(n: usize) -> Vec<BenchUser> {
    (0..n)
        .map(|i| BenchUser {
            id: 0,
            name: format!("user{i}"),
            email: format!("user{i}@example.com"),
            age: (i % 90) as i32,
        })
        .collect()
}

/// 构造 `n` 组 rusqlite 绑定参数（同样在计时区之外预构造）。
fn make_params(n: usize) -> Vec<(String, String, i64)> {
    (0..n)
        .map(|i| {
            (
                format!("user{i}"),
                format!("user{i}@example.com"),
                (i % 90) as i64,
            )
        })
        .collect()
}

/// 线性同余随机数，生成确定性的主键序列（避免引入 rand 依赖）。
fn lookup_ids(n: usize, count: usize) -> Vec<i64> {
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    (0..count)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) as usize % n + 1) as i64
        })
        .collect()
}

// ==================== alun-db 侧 ====================

/// 新建一个内存库并建表。
async fn new_alun_db() -> Db {
    let db = Db::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    db.execute(CREATE_TABLE).await.expect("create table");
    db
}

/// 批量插入（一个事务 + 自动多行 VALUES 分块）。
async fn alun_insert(db: &Db, rows: &[BenchUser]) {
    db.transaction(|tx| async move { BenchUser::insert_many(rows, &tx).await })
        .await
        .expect("alun bulk insert");
}

/// 主键点查。
async fn alun_lookup(db: &Db, ids: &[i64]) {
    for id in ids {
        BenchUser::find_or_none(db, *id)
            .await
            .expect("alun lookup")
            .expect("row exists");
    }
}

/// `BenchUser::find_or_none` 实际执行的 SQL（Builder 渲染产物）。
const SELECT_BY_ID_RENDERED: &str = "SELECT * FROM \"bench_users\" WHERE \"id\" = ? LIMIT 1";

/// 主键点查（原始 SQL，文本与 Builder 渲染产物逐字一致）。
///
/// 与 [`alun_lookup`] 的唯一差别是不经 Builder 渲染与 Model 映射，
/// 二者差额即这两层的净开销。
async fn alun_lookup_rendered(db: &Db, ids: &[i64]) {
    for id in ids {
        db.find_first_with_params(SELECT_BY_ID_RENDERED, vec![Value::I64(*id)])
            .await
            .expect("alun rendered lookup")
            .expect("row exists");
    }
}

/// 分页遍历全表。
async fn alun_paginate(db: &Db) {
    let mut page = 1_u64;
    loop {
        let result = db
            .paginate_with_params(SELECT_ALL, vec![], page, PAGE_SIZE)
            .await
            .expect("alun paginate");
        if result.list.is_empty() || (page * PAGE_SIZE) >= result.total {
            break;
        }
        page += 1;
    }
}

/// 全表聚合。
async fn alun_aggregate(db: &Db) {
    let rows = db
        .find_with_params(AGGREGATE, vec![])
        .await
        .expect("alun aggregate");
    assert_eq!(rows.len(), 1);
}

// ==================== rusqlite 侧 ====================

/// 新建内存连接并建表。
fn new_rusqlite() -> Connection {
    let conn = Connection::open_in_memory().expect("open sqlite");
    conn.execute_batch(CREATE_TABLE).expect("create table");
    conn
}

/// 批量插入（事务 + 预处理语句逐行）。
fn rusqlite_insert(conn: &Connection, params: &[(String, String, i64)]) {
    let tx = conn.unchecked_transaction().expect("begin tx");
    {
        let mut stmt = tx.prepare(INSERT_ONE).expect("prepare insert");
        for (name, email, age) in params {
            stmt.execute(rusqlite::params![name, email, age])
                .expect("insert row");
        }
    }
    tx.commit().expect("commit");
}

/// 主键点查。
fn rusqlite_lookup(conn: &Connection, ids: &[i64]) {
    let mut stmt = conn.prepare(SELECT_BY_ID).expect("prepare lookup");
    for id in ids {
        let row = stmt
            .query_row(rusqlite::params![id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .expect("lookup row");
        std::hint::black_box(row);
    }
}

/// 分页遍历全表（COUNT + LIMIT/OFFSET，与框架实现等价）。
fn rusqlite_paginate(conn: &Connection) {
    let page_sql = format!("{SELECT_ALL} LIMIT ?1 OFFSET ?2");
    let mut stmt = conn.prepare(&page_sql).expect("prepare page");
    let mut page = 1_u64;
    loop {
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM bench_users", [], |row| row.get(0))
            .expect("count");
        let offset = (page - 1) * PAGE_SIZE;
        let rows = stmt
            .query_map(rusqlite::params![PAGE_SIZE as i64, offset as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            })
            .expect("page query");
        let fetched = rows.count();
        if fetched == 0 || (page * PAGE_SIZE) >= total as u64 {
            break;
        }
        page += 1;
    }
}

/// 全表聚合。
fn rusqlite_aggregate(conn: &Connection) {
    let (count, sum): (i64, i64) = conn
        .query_row(AGGREGATE, [], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("aggregate");
    std::hint::black_box((count, sum));
}

// ==================== 结果输出 ====================

/// 一行对照结果。
struct Row {
    /// 操作名。
    op: &'static str,
    /// 量级（行数）。
    magnitude: usize,
    /// alun-db 样本。
    alun: Sample,
    /// rusqlite 样本。
    rusqlite: Sample,
}

impl Row {
    /// 打印为 Markdown 表格行。比值 > 1 表示 alun-db 更快 / 更省内存。
    ///
    /// 内存列格式为 `峰值 / 累计`：峰值即操作期间活跃堆内存的增量（「占用」），
    /// 累计为该操作总共分配过的字节数（分配次数越多、malloc 开销越大）。
    fn print(&self) {
        println!(
            "| {} | {} | {} | {} | {:.2}× | {} | {} | {:.2}× |",
            self.op,
            thousands(self.magnitude),
            fmt_time(self.alun.elapsed),
            fmt_time(self.rusqlite.elapsed),
            ratio(self.rusqlite.elapsed, self.alun.elapsed),
            fmt_mem(self.alun),
            fmt_mem(self.rusqlite),
            ratio_u(self.rusqlite.peak, self.alun.peak),
        );
    }
}

/// 内存列：`峰值 / 累计`。
fn fmt_mem(sample: Sample) -> String {
    format!("{} / {}", fmt_bytes(sample.peak), fmt_bytes(sample.alloc))
}

/// 耗时比值（>1 表示前者更慢）。
fn ratio(of_baseline: Duration, of_target: Duration) -> f64 {
    let target = of_target.as_secs_f64();
    if target <= 0.0 {
        return f64::INFINITY;
    }
    of_baseline.as_secs_f64() / target
}

/// 字节比值（>1 表示 target 更省）。
fn ratio_u(of_baseline: usize, of_target: usize) -> f64 {
    if of_target == 0 {
        return f64::INFINITY;
    }
    of_baseline as f64 / of_target as f64
}

/// 毫秒格式化（小于 1ms 用微秒）。
fn fmt_time(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1_000.0;
    if ms < 1.0 {
        format!("{:.1} µs", ms * 1_000.0)
    } else {
        format!("{ms:.2} ms")
    }
}

/// 千分位分隔。
fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// 人类可读字节数。
fn fmt_bytes(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let b = bytes as f64;
    if b >= MB {
        format!("{:.2} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    println!("# alun-db SQLite 基准（alun-db vs rusqlite 裸驱动）\n");
    println!(
        "重复 {RUNS} 次取耗时中位数；内存为 Rust 堆「峰值 / 累计」字节（不含 SQLite C 侧）。\
         耗时比 > 1 表示 alun-db 更快，内存比 > 1 表示 alun-db 峰值更省。\n"
    );
    println!("| 操作 | 量级 | alun-db | rusqlite | 速度比 | alun 内存 | rusqlite 内存 | 内存比 |");
    println!("|---|---|---|---|---|---|---|---|");

    for magnitude in MAGNITUDES {
        let rows = make_rows(magnitude);
        let params = make_params(magnitude);
        let ids = lookup_ids(magnitude, LOOKUPS);

        // ---- 批量插入 ----
        let mut alun_samples = Vec::with_capacity(RUNS);
        let mut rusqlite_samples = Vec::with_capacity(RUNS);
        for _ in 0..RUNS {
            let db = runtime.block_on(new_alun_db());
            alun_samples.push(measure(&mut || {
                runtime.block_on(alun_insert(&db, &rows));
            }));

            let conn = new_rusqlite();
            rusqlite_samples.push(measure(&mut || rusqlite_insert(&conn, &params)));
        }
        Row {
            op: "批量插入",
            magnitude,
            alun: median(alun_samples),
            rusqlite: median(rusqlite_samples),
        }
        .print();

        // ---- 其余操作：各自装载一次数据后测量 ----
        let alun_db = runtime.block_on(async {
            let db = new_alun_db().await;
            alun_insert(&db, &rows).await;
            db
        });
        let rusqlite_conn = new_rusqlite();
        rusqlite_insert(&rusqlite_conn, &params);

        let lookup = Row {
            op: "主键点查",
            magnitude,
            alun: bench(RUNS, || {
                runtime.block_on(alun_lookup(&alun_db, &ids));
            }),
            rusqlite: bench(RUNS, || rusqlite_lookup(&rusqlite_conn, &ids)),
        };
        lookup.print();

        let paginate = Row {
            op: "分页遍历",
            magnitude,
            alun: bench(RUNS, || {
                runtime.block_on(alun_paginate(&alun_db));
            }),
            rusqlite: bench(RUNS, || rusqlite_paginate(&rusqlite_conn)),
        };
        paginate.print();

        let aggregate = Row {
            op: "全表聚合",
            magnitude,
            alun: bench(RUNS, || {
                runtime.block_on(alun_aggregate(&alun_db));
            }),
            rusqlite: bench(RUNS, || rusqlite_aggregate(&rusqlite_conn)),
        };
        aggregate.print();
    }

    // ---- 线程派发开销参照 ----
    // SQLite 驱动是同步库，每次查询都要派发到阻塞线程。这里测「空任务往返」的
    // 耗时作为该固定开销的下界，用于判断点查差距中线程派发占多大比重。
    // 两种运行时各测一次：当前线程运行时下唯一的工作线程必须停摆等待，
    // 是该开销的最坏情形；多线程运行时（生产的 axum 场景）工作线程无需停摆。
    const DISPATCH_PROBES: usize = 2_000;
    let single_cost = probe_spawn_blocking(&runtime, DISPATCH_PROBES);
    let multi = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("build multi-thread runtime");
    let multi_cost = probe_spawn_blocking(&multi, DISPATCH_PROBES);
    let in_place_cost = multi.block_on(async {
        let started = Instant::now();
        for _ in 0..DISPATCH_PROBES {
            tokio::task::block_in_place(|| ());
        }
        started.elapsed() / DISPATCH_PROBES as u32
    });
    println!("\n> 线程派发参照（空任务往返，{DISPATCH_PROBES} 次平均）：");
    println!(
        "- 当前线程运行时 `spawn_blocking`：{:.2} µs/次",
        micros(single_cost)
    );
    println!(
        "- 多线程运行时 `spawn_blocking`：{:.2} µs/次",
        micros(multi_cost)
    );
    println!(
        "- 多线程运行时 `block_in_place`：{:.2} µs/次",
        micros(in_place_cost)
    );

    // 多线程运行时下复测点查：驱动在该形态下走 `block_in_place`，
    // 与上表「当前线程运行时」（走 `spawn_blocking`）可直接对照。
    // 同一份数据再跑一遍逐字等价的原始 SQL，二者差额即 Builder 渲染 + Model 映射。
    let magnitude = MAGNITUDES[MAGNITUDES.len() - 1];
    let rows = make_rows(magnitude);
    let ids = lookup_ids(magnitude, LOOKUPS);
    let probe_db = multi.block_on(async {
        let db = new_alun_db().await;
        alun_insert(&db, &rows).await;
        db
    });
    let model_lookup = bench(RUNS, || {
        multi.block_on(alun_lookup(&probe_db, &ids));
    });
    let rendered_lookup = bench(RUNS, || {
        multi.block_on(alun_lookup_rendered(&probe_db, &ids));
    });
    let model = micros(model_lookup.elapsed) / LOOKUPS as f64;
    let rendered = micros(rendered_lookup.elapsed) / LOOKUPS as f64;
    println!(
        "\n> 主键点查分解（多线程运行时，{} 行库，{} 次）：",
        thousands(magnitude),
        LOOKUPS,
    );
    println!(
        "- `Model::find_or_none`（Builder 渲染 + Model 映射）：{model:.2} µs/次，\
         堆分配 {} B/次",
        model_lookup.alloc / LOOKUPS,
    );
    println!(
        "- 原始 SQL（与 Builder 渲染产物逐字一致）：{rendered:.2} µs/次，\
         堆分配 {} B/次",
        rendered_lookup.alloc / LOOKUPS,
    );
    println!(
        "- 差额（Builder 渲染 + Model 映射）：{:.2} µs/次",
        model - rendered
    );
}

/// 测空 `spawn_blocking` 往返的每次平均耗时。
fn probe_spawn_blocking(runtime: &tokio::runtime::Runtime, probes: usize) -> Duration {
    let total = runtime.block_on(async {
        let started = Instant::now();
        for _ in 0..probes {
            tokio::task::spawn_blocking(|| ())
                .await
                .expect("join probe");
        }
        started.elapsed()
    });
    total / probes as u32
}

/// 微秒数值。
fn micros(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

/// 取耗时中位数样本。
fn median(mut samples: Vec<Sample>) -> Sample {
    samples.sort_by_key(|s| s.elapsed);
    samples[samples.len() / 2]
}

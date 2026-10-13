//! 测试 harness：连接、迁移、隔离。

use std::path::PathBuf;

use alun_db::prelude::*;
use tokio::sync::{Mutex, MutexGuard};

/// 目标方言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// SQLite 内存库（每用例独立）。
    SqliteMem,
    /// SQLite 文件库（并发/长事务用例）。
    SqliteFile,
    /// PostgreSQL。
    Postgres,
    /// MySQL。
    Mysql,
}

impl Dialect {
    /// 是否为 SQLite。
    pub fn is_sqlite(self) -> bool {
        matches!(self, Dialect::SqliteMem | Dialect::SqliteFile)
    }

    /// 是否**单连接池**方言（仅 `sqlite-mem`：共享单连接，池容量 1）。
    ///
    /// 并发/嵌套语句只有在这种方言上会被池**串行化**（无真交叉锁定、无自等超时）。
    /// 文件库与 PG/MySQL 都是多连接：并发写会真的互相加锁，`sqlite-file` 上
    /// `SQLITE_BUSY` 映射为 `DbError::LockTimeout` 属**预期**（规划 LGR-004：
    /// 允许并发冲突失败，但必须守恒、无负余额）。
    pub fn is_single_connection(self) -> bool {
        matches!(self, Dialect::SqliteMem)
    }

    /// 方言名（用于报告与差异登记）。
    pub fn name(self) -> &'static str {
        match self {
            Dialect::SqliteMem => "sqlite-mem",
            Dialect::SqliteFile => "sqlite-file",
            Dialect::Postgres => "postgres",
            Dialect::Mysql => "mysql",
        }
    }

    /// 迁移目录（`migrations/<dialect>`）。
    pub fn migration_dir(self) -> &'static str {
        match self {
            Dialect::SqliteMem | Dialect::SqliteFile => "sqlite",
            Dialect::Postgres => "postgres",
            Dialect::Mysql => "mysql",
        }
    }
}

/// 当前进程的目标方言（`ALUN_LEDGER_DB`）。
pub fn dialect() -> Dialect {
    match std::env::var("ALUN_LEDGER_DB")
        .unwrap_or_else(|_| "sqlite-mem".into())
        .as_str()
    {
        "sqlite-file" => Dialect::SqliteFile,
        "postgres" | "pg" => Dialect::Postgres,
        "mysql" => Dialect::Mysql,
        _ => Dialect::SqliteMem,
    }
}

fn strict() -> bool {
    matches!(
        std::env::var("ALUN_LEDGER_MULTIDB_STRICT").as_deref(),
        Ok("1") | Ok("true")
    )
}

fn url_for(d: Dialect) -> String {
    match d {
        Dialect::SqliteMem => "sqlite::memory:".to_string(),
        // 路径按测试二进制隔离，见 `common::sqlite_file_path`。
        Dialect::SqliteFile => format!("sqlite://{}", super::sqlite_file_path()),
        Dialect::Postgres => std::env::var("ALUN_TEST_PG_URL")
            .unwrap_or_else(|_| "postgres://alun:alun@localhost:5432/alun_test".to_string()),
        Dialect::Mysql => std::env::var("ALUN_TEST_MYSQL_URL")
            .unwrap_or_else(|_| "mysql://alun:alun@localhost:3306/alun_test".to_string()),
    }
}

/// 共享库（PG/MySQL/文件库）上的进程内串行锁：避免并行用例互相清表。
static SHARED: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

/// 测试上下文。
pub struct LedgerCtx {
    /// 连接。
    pub db: Db,
    /// 方言。
    pub dialect: Dialect,
    _guard: Option<MutexGuard<'static, ()>>,
}

/// 外部数据库连接上限：PG/MySQL 端口被代理接受但后端不应答时不能无限阻塞
/// （否则会挂住整个 `cargo test`，本地无容器时尤其明显）。
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 建立连接（失败/超时返回 `None`；strict 模式下 panic）。
pub async fn connect(d: Dialect) -> Option<Db> {
    let url = url_for(d);
    let outcome = match tokio::time::timeout(PROBE_TIMEOUT, Db::connect(&url)).await {
        Ok(Ok(db)) => return Some(db),
        Ok(Err(e)) => format!("无法连接 {url}：{e}"),
        Err(_) => format!("连接 {url} 超时（>{}s）", PROBE_TIMEOUT.as_secs()),
    };
    if strict() {
        panic!("{outcome}（ALUN_LEDGER_MULTIDB_STRICT=1）");
    }
    eprintln!("跳过 {} 用例（{outcome}）", d.name());
    None
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 清理共享库上的业务数据（PG/MySQL/文件库）。
///
/// 共享库上不同用例/不同测试二进制会累积数据（SQLite 内存库天然隔离）。
/// 由于 `ctx*()` 已持有进程内串行锁，这里直接清空业务表即可等价获得"每用例干净库"；
/// `schema_migrations` 只保留真实迁移版本（1..27），并清掉测试自己写入的探针表。
pub async fn clean_business_data(db: &Db) {
    const BUSINESS: [&str; 10] = [
        "invoice_items",
        "invoices",
        "ledger_entries",
        "transfers",
        "accounts",
        "usage_records",
        "subscriptions",
        "audit_logs",
        "users",
        "tenants",
    ];
    for table in BUSINESS {
        db.execute(&format!("DELETE FROM {table}"))
            .await
            .unwrap_or_else(|e| panic!("clean {table} 失败: {e}"));
    }
    // 测试自建探针表（可能不存在，忽略错误）
    for table in [
        "type_probe",
        "money_probe",
        "enum_probe",
        "hook_probe",
        "audit_probe",
        "ts_probe",
        "null_probe",
        "array_probe",
        "err_probe",
    ] {
        let _ = db.execute(&format!("DELETE FROM {table}")).await;
    }
    // 迁移用例会临时插入非 1..27 的版本行（如 9001），避免污染后续版本集合断言
    let _ = db
        .execute("DELETE FROM schema_migrations WHERE version > 27")
        .await;
}

/// 应用迁移：**必须先建表（01-base）→ 再跑 common（演进列）→ 最后 02-evolve**
/// （回填依赖 common 新增的列；版本号在三个目录间全局唯一）。
pub async fn migrate(db: &Db, d: Dialect) {
    let root = manifest_dir().join("migrations");

    // ── 缺陷 D023/D024 变通（仅非 SQLite）────────────────────────────
    // 全新库路径：`schema_migrations` 为空时框架会「探测并导入存量 sqlx 迁移表
    // `_sqlx_migrations`」。该路径曾有两个缺陷（均已修复）：
    //   D023：`is_missing_table` 只看 `source` 链第一层，PG/MySQL 把真实错误包了
    //         一层 `db error` → 「表不存在」被判成真实错误，迁移直接失败；
    //   D024：导入语句的 `?` 占位符在 PG 上被推断为 text，而 `applied_at` 是 bigint
    //         → 语句规划期即报错。
    // 修复后（D023 遍历整条 source 链；D024 改用零占位符语句）不再需要
    // 「预建 schema_migrations + 哨兵行」的变通，这里直接走框架路径。

    let dirs = [
        root.join(d.migration_dir()).join("01-base"),
        root.join("common"),
        root.join(d.migration_dir()).join("02-evolve"),
    ];
    for dir in dirs {
        if let Err(e) = Migrator::from_dir(&dir).run(db).await {
            // DbError::Query 的 Display 只含 SQL 文本，真实原因在 source() 链上（缺陷 D019）
            let mut chain = String::new();
            let mut cur: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(&e);
            while let Some(s) = cur {
                chain.push_str(&s.to_string());
                chain.push_str(" <- ");
                cur = s.source();
            }
            panic!("migrate {} 失败: {e} | source: {chain}", dir.display());
        }
    }
}

/// 建立测试上下文（连接 + 隔离锁 + 迁移）。
pub async fn ctx() -> Option<LedgerCtx> {
    let d = dialect();
    let db = connect(d).await?;
    let guard = if d.is_sqlite() && d == Dialect::SqliteMem {
        None
    } else {
        Some(SHARED.lock().await)
    };
    migrate(&db, d).await;
    if guard.is_some() {
        clean_business_data(&db).await;
    }
    Some(LedgerCtx {
        db,
        dialect: d,
        _guard: guard,
    })
}

/// 指定方言建立上下文（跨库用例显式声明目标库时使用）。
pub async fn ctx_for(d: Dialect) -> Option<LedgerCtx> {
    let db = connect(d).await?;
    let guard = if d == Dialect::SqliteMem {
        None
    } else {
        Some(SHARED.lock().await)
    };
    migrate(&db, d).await;
    if guard.is_some() {
        clean_business_data(&db).await;
    }
    Some(LedgerCtx {
        db,
        dialect: d,
        _guard: guard,
    })
}

impl LedgerCtx {
    /// 引用连接。
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// 断言语义：库为 3 时表示三库都应通过（调用方按需 skip）。
    pub fn require(&self, d: Dialect) -> bool {
        self.dialect == d
    }
}

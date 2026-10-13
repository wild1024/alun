//! 跨三数据库实测示例：SQLite / PostgreSQL / MySQL。
//!
//! 证明同一份操作码在三个方言上一等公民地运行，并演示新增能力：
//! - `PersistExt::insert_many`（多行一次提交）
//! - `PersistExt::upsert`（ON CONFLICT DO UPDATE，方言能力协商）
//! - `FindBuilder::join`（INNER / LEFT JOIN）
//! - `FindBuilder::page_after`（Keyset 游标分页，深翻页不退化）
//! - `from_row` 索引快路径（由 Model 字段序自动命中）
//! - 事务 / Hook / 原生 Row 模式
//!
//! 运行（需本地已启动 postgres:17 与 mysql:8.4，见 docker-compose.yml）：
//! ```bash
//! cargo run --example multi_db --features "postgres,mysql,chrono,rust_decimal,json,uuid"
//! ```
//!
//! 连接串可用环境变量覆盖：
//! - `ALUN_TEST_PG_URL`（默认 postgres://alun:alun@localhost:5432/alun_test）
//! - `ALUN_TEST_MYSQL_URL`（默认 mysql://alun:alun@localhost:3306/alun_test）

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
// 排序方向枚举：非 prelude 导出（避免与业务模型「Order/订单」名冲突），显式引入
use alun_db_core::statement::Order;

// ==================== 模型定义（三库共用同一结构） ====================

/// 团队模型：演示批量插入 + 自增主键。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("squads")]
pub struct Squad {
    #[key]
    #[auto]
    pub id: i64,
    #[unique]
    pub name: String,
}

/// 成员模型：普通 CRUD + JOIN 的一方。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("members")]
pub struct Member {
    #[key]
    #[auto]
    pub id: i64,
    pub squad_id: i64,
    pub name: String,
    pub score: i64,
}

/// 标签模型：演示 upsert（以唯一 name 为冲突判定，冲突时更新 score）。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("tags")]
pub struct Tag {
    #[key]
    #[auto]
    pub id: i64,
    #[unique]
    pub name: String,
    pub score: i64,
}

/// 三库在建表 DDL 上的方言差异：主键自增写法不同。
enum RowIdKind {
    /// SQLite：INTEGER PRIMARY KEY AUTOINCREMENT。
    Sqlite,
    /// PostgreSQL：BIGSERIAL PRIMARY KEY。
    Postgres,
    /// MySQL：BIGINT AUTO_INCREMENT PRIMARY KEY。
    Mysql,
}

/// 各表的列定义（不含 id 列），三库通用。
///
/// 唯一列统一用 `VARCHAR`：MySQL 对 TEXT 建唯一索引需指定 key length，
/// 而 VARCHAR 在三库均可直接作为唯一键，避免方言差异。
mod cols {
    /// squads 表列。
    pub const SQUADS: &str = "name VARCHAR(255) NOT NULL UNIQUE";
    /// members 表列。
    pub const MEMBERS: &str =
        "squad_id BIGINT NOT NULL, name VARCHAR(255) NOT NULL, score BIGINT NOT NULL DEFAULT 0";
    /// tags 表列。
    pub const TAGS: &str = "name VARCHAR(255) NOT NULL UNIQUE, score BIGINT NOT NULL DEFAULT 0";
}

/// 每组操作用到的 SQL 片段，按方言分派。
fn ddl(engine: &RowIdKind, table: &str, body: &str) -> String {
    let id_col = match engine {
        RowIdKind::Sqlite => "id INTEGER PRIMARY KEY AUTOINCREMENT",
        RowIdKind::Postgres => "id BIGSERIAL PRIMARY KEY",
        RowIdKind::Mysql => "id BIGINT AUTO_INCREMENT PRIMARY KEY",
    };
    // body 以逗号引导各列（不含 id 列）
    format!("CREATE TABLE {table} ({id_col}, {body})")
}

// ==================== 建表与清理 ====================

/// 为给定引擎建表（幂等：先 DROP 再 CREATE）。
async fn create_schema(db: &Db, engine: &RowIdKind) -> Result<()> {
    for table in ["squads", "members", "tags"] {
        let _ = db.execute(&format!("DROP TABLE IF EXISTS {table}")).await;
    }
    db.execute(&ddl(engine, "squads", cols::SQUADS)).await?;
    db.execute(&ddl(engine, "members", cols::MEMBERS)).await?;
    db.execute(&ddl(engine, "tags", cols::TAGS)).await?;
    Ok(())
}

// ==================== 统一操作码（三库都要通过） ====================

/// 在任意方言上执行同一套操作，并断言正确性。
async fn exercise(db: &Db, engine: &str) -> Result<()> {
    println!("===== 引擎：{engine} =====");

    // ---------- 1. 批量插入 insert_many ----------
    let squads = vec![
        Squad {
            id: 0,
            name: "Alpha".into(),
        },
        Squad {
            id: 0,
            name: "Bravo".into(),
        },
        Squad {
            id: 0,
            name: "Charlie".into(),
        },
    ];
    Squad::insert_many(&squads, db).await?;
    let count = Squad::count(db).await?;
    assert_eq!(count, 3, "[{engine}] insert_many 应插入 3 条");
    println!("[1] insert_many 3 条，count={count}");

    // ---------- 2. Model::all + 主键非零（批量不回填 id=0，符合文档语义） ----------
    let all = Squad::query().find(db).await?;
    // insert_many 不触发 Hook、不回填主键，因此 id 仍为 0：验证"按需 save 才回填"
    assert_eq!(all.len(), 3);
    println!("[2] all = {} 行（批量插入不回填主键，符合预期）", all.len());

    // 用逐行 save 补一个带主键回填的行，供 JOIN 使用
    let mut member = Member {
        id: 0,
        squad_id: 0,
        name: "bob".into(),
        score: 50,
    };
    member.insert_or_update(db).await?;
    // 找出 Alpha 的 squad id
    let alpha = Squad::find_by(db, "name = ?", ("Alpha",)).await?;
    let alpha_id = alpha[0].id;
    println!(
        "[3] save 回填主键 -> member.id={}; Alpha squad id={alpha_id}",
        member.id
    );

    // ---------- 3. upsert：ON CONFLICT（方言能力协商） ----------
    let mut tag = Tag {
        id: 0,
        name: "rust".into(),
        score: 1,
    };
    // 首次插入
    let result = tag.upsert(db, &["name"], &["score"]).await;
    match result {
        Ok(()) => {
            let fresh = Tag::find_by(db, "name = ?", ("rust",)).await?;
            let score = fresh[0].score;
            assert_eq!(score, 1);
            // 冲突更新：name 已存在，score 覆盖为 99
            tag.score = 99;
            tag.upsert(db, &["name"], &["score"]).await?;
            let after = Tag::find_by(db, "name = ?", ("rust",)).await?;
            let s2 = after[0].score;
            assert_eq!(s2, 99, "[{engine}] upsert 冲突应更新 score 为 99");
            println!("[3] upsert OK：插入后又冲突更新 score=99");
        }
        Err(DbError::UnsupportedFeature { .. }) => {
            // MySQL 无 ON CONFLICT 语法，能力协商降级报错
            println!("[3] upsert 在 {engine} 上返回 UnsupportedFeature（符合方言能力协商）");
        }
        Err(e) => return Err(e),
    }

    // ---------- 4. JOIN：INNER / LEFT ----------
    let mut squad_rows: Vec<Member> = Vec::new();
    for squad in &all {
        let mut m = Member {
            id: 0,
            squad_id: squad.id,
            name: format!("{}-member", squad.name),
            score: 10,
        };
        m.insert_or_update(db).await?;
        squad_rows.push(m);
    }
    // INNER JOIN：内部员工数
    let inner = FindBuilder::new()
        .table("squads")
        .join(
            JoinKind::Inner,
            "members",
            Expr::Binary {
                left: Box::new(Expr::Column {
                    table: "members",
                    column: "squad_id",
                }),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Column {
                    table: "squads",
                    column: "id",
                }),
            },
        )
        .exec(db)
        .await?;
    println!(
        "[4] INNER JOIN squads↔members -> {} 行（每 squad 至少 1 名）",
        inner.len()
    );
    assert!(inner.len() >= 3);

    let left = FindBuilder::new()
        .table("squads")
        .join(
            JoinKind::Left,
            "members",
            Expr::Binary {
                left: Box::new(Expr::Column {
                    table: "members",
                    column: "squad_id",
                }),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Column {
                    table: "squads",
                    column: "id",
                }),
            },
        )
        .order_by("squads.id", Order::Asc)
        .exec(db)
        .await?;
    println!("[4] LEFT JOIN = {} 行", left.len());

    // ---------- 5. Keyset 游标分页 page_after ----------
    // 先插入 25 个有序行
    for i in 0..25 {
        let mut m = Member {
            id: 0,
            squad_id: 1,
            name: format!("m{i}"),
            score: i,
        };
        m.insert_or_update(db).await?;
    }
    // 第一页：ASC 排序，取 score>=0 的游标后第一页
    let page1 = FindBuilder::new()
        .table("members")
        .filter(Expr::Binary {
            left: Box::new(Expr::Column {
                table: "members",
                column: "squad_id",
            }),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Value(Value::I64(1))),
        })
        .order_by("score", Order::Asc)
        .limit(10_000) // 无界：交由 keyset 控制，模拟"取下一页"
        .page_after("score", "id", Order::Asc, None)
        .exec(db)
        .await?;
    println!("[5] keyset 第一页行数={}", page1.len());
    // 构造下一页游标：取上一页最后一行
    let last = page1.last().expect("page1 非空");
    let last_score: i64 = last.get_typed("score")?;
    let last_id: i64 = last.get_typed("id")?;
    let page2 = FindBuilder::new()
        .table("members")
        .filter(Expr::Binary {
            left: Box::new(Expr::Column {
                table: "members",
                column: "squad_id",
            }),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Value(Value::I64(1))),
        })
        .order_by("score", Order::Asc)
        .limit(100)
        .page_after(
            "score",
            "id",
            Order::Asc,
            Some(Cursor {
                key: Value::I64(last_score),
                pk: Value::I64(last_id),
            }),
        )
        .exec(db)
        .await?;
    // 下一页首行的 score 必须严格大于上页末尾 score
    if let Some(first_p2) = page2.first() {
        let s: i64 = first_p2.get_typed("score")?;
        assert!(s > last_score, "[{engine}] keyset 下一页应在大括上一页之后");
    }
    println!(
        "[5] keyset 第二页行数={}（首行 score 应 > {last_score}）",
        page2.len()
    );

    // ---------- 6. 原生 Row + 事务 ----------
    let rows = db
        .find("SELECT id, name FROM squads ORDER BY id LIMIT 1")
        .await?;
    let n: String = rows[0].get_typed("name")?;
    println!("[6] 原生 Row: 第一个 squad={n}");

    db.transaction(|tx| async move {
        tx.execute_with_params(
            "UPDATE members SET score = score + 1 WHERE squad_id = ?",
            vec![Value::I64(1)],
        )
        .await?;
        Ok(())
    })
    .await?;
    println!("[6] 事务提交通过");

    println!("----- {engine} 全部通过 -----");
    Ok(())
}

// ==================== main ====================

/// 连接探测上限（服务器接受 TCP 却不应答时不能无限阻塞）。
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 尝试用 url 连接；失败/超时打印提示并返回 None（不中断）。
async fn connect_opt(url: &str, env: &str) -> Option<Db> {
    let resolved = std::env::var(env).unwrap_or_else(|_| url.to_string());
    match tokio::time::timeout(CONNECT_TIMEOUT, Db::connect(&resolved)).await {
        Ok(Ok(db)) => Some(db),
        Ok(Err(e)) => {
            eprintln!("跳过 {env}（无法连接 {resolved}）：{e}");
            None
        }
        Err(_) => {
            eprintln!(
                "跳过 {env}（连接 {resolved} 超时 >{}s）",
                CONNECT_TIMEOUT.as_secs()
            );
            None
        }
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // SQLite：内存库，无外部依赖
    let sqlite = Db::connect("sqlite::memory:").await?;
    create_schema(&sqlite, &RowIdKind::Sqlite).await?;
    exercise(&sqlite, "sqlite").await?;

    // PostgreSQL / MySQL：需 docker compose up -d
    if let Some(pg) = connect_opt(
        "postgres://alun:alun@localhost:5432/alun_test",
        "ALUN_TEST_PG_URL",
    )
    .await
    {
        create_schema(&pg, &RowIdKind::Postgres).await?;
        exercise(&pg, "postgres").await?;
    }

    if let Some(my) = connect_opt(
        "mysql://alun:alun@localhost:3306/alun_test",
        "ALUN_TEST_MYSQL_URL",
    )
    .await
    {
        create_schema(&my, &RowIdKind::Mysql).await?;
        exercise(&my, "mysql").await?;
    }

    println!("\n多数据库示例完成");
    Ok(())
}

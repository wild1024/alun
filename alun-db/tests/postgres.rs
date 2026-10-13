//! PostgreSQL 端到端集成测试。
//!
//! 依赖 docker compose 提供的实例（postgres:17-alpine）：
//!
//! ```text
//! docker compose up -d
//! cargo test -p alun-db --features postgres,chrono,uuid,rust_decimal,json --test postgres
//! ```
//!
//! 连接串可通过 `ALUN_TEST_PG_URL` 覆盖；连接失败时打印原因并跳过
//! （本地无 docker 环境不红测试）。

#![cfg(feature = "postgres")]

use alun_db::prelude::*;
use alun_db_core::error::DbError;
use alun_db_core::stmt::Value;
use alun_db_core::stmt::compact::CompactDecimal;
use chrono::Utc;
use tokio::sync::OnceCell;

/// 一次性建表栅栏：仅首个测试在独立连接上完成 `DROP + CREATE`，
/// 后续并发测试复用既有 schema（不重复 DROP），避免相互删表。
static SCHEMA_READY: OnceCell<()> = OnceCell::const_new();

/// docker-compose.yml 中的默认连接串。
const DEFAULT_PG_URL: &str = "postgres://alun:alun@localhost:5432/alun_test";

/// 连接探测上限：服务器接受 TCP 却不应答时（端口代理指向未就绪容器）不能无限阻塞，
/// 否则单个用例会挂住整个 `cargo test`。
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 不可用原因缓存：探测失败后同一测试二进制内的后续用例直接跳过，不再逐个等超时。
static PG_DOWN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// 连接测试库。失败/超时返回 `None`（跳过测试）。
async fn setup_pg() -> Option<Db> {
    if let Some(msg) = PG_DOWN.get() {
        eprintln!("跳过 PG 测试（{msg}）");
        return None;
    }
    let url = std::env::var("ALUN_TEST_PG_URL").unwrap_or_else(|_| DEFAULT_PG_URL.to_string());
    let msg = match tokio::time::timeout(PROBE_TIMEOUT, Db::connect(&url)).await {
        Ok(Ok(db)) => return Some(db),
        Ok(Err(e)) => format!("无法连接 {url}：{e}"),
        Err(_) => format!("连接 {url} 超时（>{}s）", PROBE_TIMEOUT.as_secs()),
    };
    let _ = PG_DOWN.set(msg.clone());
    eprintln!("跳过 PG 测试（{msg}）");
    None
}

/// 建表：覆盖常用类型（自增主键 / 唯一约束 / JSONB / TIMESTAMPTZ / NUMERIC）。
///
/// 仅首个测试执行的调用执行一次 `DROP + CREATE` 并写入 `SCHEMA_READY`；
/// 其他并发测试等待栅栏放行后直接复用 schema，不再重复建/删表。
async fn create_tables(db: &Db) {
    SCHEMA_READY
        .get_or_init(|| async {
            // 清理上次残留，保证首次建表干净
            let _ = db.execute("DROP TABLE IF EXISTS pg_users").await;
            let _ = db.execute("DROP TABLE IF EXISTS pg_products").await;
            db.execute(
                "CREATE TABLE IF NOT EXISTS pg_users (
                    id BIGSERIAL PRIMARY KEY,
                    email TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    status INTEGER NOT NULL DEFAULT 1,
                    score NUMERIC(10, 3),
                    profile JSONB,
                    created_at TIMESTAMPTZ NOT NULL
                )",
            )
            .await
            .expect("create pg_users");
            db.execute(
                "CREATE TABLE IF NOT EXISTS pg_products (
                    code TEXT PRIMARY KEY,
                    price NUMERIC(10, 2) NOT NULL
                )",
            )
            .await
            .expect("create pg_products");
        })
        .await;
}

/// 插入参数的公共部分（时间戳经 `From<DateTime<Utc>>` 转换）。
fn user_params(email: &str, name: &str) -> Vec<Value> {
    vec![
        Value::Text(email.to_string()),
        Value::Text(name.to_string()),
        Value::I32(1),
        Utc::now().into(),
    ]
}

#[derive(Model, Debug, Clone, PartialEq)]
#[table("pg_users")]
struct PgUser {
    #[key]
    #[auto]
    id: i64,
    #[unique]
    email: String,
    name: String,
    status: i32,
    score: Option<rust_decimal::Decimal>,
    #[json]
    profile: Option<Profile>,
    created_at: chrono::DateTime<Utc>,
}

#[derive(Row, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Profile {
    avatar: Option<String>,
    bio: String,
}

/// 基础 CRUD + RETURNING 主键回填。
#[tokio::test]
async fn crud_with_returning_id() {
    let Some(db) = setup_pg().await else { return };
    create_tables(&db).await;

    let mut user = PgUser {
        id: 0,
        email: "alice@example.com".into(),
        name: "Alice".into(),
        status: 1,
        score: Some(rust_decimal::Decimal::new(12345, 2)),
        profile: Some(Profile {
            avatar: None,
            bio: "hello".into(),
        }),
        created_at: Utc::now(),
    };
    user.insert_or_update(&db).await.expect("insert");
    // PG 路径：INSERT ... RETURNING 取回自增主键
    assert!(user.id > 0, "RETURNING should populate auto id");

    user.name = "Alice Updated".into();
    user.update(&db).await.expect("update");

    let fetched = PgUser::find(&db, user.id).await.expect("find");
    assert_eq!(fetched.name, "Alice Updated");
    assert_eq!(
        fetched.score.expect("score"),
        rust_decimal::Decimal::new(12345, 2)
    );
    assert_eq!(fetched.profile.expect("profile").bio, "hello");

    let affected = user.delete(&db).await.expect("delete");
    assert_eq!(affected, 1);
}

/// `?` 占位符手写路径（驱动内改写为 `$n`）。
#[tokio::test]
async fn raw_placeholder_rewrite() {
    let Some(db) = setup_pg().await else { return };
    create_tables(&db).await;

    db.execute_with_params(
        "INSERT INTO pg_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
        vec![
            Value::Text("bob@example.com".into()),
            Value::Text("Bob".into()),
            Value::I32(2),
            Utc::now().into(),
        ],
    )
    .await
    .expect("insert");

    let rows = db
        .find_with_params(
            "SELECT id, name FROM pg_users WHERE status = ? AND name LIKE ?",
            vec![2i32.into(), "B%".into()],
        )
        .await
        .expect("query");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get_typed::<String>("name").expect("name"), "Bob");
}

/// 类型往返：NUMERIC / JSONB / TIMESTAMPTZ。
#[tokio::test]
async fn type_roundtrip() {
    let Some(db) = setup_pg().await else { return };
    create_tables(&db).await;

    let now = Utc::now();
    db.execute_with_params(
        "INSERT INTO pg_users (email, name, status, score, profile, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
        vec![
            Value::Text("carol@example.com".into()),
            Value::Text("Carol".into()),
            Value::I32(1),
            Value::Decimal(CompactDecimal {
                mantissa: -6789,
                scale: 3,
            }),
            Value::Json(r#"{"bio":"hi"}"#.into()),
            now.into(),
        ],
    )
    .await
    .expect("insert");

    let row = db
        .find_first_with_params(
            "SELECT score, profile, created_at FROM pg_users WHERE email = ?",
            vec![Value::Text("carol@example.com".into())],
        )
        .await
        .expect("find")
        .expect("row");

    let score: rust_decimal::Decimal = row.get_typed("score").expect("score");
    assert_eq!(score, rust_decimal::Decimal::new(-6789, 3));

    let profile: serde_json::Value = row.get_typed("profile").expect("profile");
    assert_eq!(profile["bio"], "hi");

    let ts: chrono::DateTime<Utc> = row.get_typed("created_at").expect("created_at");
    // 微秒精度往返
    assert_eq!(ts.timestamp_micros(), now.timestamp_micros());
}

/// 唯一约束冲突映射为 `DbError::UniqueViolation`。
#[tokio::test]
async fn unique_violation_mapping() {
    let Some(db) = setup_pg().await else { return };
    create_tables(&db).await;

    db.execute_with_params(
        "INSERT INTO pg_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
        user_params("dave@example.com", "Dave"),
    )
    .await
    .expect("first insert succeeds");

    let result = db
        .execute_with_params(
            "INSERT INTO pg_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
            user_params("dave@example.com", "Duplicate"),
        )
        .await;
    match result {
        Err(DbError::UniqueViolation { .. }) => {}
        other => panic!("expected UniqueViolation, got {other:?}"),
    }
}

/// 事务三态：提交持久化 / 回滚丢弃 / Drop 丢弃连接回滚。
#[tokio::test]
async fn transaction_semantics() {
    let Some(db) = setup_pg().await else { return };
    create_tables(&db).await;

    // 1. 闭包式提交
    db.transaction(|tx| {
        Box::pin(async move {
            tx.execute_with_params(
                "INSERT INTO pg_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
                user_params("eve@example.com", "Eve"),
            )
            .await?;
            Ok(())
        })
    })
    .await
    .expect("commit");
    let count: i64 = db
        .scalar_with_params(
            "SELECT COUNT(*) FROM pg_users WHERE email = ?",
            vec![Value::Text("eve@example.com".into())],
        )
        .await
        .expect("count");
    assert_eq!(count, 1);

    // 2. 闭包式回滚（`?` 传播错误）
    let result: alun_db_core::error::Result<()> = db
        .transaction(|tx| {
            Box::pin(async move {
                tx.execute_with_params(
                    "INSERT INTO pg_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
                    user_params("frank@example.com", "Frank"),
                )
                .await?;
                // 制造失败触发回滚
                Err(DbError::InvalidValue("rollback".into()))
            })
        })
        .await;
    assert!(result.is_err());
    let count: i64 = db
        .scalar_with_params(
            "SELECT COUNT(*) FROM pg_users WHERE email = ?",
            vec![Value::Text("frank@example.com".into())],
        )
        .await
        .expect("count");
    assert_eq!(count, 0);

    // 3. Drop 未终结事务：连接丢弃触发服务端回滚
    {
        let tx = db.begin().await.expect("begin");
        tx.execute_with_params(
            "INSERT INTO pg_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
            user_params("grace@example.com", "Grace"),
        )
        .await
        .expect("insert in tx");
        // tx 在此 Drop，未提交 → 服务端断连自动回滚
    }
    let count: i64 = db
        .scalar_with_params(
            "SELECT COUNT(*) FROM pg_users WHERE email = ?",
            vec![Value::Text("grace@example.com".into())],
        )
        .await
        .expect("count");
    assert_eq!(count, 0, "dropped transaction should roll back");
}

/// NUMERIC 值读写（自然键表）。
#[tokio::test]
async fn numeric_read_write() {
    let Some(db) = setup_pg().await else { return };
    create_tables(&db).await;

    db.execute_with_params(
        "INSERT INTO pg_products (code, price) VALUES (?, ?)",
        vec![
            Value::Text("SKU-1".into()),
            Value::Decimal(CompactDecimal {
                mantissa: 1999,
                scale: 2,
            }),
        ],
    )
    .await
    .expect("insert");

    let affected = db
        .execute_with_params(
            "UPDATE pg_products SET price = ? WHERE code = ?",
            vec![
                Value::Decimal(CompactDecimal {
                    mantissa: 2499,
                    scale: 2,
                }),
                Value::Text("SKU-1".into()),
            ],
        )
        .await
        .expect("update");
    assert_eq!(affected, 1);

    let price: rust_decimal::Decimal = db
        .scalar_with_params(
            "SELECT price FROM pg_products WHERE code = ?",
            vec![Value::Text("SKU-1".into())],
        )
        .await
        .expect("price");
    assert_eq!(price, rust_decimal::Decimal::new(2499, 2));
}

/// 池并发：多任务并行借出与归还（Clone 共享同一连接池）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pool_concurrency() {
    let Some(db) = setup_pg().await else { return };
    create_tables(&db).await;

    let mut handles = Vec::new();
    for i in 0..8 {
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            db.execute_with_params(
                "INSERT INTO pg_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
                user_params(&format!("pool{i}@example.com"), &format!("Pool{i}")),
            )
            .await
        }));
    }
    for handle in handles {
        handle
            .await
            .expect("join")
            .expect("insert through pooled connection");
    }

    let count: i64 = db
        .scalar("SELECT COUNT(*) FROM pg_users WHERE email LIKE 'pool%@example.com'")
        .await
        .expect("count");
    assert_eq!(count, 8);
}

/// 回归（D026）：写入的时间戳必须与**服务端**看到的一致。
///
/// PG 二进制时间戳以 PG 纪元（2000-01-01）为基准；写入侧曾把纪元差**加**了一次
/// （正确应为减），库内实际时间被推后约 60 年。由于读回侧用的是同一个错误偏移，
/// 框架内往返「自洽」，只有服务端函数（`to_char` / `extract(epoch)`）或其它客户端
/// 才能看见错误——因此本用例断言服务端视角与反向字面量读回。
#[tokio::test]
async fn timestamp_stored_value_matches_server_epoch() {
    let Some(db) = setup_pg().await else { return };
    create_tables(&db).await;

    // 2026-01-01T00:00:00Z
    const MICROS: i64 = 1_767_225_600_000_000;
    let instant: chrono::DateTime<Utc> =
        chrono::DateTime::from_timestamp_micros(MICROS).expect("instant");
    db.execute_with_params(
        "INSERT INTO pg_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
        vec![
            Value::Text("ts-epoch@example.com".into()),
            Value::Text("Ts".into()),
            Value::I32(1),
            instant.into(),
        ],
    )
    .await
    .expect("insert");

    let rendered: String = db
        .scalar_with_params(
            "SELECT to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS')
             FROM pg_users WHERE email = ?",
            vec![Value::Text("ts-epoch@example.com".into())],
        )
        .await
        .expect("to_char");
    assert_eq!(
        rendered, "2026-01-01T00:00:00",
        "库内实际时间必须等于写入瞬间（D026：纪元差方向写反）"
    );

    // PG 17 的 extract() 返回 NUMERIC（旧版为 double precision），按 Decimal 读
    let epoch: rust_decimal::Decimal = db
        .scalar_with_params(
            "SELECT extract(epoch FROM created_at) FROM pg_users WHERE email = ?",
            vec![Value::Text("ts-epoch@example.com".into())],
        )
        .await
        .expect("extract epoch");
    assert_eq!(
        epoch,
        rust_decimal::Decimal::new(1_767_225_600, 0),
        "extract(epoch) 必须等于写入瞬间的 Unix 秒"
    );

    // 反向：服务端字面量经驱动读回，也必须落在同一瞬间
    let back: chrono::DateTime<Utc> = db
        .scalar("SELECT TIMESTAMPTZ '2026-01-01 00:00:00+00'")
        .await
        .expect("literal readback");
    assert_eq!(
        back.timestamp_micros(),
        MICROS,
        "服务端字面量读回必须无损（读回侧同为纪元差）"
    );
}

/// 回归（D025）：NUMERIC 的**服务端表示**必须与写入值一致。
///
/// PG 发送的 numeric 已规范化：尾随零组不出现在 `digits` 中而是抬高 `weight`
/// （10000 = digits[1]/weight[1]）。解码侧曾忽略 `weight` 超出数字组的部分，
/// 于是 10000 解成 1、1e10 解成 100 —— 只有服务端文本或外部客户端能发现。
#[tokio::test]
async fn numeric_matches_server_representation() {
    let Some(db) = setup_pg().await else { return };
    db.execute("DROP TABLE IF EXISTS pg_numeric_probe")
        .await
        .expect("drop probe");
    db.execute("CREATE TABLE pg_numeric_probe (id INT PRIMARY KEY, v NUMERIC)")
        .await
        .expect("create probe");

    let cases = [
        (
            CompactDecimal {
                mantissa: 10_000,
                scale: 0,
            },
            "10000",
        ),
        (
            CompactDecimal {
                mantissa: 10_000_000_000,
                scale: 0,
            },
            "10000000000",
        ),
        (
            CompactDecimal {
                mantissa: 12_345,
                scale: 2,
            },
            "123.45",
        ),
        (
            CompactDecimal {
                mantissa: -12_345,
                scale: 2,
            },
            "-123.45",
        ),
        (
            CompactDecimal {
                mantissa: 1_234,
                scale: 8,
            },
            "0.00001234",
        ),
        (
            CompactDecimal {
                mantissa: 1,
                scale: 2,
            },
            "0.01",
        ),
        (
            CompactDecimal {
                mantissa: 0,
                scale: 0,
            },
            "0",
        ),
    ];

    for (i, (dec, expected)) in cases.iter().enumerate() {
        let id = i as i32;
        // 写入 → 服务端文本
        db.execute_with_params(
            "INSERT INTO pg_numeric_probe (id, v) VALUES (?, ?)",
            vec![Value::I32(id), Value::Decimal(*dec)],
        )
        .await
        .expect("insert decimal");
        let rendered: String = db
            .scalar_with_params(
                "SELECT v::text FROM pg_numeric_probe WHERE id = ?",
                vec![Value::I32(id)],
            )
            .await
            .expect("server text");
        assert_eq!(rendered, *expected, "写入 {:?} 库内应显示 {expected}", dec);

        // 服务端字面量 → 驱动读回（覆盖「尾零组被规范化」的读取路径）。
        // 注意：`?` 绑定到 NUMERIC 目标列要求二进制 numeric 载荷，文本参数会被拒绝，
        // 故这里用内联字面量构造服务端值。
        let back: rust_decimal::Decimal = db
            .scalar(&format!("SELECT '{expected}'::numeric"))
            .await
            .expect("literal readback");
        assert_eq!(
            back,
            rust_decimal::Decimal::from_i128_with_scale(dec.mantissa, dec.scale),
            "服务端字面量 {expected} 读回不一致"
        );
    }

    let _ = db.execute("DROP TABLE pg_numeric_probe").await;
}

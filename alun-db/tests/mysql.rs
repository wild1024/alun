//! MySQL 端到端集成测试。
//!
//! 依赖 docker compose 提供的实例（mysql:8.4）：
//!
//! ```text
//! docker compose up -d
//! cargo test -p alun-db --features mysql,chrono,uuid,rust_decimal,json --test mysql
//! ```
//!
//! 连接串可通过 `ALUN_TEST_MYSQL_URL` 覆盖；连接失败时打印原因并跳过
//! （本地无 docker 环境不红测试）。

#![cfg(feature = "mysql")]

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
const DEFAULT_MYSQL_URL: &str = "mysql://alun:alun@localhost:3306/alun_test";

/// 连接探测上限：服务器接受 TCP 却不应答时（端口代理指向未就绪容器）不能无限阻塞，
/// 否则单个用例会挂住整个 `cargo test`。
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 不可用原因缓存：探测失败后同一测试二进制内的后续用例直接跳过，不再逐个等超时。
static MYSQL_DOWN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// 连接测试库。失败/超时返回 `None`（跳过测试）。
async fn setup_mysql() -> Option<Db> {
    if let Some(msg) = MYSQL_DOWN.get() {
        eprintln!("跳过 MySQL 测试（{msg}）");
        return None;
    }
    let url =
        std::env::var("ALUN_TEST_MYSQL_URL").unwrap_or_else(|_| DEFAULT_MYSQL_URL.to_string());
    let msg = match tokio::time::timeout(PROBE_TIMEOUT, Db::connect(&url)).await {
        Ok(Ok(db)) => return Some(db),
        Ok(Err(e)) => format!("无法连接 {url}：{e}"),
        Err(_) => format!("连接 {url} 超时（>{}s）", PROBE_TIMEOUT.as_secs()),
    };
    let _ = MYSQL_DOWN.set(msg.clone());
    eprintln!("跳过 MySQL 测试（{msg}）");
    None
}

/// 建表：覆盖常用类型（自增主键 / 唯一约束 / JSON / DATETIME(6) /
/// DECIMAL / CHECK 约束）。
///
/// 仅首个测试执行的调用执行一次 `DROP + CREATE` 并写入 `SCHEMA_READY`；
/// 其他并发测试等待栅栏放行后直接复用 schema，不再重复建/删表。
async fn create_tables(db: &Db) {
    SCHEMA_READY
        .get_or_init(|| async {
            // 清理上次残留，保证首次建表干净
            let _ = db.execute("DROP TABLE IF EXISTS my_users").await;
            let _ = db.execute("DROP TABLE IF EXISTS my_products").await;
            db.execute(
                "CREATE TABLE IF NOT EXISTS my_users (
                    id BIGINT AUTO_INCREMENT PRIMARY KEY,
                    email VARCHAR(255) NOT NULL UNIQUE,
                    name VARCHAR(255) NOT NULL,
                    status INT NOT NULL DEFAULT 1,
                    score DECIMAL(10, 3),
                    profile JSON,
                    created_at DATETIME(6) NOT NULL,
                    CONSTRAINT chk_status CHECK (status >= 0)
                )",
            )
            .await
            .expect("create my_users");
            db.execute(
                "CREATE TABLE IF NOT EXISTS my_products (
                    sku VARCHAR(64) PRIMARY KEY,
                    price DECIMAL(10, 2) NOT NULL
                )",
            )
            .await
            .expect("create my_products");
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
#[table("my_users")]
struct MyUser {
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

/// 基础 CRUD + `LAST_INSERT_ID` 主键回填。
#[tokio::test]
async fn crud_with_last_insert_id() {
    let Some(db) = setup_mysql().await else {
        return;
    };
    create_tables(&db).await;

    let mut user = MyUser {
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
    // MySQL 路径：LAST_INSERT_ID 取回自增主键
    assert!(user.id > 0, "LAST_INSERT_ID should populate auto id");

    user.name = "Alice Updated".into();
    user.update(&db).await.expect("update");

    let fetched = MyUser::find(&db, user.id).await.expect("find");
    assert_eq!(fetched.name, "Alice Updated");
    assert_eq!(
        fetched.score.expect("score"),
        rust_decimal::Decimal::new(12345, 2)
    );
    assert_eq!(fetched.profile.expect("profile").bio, "hello");

    let affected = user.delete(&db).await.expect("delete");
    assert_eq!(affected, 1);
}

/// `?` 占位符手写路径（MySQL 原生 `?`，无需改写）。
#[tokio::test]
async fn raw_question_placeholder() {
    let Some(db) = setup_mysql().await else {
        return;
    };
    create_tables(&db).await;

    db.execute_with_params(
        "INSERT INTO my_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
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
            "SELECT id, name FROM my_users WHERE status = ? AND name LIKE ?",
            vec![2i32.into(), "B%".into()],
        )
        .await
        .expect("query");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get_typed::<String>("name").expect("name"), "Bob");
}

/// 类型往返：DECIMAL / JSON / DATETIME(6) 微秒精度。
#[tokio::test]
async fn type_roundtrip() {
    let Some(db) = setup_mysql().await else {
        return;
    };
    create_tables(&db).await;

    let now = Utc::now();
    db.execute_with_params(
        "INSERT INTO my_users (email, name, status, score, profile, created_at)
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
            "SELECT score, profile, created_at FROM my_users WHERE email = ?",
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
    // 微秒精度往返（DATETIME(6)）
    assert_eq!(ts.timestamp_micros(), now.timestamp_micros());
}

/// 唯一约束冲突映射为 `DbError::UniqueViolation`。
#[tokio::test]
async fn unique_violation_mapping() {
    let Some(db) = setup_mysql().await else {
        return;
    };
    create_tables(&db).await;

    db.execute_with_params(
        "INSERT INTO my_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
        user_params("dave@example.com", "Dave"),
    )
    .await
    .expect("first insert succeeds");

    let result = db
        .execute_with_params(
            "INSERT INTO my_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
            user_params("dave@example.com", "Duplicate"),
        )
        .await;
    match result {
        Err(DbError::UniqueViolation { .. }) => {}
        other => panic!("expected UniqueViolation, got {other:?}"),
    }
}

/// CHECK 约束冲突（MySQL 8.0.16+ 错误码 3819）映射为 `DbError::CheckViolation`。
#[tokio::test]
async fn check_violation_mapping() {
    let Some(db) = setup_mysql().await else {
        return;
    };
    create_tables(&db).await;

    let mut params = user_params("evil@example.com", "Evil");
    // status 列有 CHECK (status >= 0)，负值触发 3819
    params[2] = Value::I32(-1);
    let result = db
        .execute_with_params(
            "INSERT INTO my_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
            params,
        )
        .await;
    match result {
        Err(DbError::CheckViolation { .. }) => {}
        other => panic!("expected CheckViolation, got {other:?}"),
    }
}

/// 事务三态：提交持久化 / 回滚丢弃 / Drop 丢弃连接回滚。
#[tokio::test]
async fn transaction_semantics() {
    let Some(db) = setup_mysql().await else {
        return;
    };
    create_tables(&db).await;

    // 1. 闭包式提交
    db.transaction(|tx| {
        Box::pin(async move {
            tx.execute_with_params(
                "INSERT INTO my_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
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
            "SELECT COUNT(*) FROM my_users WHERE email = ?",
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
                    "INSERT INTO my_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
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
            "SELECT COUNT(*) FROM my_users WHERE email = ?",
            vec![Value::Text("frank@example.com".into())],
        )
        .await
        .expect("count");
    assert_eq!(count, 0);

    // 3. Drop 未终结事务：连接丢弃触发服务端回滚
    {
        let tx = db.begin().await.expect("begin");
        tx.execute_with_params(
            "INSERT INTO my_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
            user_params("grace@example.com", "Grace"),
        )
        .await
        .expect("insert in tx");
        // tx 在此 Drop，未提交 → 服务端断连自动回滚
    }
    let count: i64 = db
        .scalar_with_params(
            "SELECT COUNT(*) FROM my_users WHERE email = ?",
            vec![Value::Text("grace@example.com".into())],
        )
        .await
        .expect("count");
    assert_eq!(count, 0, "dropped transaction should roll back");
}

/// DECIMAL 值读写（自然键表）。
#[tokio::test]
async fn decimal_read_write() {
    let Some(db) = setup_mysql().await else {
        return;
    };
    create_tables(&db).await;

    db.execute_with_params(
        "INSERT INTO my_products (sku, price) VALUES (?, ?)",
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
            "UPDATE my_products SET price = ? WHERE sku = ?",
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
            "SELECT price FROM my_products WHERE sku = ?",
            vec![Value::Text("SKU-1".into())],
        )
        .await
        .expect("price");
    assert_eq!(price, rust_decimal::Decimal::new(2499, 2));
}

/// 池并发：多任务并行借出与归还（Clone 共享同一连接池）。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pool_concurrency() {
    let Some(db) = setup_mysql().await else {
        return;
    };
    create_tables(&db).await;

    let mut handles = Vec::new();
    for i in 0..8 {
        let db = db.clone();
        handles.push(tokio::spawn(async move {
            db.execute_with_params(
                "INSERT INTO my_users (email, name, status, created_at) VALUES (?, ?, ?, ?)",
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
        .scalar("SELECT COUNT(*) FROM my_users WHERE email LIKE 'pool%@example.com'")
        .await
        .expect("count");
    assert_eq!(count, 8);
}

//! alun-db 全功能使用示例。
//!
//! 运行：
//! ```bash
//! cargo run --example all_features --features chrono,json
//! ```
//!
//! 覆盖能力：
//! - `#[derive(Model)]` 自增主键 / 唯一字段 / 嵌套 JSON / 时间字段
//! - `#[derive(Row)]` 嵌套结构（JSON 列）
//! - `#[derive(TypeMapper)]` + `#[value_map]` 字符串枚举
//! - 持久化：`save`（自动 INSERT/UPDATE + 主键回填）/ `update` / `delete`
//! - 查询：`find` / `find_or_none` / `find_by` / `all` / `count` / `paginate`
//! - 事务：`db.transaction(|tx| ...)`，闭包 Ok 提交 / Err 回滚
//! - 原生 Row 模式：`db.find` + `Row::get_typed`
//! - 全局单例：`alun_db::init` + `db()`

use alun_db::prelude::*;
use chrono::Utc;

// ==================== 模型定义 ====================

/// 嵌套 JSON 子结构：用 `#[derive(Row)]`，可被序列化进 TEXT/JSON 列。
#[derive(Row, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub avatar: Option<String>,
    pub bio: String,
}

/// 主模型：用户表。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("users")]
pub struct User {
    #[key]
    #[auto]
    pub id: i64,
    #[unique]
    pub email: String,
    pub name: String,
    pub status: i32,
    #[json]
    pub profile: Option<Profile>,
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
}

/// 订单状态枚举：自定义字符串编码。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq)]
#[value_map("P" => Pending, "C" => Confirmed, "S" => Shipped)]
pub enum OrderStatus {
    Pending,
    Confirmed,
    Shipped,
}

/// 订单模型：演示枚举字段 + 外键。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("orders")]
pub struct Order {
    #[key]
    #[auto]
    pub id: i64,
    pub user_id: i64,
    pub amount: String,
    pub status: OrderStatus,
    pub created_at: chrono::DateTime<Utc>,
}

// ==================== 建表 ====================

async fn create_schema(db: &Db) {
    db.execute(
        "CREATE TABLE users (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            email      TEXT NOT NULL UNIQUE,
            name       TEXT NOT NULL,
            status     INTEGER NOT NULL DEFAULT 1,
            profile    TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .await
    .unwrap();

    db.execute(
        "CREATE TABLE orders (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id    INTEGER NOT NULL,
            amount     TEXT NOT NULL,
            status     TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .await
    .unwrap();
}

fn new_user(email: &str, name: &str, profile: Option<Profile>) -> User {
    let now = Utc::now();
    User {
        id: 0, // 自增主键：0 表示新行，save 后回填
        email: email.into(),
        name: name.into(),
        status: 1,
        profile,
        created_at: now,
        updated_at: now,
    }
}

// ==================== 主流程 ====================

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // 直接用内存库演示（也可以 alun_db::init("sqlite://./dev.db").await?; 然后用 db()）
    let db = Db::connect("sqlite::memory:").await?;
    create_schema(&db).await;

    // ---------- 1. INSERT（save 自动识别新行 + 回填主键） ----------
    let mut alice = new_user(
        "alice@example.com",
        "Alice",
        Some(Profile {
            avatar: Some("https://cdn.example.com/a.png".into()),
            bio: "Rust 爱好者".into(),
        }),
    );
    alice.insert_or_update(&db).await?;
    println!("[1] INSERT Alice -> id = {} (主键已回填)", alice.id);

    let mut bob = new_user("bob@example.com", "Bob", None);
    bob.insert_or_update(&db).await?;
    println!("[1] INSERT Bob   -> id = {}", bob.id);

    // ---------- 2. find / find_or_none ----------
    let fetched = User::find(&db, alice.id).await?;
    println!(
        "[2] find: {} <{}>, profile.bio = {}",
        fetched.name,
        fetched.email,
        fetched.profile.unwrap().bio
    );

    let missing = User::find_or_none(&db, 99999i64).await?;
    println!("[2] find_or_none(99999) = {:?}", missing.is_none());

    // ---------- 3. find_by（参数元组，支持 &str / i32） ----------
    let rows = User::find_by(&db, "email = ? AND status = ?", ("bob@example.com", 1i32)).await?;
    println!("[3] find_by -> {} row(s)", rows.len());

    // ---------- 4. all / count ----------
    let all = User::query().find(&db).await?;
    let total = User::count(&db).await?;
    println!("[4] all users = {}, count = {}", all.len(), total);

    // ---------- 5. paginate ----------
    for i in 0..8 {
        let mut u = new_user(&format!("u{}@x.com", i), &format!("User {}", i), None);
        u.insert_or_update(&db).await?;
    }
    let page = User::query().paginate(&db, 2, 3).await?;
    println!(
        "[5] paginate page=2 size=3 -> total={}, page={}, list={}",
        page.total,
        page.page,
        page.list.len()
    );

    // ---------- 6. UPDATE（save 识别已存在主键 -> 走 UPDATE） ----------
    alice.name = "Alice Wang".into();
    alice.status = 2;
    alice.insert_or_update(&db).await?; // 主键已存在，内部走 UPDATE
    let after = User::find(&db, alice.id).await?;
    println!(
        "[6] UPDATE Alice -> name={}, status={}",
        after.name, after.status
    );

    // ---------- 7. 枚举字段 OrderStatus ----------
    let mut order = Order {
        id: 0,
        user_id: alice.id,
        amount: "99.50".into(),
        status: OrderStatus::Pending,
        created_at: Utc::now(),
    };
    order.insert_or_update(&db).await?;
    println!(
        "[7] INSERT order id={}, status={:?}",
        order.id, order.status
    );

    // 从库里读回，验证 value_map 字符串编码 roundtrip
    let order_back = Order::find(&db, order.id).await?;
    println!(
        "[7] order from db: amount={}, status={:?}",
        order_back.amount, order_back.status
    );

    // ---------- 8. 事务：提交 ----------
    db.transaction(|tx| async move {
        // 事务内语句走 tx 句柄（在事务专属连接上执行）
        tx.execute_with_params(
            "UPDATE users SET status = 9 WHERE id = ?",
            vec![alun_db::prelude::Value::I64(bob.id)],
        )
        .await?;
        Ok(())
    })
    .await?;
    let bob_after_tx = User::find(&db, bob.id).await?;
    println!(
        "[8] 事务提交后 Bob.status = {} (期望 9)",
        bob_after_tx.status
    );

    // ---------- 9. 事务：回滚（闭包返回 Err） ----------
    let rollback_result = db
        .transaction(|tx| async move {
            tx.execute_with_params(
                "UPDATE users SET name = 'HACKED' WHERE id = ?",
                vec![alun_db::prelude::Value::I64(alice.id)],
            )
            .await?;
            // 故意返回 Err 触发回滚
            Err::<(), _>(alun_db_core::error::DbError::InvalidValue("boom".into()))
        })
        .await;
    println!("[9] 事务返回 Err = {}", rollback_result.is_err());
    let alice_after_rollback = User::find(&db, alice.id).await?;
    println!(
        "[9] 回滚后 Alice.name = {} (未被改成 HACKED)",
        alice_after_rollback.name
    );

    // ---------- 10. 原生 Row 模式（不经过 Model） ----------
    let rows = db
        .find("SELECT id, email, status FROM users ORDER BY id LIMIT 3")
        .await?;
    println!("[10] 原生 Row 查询 {} 行：", rows.len());
    for r in &rows {
        let id: i64 = r.get_typed("id")?;
        let email: String = r.get_typed("email")?;
        let status: i64 = r.get_typed("status")?;
        println!("      - id={} email={} status={}", id, email, status);
    }

    // ---------- 11. DELETE ----------
    let deleted = order.delete(&db).await?;
    println!("[11] DELETE order -> {} row affected", deleted);
    assert!(Order::find_or_none(&db, order.id).await?.is_none());

    println!("\n全部示例执行完毕 ✓");
    Ok(())
}

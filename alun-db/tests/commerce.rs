//! 电商场景集成测试（小型项目：用户 / 商品 / 订单）。
//!
//! 从调用方视角以三个业务域覆盖 `/alun-db` 对外暴露的全部 API：
//!
//! - 强类型 CRUD：`CrudExt` / `PersistExt`（insert_or_update / update / ChangeSet /
//!   delete / upsert / insert_many / find_* / paginate / count / delete_*）
//! - 动态行操作：`Db::insert_row` / `update_row` / `delete_row` / `find_row_by_pk`
//! - 事务：`transaction` 闭包式与 `begin` / `commit` / `rollback` 显式式
//! - 迁移：`migrate` / `migrate_down` / `execute_script`
//! - Builder：FindBuilder / PageBuilder / CountBuilder，含 keyset 游标分页
//! - `sql!` 宏链式 API：find / find_one / find_first / execute
//! - 软删除：`#[soft_delete]` 查询过滤 + 删除改写为标记
//! - 主键/唯一列重命名：`#[column(...)]` 在 insert_or_update / update / ChangeSet / upsert 全链路生效

mod common;

use alun_db::prelude::*;
// 排序枚举 `Order` 现由 facade prelude 导出；本文件存在同名领域模型 `Order`，
// 故以别名引用排序枚举，规避同名冲突。
use alun_db::prelude::Order as SortDir;
use chrono::{DateTime, Utc};

// ==================== 领域模型 ====================

/// 用户。含软删除标记列。
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
    /// 软删除标记列。
    #[soft_delete]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// 商品的支付状态枚举（TypeMapper 字符串映射）。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
#[value_map("on_sale" => OnSale, "off_shelf" => OffShelf)]
pub enum ShelfStatus {
    OnSale,
    OffShelf,
}

/// 商品。主键列重命名为 `item_id`，验证 `#[column(...)]` 全链路。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("products")]
#[hook(TimestampHook)]
pub struct Product {
    #[key]
    #[auto]
    #[column("item_id")]
    pub id: i64,
    #[unique]
    pub sku: String,
    pub name: String,
    pub price_cents: i64,
    pub stock: i64,
    pub status: ShelfStatus,
}

/// 订单状态枚举（TypeMapper 字符串映射）。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
#[value_map("pending" => Pending, "paid" => Paid, "cancelled" => Cancelled)]
pub enum OrderStatus {
    Pending,
    Paid,
    Cancelled,
}

/// 订单。外键引用 users.id 与 products.item_id。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("orders")]
pub struct Order {
    #[key]
    #[auto]
    #[column("order_id")]
    pub id: i64,
    pub user_id: i64,
    pub item_id: i64,
    pub qty: i64,
    pub total_cents: i64,
    pub status: OrderStatus,
    pub created_at: DateTime<Utc>,
}

/// 建表 SQL（含外键与唯一约束）。
async fn create_schema(db: &Db) {
    db.execute_script(
        "CREATE TABLE users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            email TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            status INTEGER NOT NULL DEFAULT 1,
            deleted_at TEXT
        );
        CREATE TABLE products (
            item_id INTEGER PRIMARY KEY AUTOINCREMENT,
            sku TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            price_cents INTEGER NOT NULL,
            stock INTEGER NOT NULL,
            status TEXT NOT NULL
        );
        CREATE TABLE orders (
            order_id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL REFERENCES users(id),
            item_id INTEGER NOT NULL REFERENCES products(item_id),
            qty INTEGER NOT NULL,
            total_cents INTEGER NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL
        );",
    )
    .await
    .expect("create schema");
}

fn new_user(email: &str, name: &str) -> User {
    User {
        id: 0,
        email: email.into(),
        name: name.into(),
        status: 1,
        deleted_at: None,
    }
}

fn new_product(sku: &str, name: &str, price: i64, stock: i64) -> Product {
    Product {
        id: 0,
        sku: sku.into(),
        name: name.into(),
        price_cents: price,
        stock,
        status: ShelfStatus::OnSale,
    }
}

// ==================== 基础 CRUD（用户域） ====================

#[tokio::test]
async fn user_full_crud_roundtrip() {
    let db = common::setup_db().await;
    create_schema(&db).await;

    // save：INSERT + 自增主键回填 + TimestampHook 填充 created_at
    let mut u = new_user("alice@example.com", "Alice");
    u.insert_or_update(&db).await.expect("save");
    assert!(u.id > 0, "auto id must be populated");
    assert!(u.email == "alice@example.com");

    // find / find_or_none
    let found = User::find(&db, u.id).await.expect("find");
    assert_eq!(found.name, "Alice");
    assert!(
        User::find_or_none(&db, u.id)
            .await
            .expect("find_or_none")
            .is_some()
    );
    assert!(
        User::find_or_none(&db, 999999)
            .await
            .expect("find_or_none missing")
            .is_none()
    );

    // update：全字段更新
    let mut upd = found;
    upd.name = "Alice Updated".into();
    upd.update(&db).await.expect("update");
    assert_eq!(User::find(&db, u.id).await.unwrap().name, "Alice Updated");

    // 变更集：部分更新只写指定列
    upd.status = 9;
    let affected = User::of(upd.id)
        .set(User::status(), upd.status)
        .update(&db)
        .await
        .expect("update");
    assert_eq!(affected, 1);
    let after = User::find(&db, u.id).await.unwrap();
    assert_eq!(after.status, 9);
    assert_eq!(
        after.name, "Alice Updated",
        "unlisted field must be unchanged"
    );

    // count / count_by（Alice 的 status 已被改 = 9）
    let mut u2 = new_user("bob@example.com", "Bob");
    u2.status = 0;
    u2.insert_or_update(&db).await.expect("save bob");
    assert_eq!(User::count(&db).await.expect("count"), 2);
    assert_eq!(
        User::count_by(&db, "status = ?", (9,))
            .await
            .expect("count_by"),
        1
    );

    // all / find_by / find_first_by
    assert_eq!(User::query().find(&db).await.expect("all").len(), 2);
    assert_eq!(
        User::find_by(&db, "email = ?", ("bob@example.com",))
            .await
            .expect("find_by")
            .len(),
        1
    );
    let first = User::find_first_by(&db, "email = ?", ("alice@example.com",))
        .await
        .expect("find_first_by");
    assert_eq!(first.unwrap().name, "Alice Updated");

    // paginate
    let page = User::query().paginate(&db, 1, 1).await.expect("paginate");
    assert_eq!(page.total, 2);
    assert_eq!(page.list.len(), 1);

    // delete / delete（软删除，库存数据不硬删）
    let affected = User::delete_by_pk(&db, u.id).await.expect("delete");
    assert_eq!(affected, 1);
    assert!(
        User::find_or_none(&db, u.id)
            .await
            .expect("after soft delete")
            .is_none(),
        "soft-deleted user must be filtered from reads"
    );
    // 未删除用户仍可见
    assert_eq!(
        User::find_or_none(&db, u2.id).await.unwrap().unwrap().name,
        "Bob"
    );

    // delete_by
    let affected = User::delete_by(&db, "name = ?", ("Bob",))
        .await
        .expect("delete_by");
    assert_eq!(affected, 1);
    assert_eq!(User::count(&db).await.expect("count after delete_by"), 0);
}

// ==================== 商品域：主键重命名 + upsert + 批量 ====================

#[tokio::test]
async fn product_upsert_batch_and_column_rename() {
    let db = common::setup_db().await;
    create_schema(&db).await;

    // 主键列名约定：字段名 id <-> 列名 item_id
    assert_eq!(Product::PRIMARY_KEY, "id");
    assert_eq!(Product::primary_key_column(), "item_id");

    let mut p = new_product("SKU-001", "Keyboard", 12900, 50);
    p.insert_or_update(&db).await.expect("save product");
    assert!(p.id > 0);
    assert_eq!(Product::find(&db, p.id).await.unwrap().name, "Keyboard");

    // update：SET/WHERE 使用真实列名 item_id
    let mut upd = p.clone();
    upd.price_cents = 13900;
    upd.update(&db).await.expect("product update");
    assert_eq!(Product::find(&db, p.id).await.unwrap().price_cents, 13900);

    // 变更集：主键列被重命名（item_id）的模型同样只写指定列
    Product::of(p.id)
        .set(Product::stock(), 60)
        .update(&db)
        .await
        .expect("partial update");
    assert_eq!(Product::find(&db, p.id).await.unwrap().stock, 60);

    // upsert：以 sku 为冲突键（字段名，内部映射到 sku 列）
    // 自增模型 INSERT 路径会经 RETURNING 回填主键到模型
    let mut inserted = new_product("SKU-UPS", "Mouse", 5000, 10);
    inserted
        .upsert(&db, &["sku"], &["price_cents", "stock"])
        .await
        .expect("upsert insert");
    assert!(inserted.id > 0, "upsert must backfill the pk on INSERT");

    // 同 sku 再次 upsert → 冲突，更新 price_cents / stock；主键回填为已存在行的 id
    let mut conflict = new_product("SKU-UPS", "Mouse", 5500, 13);
    conflict
        .upsert(&db, &["sku"], &["price_cents", "stock"])
        .await
        .expect("upsert conflict");
    assert_eq!(
        conflict.id, inserted.id,
        "conflict upsert must backfill the existing row's pk"
    );
    assert_eq!(
        db.scalar::<i64>("SELECT COUNT(*) FROM products")
            .await
            .expect("count products"),
        2,
        "conflict upsert must not insert a new row"
    );
    let merged = Product::find_first_by(&db, "sku = ?", ("SKU-UPS",))
        .await
        .expect("find upserted")
        .expect("upserted row exists");
    assert_eq!(merged.price_cents, 5500);
    assert_eq!(merged.stock, 13);

    // 批量插入：自增主键由数据库生成，不回填
    Product::insert_many(
        &[
            new_product("SKU-B1", "Pad1", 100, 1),
            new_product("SKU-B2", "Pad2", 200, 2),
            new_product("SKU-B3", "Pad3", 300, 3),
        ],
        &db,
    )
    .await
    .expect("insert_many");
    assert_eq!(
        db.scalar::<i64>("SELECT COUNT(*) FROM products")
            .await
            .expect("count after batch"),
        5
    );
    // 批量插入字段不错位（按 SKU 反查）
    assert_eq!(
        Product::find_first_by(&db, "sku = ?", ("SKU-B3",))
            .await
            .unwrap()
            .unwrap()
            .stock,
        3
    );
}

// ==================== 订单域：关系查询 + Row 动态操作 ====================

#[tokio::test]
async fn order_with_row_dynamic_ops() {
    let db = common::setup_db().await;
    create_schema(&db).await;

    // 依赖用户与商品
    let mut user = new_user("carol@example.com", "Carol");
    user.insert_or_update(&db).await.expect("save user");
    let mut prod = new_product("SKU-O", "Screen", 89900, 5);
    prod.insert_or_update(&db).await.expect("save product");

    // 强类型保存订单
    let mut order = Order {
        id: 0,
        user_id: user.id,
        item_id: prod.id,
        qty: 2,
        total_cents: 179800,
        status: OrderStatus::Paid,
        created_at: Utc::now(),
    };
    order.insert_or_update(&db).await.expect("save order");
    let order_id = order.id;
    assert!(order_id > 0);

    // find_by 关联查询订单（派生 TypeMapper 枚举可直接作为 Into<Value> 参数）
    let orders = Order::find_by(
        &db,
        "user_id = ? AND status = ?",
        (user.id, OrderStatus::Paid),
    )
    .await
    .expect("order find_by");
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].total_cents, 179800);

    // count_by
    assert_eq!(
        Order::count_by(&db, "item_id = ?", (prod.id,))
            .await
            .expect("order count"),
        1
    );

    // 动态 Row 操作：insert_row（自增回填）
    let mut row = Row::new().with_entity("orders", "order_id");
    row.insert("user_id", user.id);
    row.insert("item_id", prod.id);
    row.insert("qty", 1i64);
    row.insert("total_cents", 0i64);
    row.insert("status", "pending");
    row.insert("created_at", "2026-01-01T00:00:00Z");
    let inserted = db.insert_row(&row).await.expect("insert_row");
    let dyn_id = inserted.get("order_id").cloned();
    assert!(
        dyn_id.is_some(),
        "auto pk must be backfilled for dynamic row"
    );

    // find_row_by_pk
    let found = db
        .find_row_by_pk("orders", "order_id", Value::I64(order_id))
        .await
        .expect("find_row_by_pk")
        .expect("row must exist");
    assert_eq!(found.table(), Some("orders"));
    assert_eq!(found.get("total_cents"), Some(&Value::I64(179800)));

    // update_row：更新非主键字段
    let mut upd = Row::new().with_entity("orders", "order_id");
    upd.insert("order_id", order_id);
    upd.insert("status", "cancelled");
    let affected = db.update_row(&upd).await.expect("update_row");
    assert_eq!(affected, 1);
    let after = Order::find(&db, order_id)
        .await
        .expect("find after update_row");
    assert_eq!(after.status, OrderStatus::Cancelled);

    // delete_row（用 insert_row 返回的动态行 id）
    let dyn_id: i64 = dyn_id
        .and_then(|v| v.into_i64())
        .expect("dynamic pk is integer");
    let mut del = Row::new().with_entity("orders", "order_id");
    del.insert("order_id", Value::I64(dyn_id));
    assert_eq!(db.delete_row(&del).await.expect("delete_row"), 1);
    assert_eq!(Order::count(&db).await.expect("order count after del"), 1);
}

/// 便捷：把 Value 转 i64（测试辅助）。
trait IntoI64 {
    fn into_i64(self) -> Option<i64>;
}
impl IntoI64 for Value {
    fn into_i64(self) -> Option<i64> {
        match self {
            Value::I64(v) => Some(v),
            Value::I32(v) => Some(v as i64),
            Value::U64(v) => Some(v as i64),
            Value::I8(v) => Some(v as i64),
            _ => None,
        }
    }
}

// ==================== 事务 ====================

#[tokio::test]
async fn transaction_closure_commit_rollback() {
    let db = common::setup_db().await;
    create_schema(&db).await;

    // 闭包式：Ok → 提交
    db.transaction(|tx| async move {
        // 事务内通过 tx 句柄执行写操作
        sql!(
            "INSERT INTO users (email, name, status, deleted_at) VALUES (",
            { "tx@example.com" },
            ", ",
            { "TXUser" },
            ", 1, NULL)"
        )
        .execute(&tx)
        .await?;
        Ok(())
    })
    .await
    .expect("commit txn");

    assert_eq!(
        User::count_by(&db, "email = ?", ("tx@example.com",))
            .await
            .expect("count"),
        1,
        "committed user must be persisted"
    );

    // 闭包式：Err → 回滚
    let result: std::result::Result<(), alun_db_core::error::DbError> = db
        .transaction(|tx| async move {
            sql!(
                "INSERT INTO users (email, name, status, deleted_at) VALUES (",
                { "rollback@example.com" },
                ", 1, 1, NULL)"
            )
            .execute(&tx)
            .await?;
            Err(alun_db_core::error::DbError::NotFound)
        })
        .await;
    assert!(result.is_err(), "Err must abort transaction");
    assert_eq!(
        User::count_by(&db, "email = ?", ("rollback@example.com",))
            .await
            .expect("count"),
        0,
        "rolled-back insert must not persist"
    );
}

#[tokio::test]
async fn explicit_transaction_commit_rollback() {
    let db = common::setup_db().await;
    create_schema(&db).await;

    // begin / execute / commit
    let tx = db.begin().await.expect("begin");
    tx.execute_with_params(
        "INSERT INTO users (email, name, status, deleted_at) VALUES (?, ?, 1, NULL)",
        vec![
            Value::Text("explicit@example.com".into()),
            Value::Text("Explicit".into()),
        ],
    )
    .await
    .expect("tx execute");
    tx.commit().await.expect("commit");
    assert_eq!(
        User::count_by(&db, "email = ?", ("explicit@example.com",))
            .await
            .unwrap(),
        1
    );

    // begin / 写后 rollback
    let tx2 = db.begin().await.expect("begin2");
    tx2.execute(
        "INSERT INTO users (email, name, status, deleted_at) VALUES ('rb@example.com', 'RB', 1, NULL)",
    )
    .await
    .expect("tx2 execute");
    tx2.rollback().await.expect("rollback");
    assert_eq!(
        User::count_by(&db, "email = ?", ("rb@example.com",))
            .await
            .unwrap(),
        0
    );

    // 事务终结后再次操作应报 Transaction 错误
    let tx3 = db.begin().await.expect("begin3");
    tx3.commit().await.expect("commit");
    let err = tx3.execute("SELECT 1").await;
    assert!(matches!(
        err,
        Err(alun_db_core::error::DbError::Transaction(_))
    ));
}

// ==================== 迁移 ====================

#[tokio::test]
async fn migrate_and_rollback() {
    let db = common::setup_db().await;
    // 用迁移建表，验证 run / rollback
    let migrations = vec![
        Migration::new(
            1,
            "create_users",
            "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, email TEXT NOT NULL UNIQUE, name TEXT NOT NULL, status INTEGER NOT NULL DEFAULT 1, deleted_at TEXT)",
            "DROP TABLE users",
        ),
        Migration::new(
            2,
            "create_products",
            "CREATE TABLE products (item_id INTEGER PRIMARY KEY AUTOINCREMENT, sku TEXT NOT NULL UNIQUE, name TEXT NOT NULL, price_cents INTEGER NOT NULL, stock INTEGER NOT NULL, status TEXT NOT NULL)",
            "DROP TABLE products",
        ),
    ];

    let applied = Migrator::from_migrations(migrations.clone())
        .run(&db)
        .await
        .expect("migrate");
    assert_eq!(applied, 2);
    // 幂等：再次迁移不重复应用
    assert_eq!(
        Migrator::from_migrations(migrations.clone())
            .run(&db)
            .await
            .expect("migrate again"),
        0
    );

    // 回滚最近一条（LIFO）
    let rolled = Migrator::from_migrations(migrations.clone())
        .rollback(&db)
        .await
        .expect("rollback");
    assert_eq!(rolled.map(|v| v.to_string()), Some("2".to_string()));
    // 再回滚
    let rolled2 = Migrator::from_migrations(migrations.clone())
        .rollback(&db)
        .await
        .expect("rollback2");
    assert_eq!(rolled2.map(|v| v.to_string()), Some("1".to_string()));
    // 无可回滚
    assert!(
        Migrator::from_migrations(migrations)
            .rollback(&db)
            .await
            .expect("rollback empty")
            .is_none()
    );
}

// ==================== Builder（Server 域查询/表达式） ====================

#[tokio::test]
async fn builders_and_expression_queries() {
    let db = common::setup_db().await;
    create_schema(&db).await;

    // 造数据
    for (i, email) in ["a@x.com", "b@x.com", "c@x.com"].iter().enumerate() {
        let mut u = new_user(email, &format!("User{i}"));
        u.insert_or_update(&db).await.expect("seed user");
    }

    // FindBuilder：filter + exec
    let rows = db
        .query()
        .table("users")
        .filter(Expr::Binary {
            left: Box::new(Expr::Column {
                table: "users",
                column: "status",
            }),
            op: BinaryOp::Ge,
            right: Box::new(Expr::Value(Value::I32(1))),
        })
        .order_by("id", SortDir::Desc)
        .limit(2)
        .exec(&db)
        .await
        .expect("find builder");
    assert_eq!(rows.len(), 2);

    // FindBuilder::first → LIMIT 1 + 条件
    let some = db
        .query()
        .table("users")
        .filter(Expr::Binary {
            left: Box::new(Expr::Column {
                table: "users",
                column: "email",
            }),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Value(Value::Text("b@x.com".into()))),
        })
        .first(&db)
        .await
        .expect("find builder first");
    assert!(some.is_some());

    // PageBuilder
    let page = db
        .page(1, 2)
        .table("users")
        .order_by("id", SortDir::Asc)
        .exec(&db)
        .await
        .expect("page builder");
    assert_eq!(page.total, 3);
    assert_eq!(page.list.len(), 2);

    // CountBuilder
    let cnt = db
        .count()
        .table("users")
        .filter(Expr::Binary {
            left: Box::new(Expr::Column {
                table: "users",
                column: "status",
            }),
            op: BinaryOp::Lt,
            right: Box::new(Expr::Value(Value::I32(2))),
        })
        .exec(&db)
        .await
        .expect("count builder");
    assert_eq!(cnt, 3);

    // keyset 游标分页：先取第一页尾部构造 Cursor
    let rows = db
        .query()
        .table("users")
        .order_by("id", SortDir::Asc)
        .limit(2)
        .exec(&db)
        .await
        .expect("first page for cursor");
    let last = rows.last().expect("last row of first page");
    let cursor = Cursor {
        key: last.get("id").cloned().unwrap_or(Value::I64(0)),
        pk: last.get("id").cloned().unwrap_or(Value::I64(0)),
    };
    let next = db
        .query()
        .table("users")
        .page_after("id", "id", SortDir::Asc, Some(cursor))
        .order_by("id", SortDir::Asc)
        .exec(&db)
        .await
        .expect("keyset next page");
    assert_eq!(next.len(), 1, "remaining 1 row after cursor");

    // PageBuilder 也支持 keyset 分页（能力与 FindBuilder 对齐）
    let page2 = db
        .page(1, 2)
        .table("users")
        .order_by("id", SortDir::Asc)
        .page_after("id", "id", SortDir::Asc, None)
        .exec(&db)
        .await
        .expect("page builder keyset");
    assert_eq!(page2.list.len(), 2, "keyset first page size");
    assert_eq!(page2.total, 0, "keyset mode does not precompute total");
    // 基于首页末行继续 keyset 翻页
    let last2 = page2.list.last().expect("last of keyset page");
    let cursor2 = Cursor {
        key: last2.get("id").cloned().unwrap_or(Value::I64(0)),
        pk: last2.get("id").cloned().unwrap_or(Value::I64(0)),
    };
    let page3 = db
        .page(1, 2)
        .table("users")
        .order_by("id", SortDir::Asc)
        .page_after("id", "id", SortDir::Asc, Some(cursor2))
        .exec(&db)
        .await
        .expect("page builder keyset next");
    assert_eq!(page3.list.len(), 1, "remaining 1 row in keyset next page");
}

// ==================== sql! 宏 + 表达式函数 ====================

#[tokio::test]
async fn sql_macro_and_scalar_functions() {
    let db = common::setup_db().await;
    create_schema(&db).await;

    for (sku, name, price) in [("S1", "aaa", 100), ("S2", "bbb", 200), ("S3", "ccc", 300)] {
        let mut p = new_product(sku, name, price, 5);
        p.insert_or_update(&db).await.expect("seed product");
    }

    // find / find_one / find_first
    let rows = sql!("SELECT name FROM products WHERE price_cents >= ", { 200 })
        .find(&db)
        .await
        .expect("find");
    assert_eq!(rows.len(), 2);

    let one = sql!("SELECT name FROM products WHERE sku = ", { "S2" })
        .find_one(&db)
        .await
        .expect("find_one");
    assert_eq!(one.get("name"), Some(&Value::Text("bbb".into())));

    assert!(
        sql!("SELECT name FROM products WHERE sku = ", { "NOPE" })
            .find_first(&db)
            .await
            .expect("find_first")
            .is_none()
    );

    // execute 写操作
    let affected = sql!(
        "UPDATE products SET price_cents = ",
        { 250 },
        " WHERE sku = ",
        { "S2" }
    )
    .execute(&db)
    .await
    .expect("execute");
    assert_eq!(affected, 1);

    // scalar / scalar_with_params
    let sum: i64 = db
        .scalar("SELECT SUM(price_cents) FROM products")
        .await
        .expect("scalar sum");
    assert_eq!(sum, 650);
    let sales_price: i64 = db
        .scalar_with_params(
            "SELECT price_cents FROM products WHERE sku = ?",
            vec![Value::Text("S3".into())],
        )
        .await
        .expect("scalar_with_params");
    assert_eq!(sales_price, 300);

    // find / find_first / find_with_params / find_first_with_params
    assert_eq!(
        db.find("SELECT * FROM products").await.expect("find").len(),
        3
    );
    assert!(
        db.find_first("SELECT * FROM products")
            .await
            .expect("find_first")
            .is_some()
    );
    assert_eq!(
        db.find_with_params(
            "SELECT * FROM products WHERE sku = ?",
            vec![Value::Text("S1".into())]
        )
        .await
        .expect("find_with_params")
        .len(),
        1
    );
    assert!(
        db.find_first_with_params(
            "SELECT * FROM products WHERE sku = ?",
            vec![Value::Text("S1".into())]
        )
        .await
        .expect("find_first_with_params")
        .is_some()
    );

    // paginate_with_params / paginate
    let p = db
        .paginate_with_params(
            "SELECT * FROM products WHERE price_cents > ?",
            vec![Value::I64(100)],
            1,
            10,
        )
        .await
        .expect("paginate_with_params");
    assert_eq!(p.total, 2);
    assert!(
        db.paginate("SELECT * FROM products", 1, 10)
            .await
            .expect("paginate")
            .total
            == 3
    );

    // 类型化原始 SQL：ModelQuery 脊柱（原 `find_as_with_params`）
    let typed = Product::sql(sql!("SELECT * FROM products WHERE sku = ", { "S2" }))
        .find(&db)
        .await
        .expect("typed raw sql");
    assert_eq!(typed.len(), 1);
    assert_eq!(typed[0].price_cents, 250);
}

// ==================== 边界与容错 ====================

#[tokio::test]
async fn error_handling_and_boundaries() {
    let db = common::setup_db().await;
    create_schema(&db).await;

    // 唯一约束冲突应报错
    let mut u1 = new_user("dup@example.com", "D1");
    u1.insert_or_update(&db).await.expect("first save");
    let mut u2 = new_user("dup@example.com", "D2");
    assert!(
        u2.insert_or_update(&db).await.is_err(),
        "duplicate unique email must error"
    );

    // find 不存在 → NotFound
    assert!(matches!(
        User::find(&db, 999999).await,
        Err(alun_db_core::error::DbError::NotFound)
    ));

    // find_by 空结果 → 空向量（非错误）
    assert!(
        User::find_by(&db, "name = ?", ("Nobody",))
            .await
            .expect("empty find")
            .is_empty()
    );

    // 变更集未 set 任何列 → 报错
    let u = User {
        id: u1.id,
        email: "dup@example.com".into(),
        name: "D1".into(),
        status: 1,
        deleted_at: None,
    };
    assert!(matches!(
        User::of(u.id).update(&db).await,
        Err(alun_db_core::error::DbError::InvalidValue(_))
    ));

    // upsert 在含唯一键的场景重复调用不重复插入（已在商品域覆盖）
    // TypeMapper 字符串映射往返
    let mut p = new_product("SKU-S", "Status", 1, 1);
    p.insert_or_update(&db).await.expect("save for status");
    let fetched = Product::find(&db, p.id).await.expect("find status");
    assert_eq!(fetched.status, ShelfStatus::OnSale);
}

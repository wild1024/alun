//! Hook 生命周期测试。
//!
//! 覆盖：TimestampHook 时间戳自动填充、before_* 错误中断、
//! 洋葱模型执行顺序（before 正序 / after 逆序）、Hook 修改行数据。

mod common;
use alun_db::prelude::*;
use alun_db_core::error::{DbError, Result};
use alun_db_core::row::Row;
use alun_db_core::traits::Hook;
use chrono::{DateTime, Utc};
use common::*;

// ==================== 测试模型 ====================

/// 挂载内置 TimestampHook 的模型。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("hook_users")]
#[hook(TimestampHook)]
pub struct HookUser {
    #[key]
    #[auto]
    pub id: i64,
    pub email: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 挂载拒绝插入 Hook 的模型：验证 before_insert 返回 Err 中断保存。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("reject_users")]
#[hook(RejectInsertHook)]
pub struct RejectUser {
    #[key]
    #[auto]
    pub id: i64,
    pub email: String,
}

/// 挂载两个探针 Hook 的模型：验证执行顺序与行数据修改。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("probe_users")]
#[hook(OrderProbeHookA)]
#[hook(OrderProbeHookB)]
pub struct ProbeUser {
    #[key]
    #[auto]
    pub id: i64,
    pub email: String,
}

/// 挂载变更追踪探针的模型：验证变更集传给 Hook 的行只含变更列。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("changed_users")]
#[hook(ChangedColumnsHook)]
pub struct ChangedUser {
    #[key]
    #[auto]
    pub id: i64,
    pub email: String,
    pub status: i64,
}

// ==================== 测试 Hook ====================

/// 拒绝一切插入的 Hook。
struct RejectInsertHook;

impl Hook for RejectInsertHook {
    fn before_insert(&self, _row: &mut Row) -> Result<()> {
        Err(DbError::InvalidValue("insert rejected by hook".into()))
    }
}

/// 探针 A：before_insert 顺带把 email 改为大写，验证行数据可修改。
struct OrderProbeHookA;

/// 探针 B：仅记录事件。
struct OrderProbeHookB;

/// 执行顺序日志。事件形如 "A:before" / "B:after"。
static ORDER_LOG: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

/// 推送一条事件到顺序日志（锁中毒视为测试基础设施错误）。
fn push_log(event: &'static str) -> Result<()> {
    ORDER_LOG
        .lock()
        .map_err(|_| DbError::InvalidValue("order log poisoned".into()))?
        .push(event);
    Ok(())
}

impl Hook for OrderProbeHookA {
    fn before_insert(&self, row: &mut Row) -> Result<()> {
        if let Some(alun_db_core::stmt::Value::Text(email)) = row.get("email").cloned() {
            row.insert("email", email.to_uppercase());
        }
        push_log("A:before")
    }

    fn after_insert(&self, _row: &Row) -> Result<()> {
        push_log("A:after")
    }
}

impl Hook for OrderProbeHookB {
    fn before_insert(&self, _row: &mut Row) -> Result<()> {
        push_log("B:before")
    }

    fn after_insert(&self, _row: &Row) -> Result<()> {
        push_log("B:after")
    }
}

/// 变更追踪探针：记录每次 update Hook 看到的列集合（`Row::keys()` 顺序）。
struct ChangedColumnsHook;

thread_local! {
    /// 本线程的 update Hook 痕迹，形如 `before_update[id,status]`。
    static CHANGED_LOG: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// 取出并清空本线程的变更痕迹。
fn drain_changed_log() -> Vec<String> {
    CHANGED_LOG.with(|log| std::mem::take(&mut *log.borrow_mut()))
}

/// 记录 Hook 收到的列集合。
fn push_changed(label: &str, row: &Row) {
    let keys: Vec<&str> = row.keys().collect();
    CHANGED_LOG.with(|log| {
        log.borrow_mut()
            .push(format!("{label}[{}]", keys.join(",")))
    });
}

impl Hook for ChangedColumnsHook {
    fn before_update(&self, row: &mut Row) -> Result<()> {
        push_changed("before_update", row);
        Ok(())
    }

    fn after_update(&self, row: &Row) -> Result<()> {
        push_changed("after_update", row);
        Ok(())
    }
}

// ==================== 建表 ====================

/// 创建 Hook 测试所需的三张表。
async fn create_hook_tables(db: &Db) {
    db.execute(
        "CREATE TABLE hook_users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            email TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .await
    .expect("create hook_users table");

    db.execute(
        "CREATE TABLE reject_users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            email TEXT NOT NULL
        )",
    )
    .await
    .expect("create reject_users table");

    db.execute(
        "CREATE TABLE probe_users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            email TEXT NOT NULL
        )",
    )
    .await
    .expect("create probe_users table");

    db.execute(
        "CREATE TABLE changed_users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            email TEXT NOT NULL,
            status INTEGER NOT NULL
        )",
    )
    .await
    .expect("create changed_users table");
}

// ==================== 测试 ====================

#[tokio::test]
async fn timestamp_hook_fills_created_at_on_insert() {
    let db = setup_db().await;
    create_hook_tables(&db).await;

    // 故意设置 epoch 时间，验证 Hook 无条件覆盖为当前时间
    let epoch = DateTime::<Utc>::UNIX_EPOCH;
    let mut user = HookUser {
        id: 0,
        email: "hook@x.com".into(),
        created_at: epoch,
        updated_at: epoch,
    };
    user.insert_or_update(&db).await.unwrap();

    // save 后主键回填，self 通过 from_row 重建，Hook 填充值应同步回 self
    assert!(user.id > 0, "primary key should be backfilled");
    assert!(
        user.created_at > epoch,
        "self.created_at should be hook-filled"
    );
    assert!(
        user.updated_at > epoch,
        "self.updated_at should be hook-filled"
    );

    // 数据库中的值同样为 Hook 填充的当前时间
    let fetched = HookUser::find(&db, user.id).await.unwrap();
    assert!(
        fetched.created_at > epoch,
        "db created_at should be hook-filled"
    );
    assert!(
        fetched.updated_at > epoch,
        "db updated_at should be hook-filled"
    );
    assert_eq!(fetched.email, "hook@x.com");
}

#[tokio::test]
async fn timestamp_hook_refreshes_updated_at_on_update() {
    let db = setup_db().await;
    create_hook_tables(&db).await;

    let mut user = HookUser {
        id: 0,
        email: "update@x.com".into(),
        created_at: DateTime::<Utc>::UNIX_EPOCH,
        updated_at: DateTime::<Utc>::UNIX_EPOCH,
    };
    user.insert_or_update(&db).await.unwrap();
    let created_at_on_insert = user.created_at;

    // 确保两次取时有可观测的时间差
    std::thread::sleep(std::time::Duration::from_millis(5));

    user.email = "updated@x.com".into();
    user.insert_or_update(&db).await.unwrap(); // 主键非 0，走 UPDATE 分支

    let fetched = HookUser::find(&db, user.id).await.unwrap();
    assert_eq!(fetched.email, "updated@x.com");
    // updated_at 被 before_update 刷新为更晚的时间
    assert!(
        fetched.updated_at > created_at_on_insert,
        "updated_at should be refreshed on update"
    );
    // created_at 不受 UPDATE 影响
    assert_eq!(fetched.created_at, created_at_on_insert);
}

#[tokio::test]
async fn before_insert_error_aborts_save() {
    let db = setup_db().await;
    create_hook_tables(&db).await;

    let mut user = RejectUser {
        id: 0,
        email: "reject@x.com".into(),
    };

    let save_result = user.insert_or_update(&db).await;
    assert!(save_result.is_err(), "before_insert Err should abort save");

    // 数据库中不应有任何行
    let total = RejectUser::count(&db).await.unwrap();
    assert_eq!(total, 0, "no row should be inserted when hook rejects");
}

#[tokio::test]
async fn onion_order_and_row_mutation() {
    let db = setup_db().await;
    create_hook_tables(&db).await;

    let mut user = ProbeUser {
        id: 0,
        email: "probe@x.com".into(),
    };
    user.insert_or_update(&db).await.unwrap();

    // 洋葱模型：before 正序（A→B）、after 逆序（B→A）
    {
        let log = ORDER_LOG
            .lock()
            .map_err(|_| DbError::InvalidValue("order log poisoned".into()))
            .unwrap();
        assert_eq!(&*log, &["A:before", "B:before", "B:after", "A:after"]);
    }

    // 探针 A 的 before_insert 把 email 改为大写：应落库且同步回 self
    assert_eq!(user.email, "PROBE@X.COM");
    let fetched = ProbeUser::find(&db, user.id).await.unwrap();
    assert_eq!(fetched.email, "PROBE@X.COM");
}

// ==================== 变更追踪（D016） ====================

#[tokio::test]
async fn partial_update_hook_receives_only_changed_columns() {
    let db = setup_db().await;
    create_hook_tables(&db).await;

    let mut user = ChangedUser {
        id: 0,
        email: "keep@x.com".into(),
        status: 1,
    };
    user.insert_or_update(&db).await.unwrap();
    let _ = drain_changed_log();

    // 部分更新：变更集只改 status
    user.status = 2;
    let affected = ChangedUser::of(user.id)
        .set(ChangedUser::status(), user.status)
        .update(&db)
        .await
        .unwrap();
    assert_eq!(affected, 1);

    // Hook 拿到的行只含主键 + 变更列：审计无需再 SELECT + diff
    assert_eq!(
        drain_changed_log(),
        vec![
            "before_update[id,status]".to_string(),
            "after_update[id,status]".to_string(),
        ],
        "变更集只应把「主键 + 变更列」交给 Hook"
    );

    // 未列出的列保持原值
    let fetched = ChangedUser::find(&db, user.id).await.unwrap();
    assert_eq!(fetched.status, 2);
    assert_eq!(fetched.email, "keep@x.com");
}

#[tokio::test]
async fn full_update_hook_still_receives_every_column() {
    let db = setup_db().await;
    create_hook_tables(&db).await;

    let mut user = ChangedUser {
        id: 0,
        email: "all@x.com".into(),
        status: 1,
    };
    user.insert_or_update(&db).await.unwrap();
    let _ = drain_changed_log();

    // 整行更新：Hook 仍看到全部列（变更集的变更追踪不改变 update 语义）
    user.email = "all-updated@x.com".into();
    user.update(&db).await.unwrap();
    assert_eq!(
        drain_changed_log(),
        vec![
            "before_update[id,email,status]".to_string(),
            "after_update[id,email,status]".to_string(),
        ],
    );
}

#[tokio::test]
async fn partial_update_rejects_empty_changeset_without_writing() {
    let db = setup_db().await;
    create_hook_tables(&db).await;

    let mut user = ChangedUser {
        id: 0,
        email: "fail@x.com".into(),
        status: 1,
    };
    user.insert_or_update(&db).await.unwrap();

    // 未 `set` 任何列 → InvalidValue，且不发语句、不改数据
    //
    // 字段名写错的路径已由类型系统消除：`set` 的键是 `FieldExpr`，
    // 列名与值类型都在编译期校验，不存在「未知列」的运行时错误。
    let empty = ChangedUser::of(user.id).update(&db).await.unwrap_err();
    assert!(empty.to_string().contains("no fields set"), "got {empty}");

    let fetched = ChangedUser::find(&db, user.id).await.unwrap();
    assert_eq!(fetched.email, "fail@x.com");
    assert_eq!(fetched.status, 1);
}

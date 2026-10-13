//! 事务测试。

mod common;
use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use common::*;

/// 构造一条用户的 INSERT 参数。
fn user_params(email: &str, name: &str) -> Vec<Value> {
    vec![
        Value::Text(email.into()),
        Value::Text(name.into()),
        Value::I32(1),
        Value::Text("2026-01-01T00:00:00Z".into()),
        Value::Text("2026-01-01T00:00:00Z".into()),
    ]
}

/// 事务内插入用户的 SQL。
fn insert_user_sql() -> &'static str {
    "INSERT INTO users (email, name, status, created_at, updated_at) VALUES (?, ?, ?, ?, ?)"
}

#[tokio::test]
async fn transaction_commit_persists() {
    let db = setup().await;

    db.transaction(|tx| async move {
        tx.execute_with_params(insert_user_sql(), user_params("tx@example.com", "Tx"))
            .await?;
        Ok(())
    })
    .await
    .unwrap();

    let rows = db
        .find("SELECT * FROM users WHERE email = 'tx@example.com'")
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "committed data must be visible");
}

#[tokio::test]
async fn transaction_rollback_on_error() {
    let db = setup().await;

    let result: Result<()> = db
        .transaction(|tx| async move {
            tx.execute_with_params(insert_user_sql(), user_params("rb@example.com", "Rb"))
                .await?;
            Err(alun_db_core::error::DbError::InvalidValue("forced".into()))
        })
        .await;

    assert!(result.is_err());
    let rows = db
        .find("SELECT * FROM users WHERE email = 'rb@example.com'")
        .await
        .unwrap();
    assert!(rows.is_empty(), "rolled back data must not be visible");
}

#[tokio::test]
async fn transaction_rollback_on_drop() {
    let db = setup().await;

    // 显式句柄：插入后不提交也不回滚，句柄销毁触发 RAII 回滚
    {
        let tx = db.begin().await.unwrap();
        tx.execute_with_params(insert_user_sql(), user_params("drop@example.com", "Drop"))
            .await
            .unwrap();
    }

    let rows = db
        .find("SELECT * FROM users WHERE email = 'drop@example.com'")
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "uncommitted transaction must be rolled back on drop"
    );
}

#[tokio::test]
async fn explicit_transaction_commit() {
    let db = setup().await;

    let tx = db.begin().await.unwrap();
    tx.execute_with_params(
        insert_user_sql(),
        user_params("explicit@example.com", "Explicit"),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let rows = db
        .find("SELECT * FROM users WHERE email = 'explicit@example.com'")
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "explicit commit must persist");
}

#[tokio::test]
async fn explicit_transaction_rollback() {
    let db = setup().await;

    let tx = db.begin().await.unwrap();
    tx.execute_with_params(insert_user_sql(), user_params("exrb@example.com", "ExRb"))
        .await
        .unwrap();
    tx.rollback().await.unwrap();

    let rows = db
        .find("SELECT * FROM users WHERE email = 'exrb@example.com'")
        .await
        .unwrap();
    assert!(rows.is_empty(), "explicit rollback must discard changes");
}

#[tokio::test]
async fn transaction_query_reads_uncommitted_state() {
    let db = setup().await;

    // 事务内插入后，同一事务内可见（未提交读自身修改）
    let count = db
        .transaction(|tx| async move {
            tx.execute_with_params(insert_user_sql(), user_params("same@example.com", "Same"))
                .await?;
            let rows = tx
                .find("SELECT * FROM users WHERE email = 'same@example.com'")
                .await?;
            Ok(rows.len())
        })
        .await
        .unwrap();

    assert_eq!(count, 1, "own writes must be visible inside transaction");
}

#[tokio::test]
async fn commit_is_idempotent() {
    let db = setup().await;

    let tx = db.begin().await.unwrap();
    tx.commit().await.unwrap();
    // 第二次 commit 为幂等无操作
    tx.commit().await.unwrap();
}

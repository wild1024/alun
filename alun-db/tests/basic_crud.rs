//! 端到端 CRUD 测试。

mod common;
use alun_db::prelude::*;
use chrono::Utc;
use common::models::*;
use common::*;

fn sample_user(email: &str, name: &str) -> User {
    let now = Utc::now();
    User {
        id: 0,
        email: email.into(),
        name: name.into(),
        status: 1,
        profile: None,
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn insert_and_find() {
    let db = setup().await;

    let mut user = sample_user("alice@example.com", "Alice");
    user.insert_or_update(&db).await.unwrap();
    assert!(user.id > 0, "auto id should be populated");

    let fetched = User::find(&db, user.id).await.unwrap();
    assert_eq!(fetched.email, "alice@example.com");
    assert_eq!(fetched.name, "Alice");
}

#[tokio::test]
async fn find_or_none_returns_none_when_missing() {
    let db = setup().await;
    let result = User::find_or_none(&db, 999i64).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn find_by_unique_field() {
    let db = setup().await;

    let mut u1 = sample_user("alice@example.com", "Alice");
    u1.insert_or_update(&db).await.unwrap();

    let mut u2 = sample_user("bob@example.com", "Bob");
    u2.insert_or_update(&db).await.unwrap();

    let rows = User::find_by(&db, "email = ?", ("alice@example.com",))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Alice");
}

#[tokio::test]
async fn update_user() {
    let db = setup().await;

    let mut user = sample_user("alice@example.com", "Alice");
    user.insert_or_update(&db).await.unwrap();

    user.name = "Alice Updated".into();
    user.update(&db).await.unwrap();

    let fetched = User::find(&db, user.id).await.unwrap();
    assert_eq!(fetched.name, "Alice Updated");
}

#[tokio::test]
async fn delete_user() {
    let db = setup().await;

    let mut user = sample_user("alice@example.com", "Alice");
    user.insert_or_update(&db).await.unwrap();
    let id = user.id;

    user.delete(&db).await.unwrap();

    let result = User::find_or_none(&db, id).await.unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn count_all() {
    let db = setup().await;

    for i in 0..5 {
        let mut u = sample_user(&format!("u{}@x.com", i), &format!("User{}", i));
        u.insert_or_update(&db).await.unwrap();
    }

    let count = User::count(&db).await.unwrap();
    assert_eq!(count, 5);
}

#[tokio::test]
async fn all_users() {
    let db = setup().await;

    for i in 0..3 {
        let mut u = sample_user(&format!("u{}@x.com", i), &format!("User{}", i));
        u.insert_or_update(&db).await.unwrap();
    }

    let users = User::query().find(&db).await.unwrap();
    assert_eq!(users.len(), 3);
}

#[tokio::test]
async fn unique_constraint_violation() {
    let db = setup().await;

    let mut u1 = sample_user("alice@example.com", "Alice");
    u1.insert_or_update(&db).await.unwrap();

    let mut u2 = sample_user("alice@example.com", "Another Alice");
    let result = u2.insert_or_update(&db).await;
    assert!(result.is_err(), "duplicate email should fail");
}

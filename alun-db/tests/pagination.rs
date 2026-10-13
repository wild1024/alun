//! 分页测试。

mod common;
use alun_db::prelude::*;
use chrono::Utc;
use common::models::*;
use common::*;

async fn seed_users(db: &Db, n: usize) {
    for i in 0..n {
        let now = Utc::now();
        let mut u = User {
            id: 0,
            email: format!("u{}@x.com", i),
            name: format!("User{:02}", i),
            status: 1,
            profile: None,
            created_at: now,
            updated_at: now,
        };
        u.insert_or_update(db).await.unwrap();
    }
}

#[tokio::test]
async fn paginate_first_page() {
    let db = setup().await;
    seed_users(&db, 25).await;

    let page = User::query().paginate(&db, 1, 10).await.unwrap();
    assert_eq!(page.total, 25);
    assert_eq!(page.pages, 3);
    assert_eq!(page.page, 1);
    assert_eq!(page.size, 10);
    assert_eq!(page.list.len(), 10);
    assert!(page.has_next());
    assert!(!page.has_prev());
}

#[tokio::test]
async fn paginate_middle_page() {
    let db = setup().await;
    seed_users(&db, 25).await;

    let page = User::query().paginate(&db, 2, 10).await.unwrap();
    assert_eq!(page.list.len(), 10);
    assert!(page.has_next());
    assert!(page.has_prev());
}

#[tokio::test]
async fn paginate_last_page() {
    let db = setup().await;
    seed_users(&db, 25).await;

    let page = User::query().paginate(&db, 3, 10).await.unwrap();
    assert_eq!(page.list.len(), 5);
    assert!(!page.has_next());
    assert!(page.has_prev());
}

#[tokio::test]
async fn paginate_empty_result() {
    let db = setup().await;
    let page = User::query().paginate(&db, 1, 10).await.unwrap();
    assert_eq!(page.total, 0);
    assert_eq!(page.pages, 0);
    assert!(page.is_empty());
}

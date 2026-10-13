//! 类型映射测试：覆盖日期、JSON、枚举、可空字段。

mod common;
use common::models::*;
use common::*;
// 本文件使用 `Order` 作为领域模型；prelude 亦导出排序枚举 `Order`。
// 显式导入领域模型以覆盖 glob 导入，避免与排序枚举同名冲突。
use alun_db::prelude::*;
use chrono::{TimeZone, Utc};
use common::models::Order;

#[tokio::test]
async fn datetime_roundtrip() {
    let db = setup().await;

    let now = Utc.with_ymd_and_hms(2026, 9, 21, 12, 30, 45).unwrap();
    let mut user = User {
        id: 0,
        email: "dt@x.com".into(),
        name: "DateTime".into(),
        status: 1,
        profile: None,
        created_at: now,
        updated_at: now,
    };
    user.insert_or_update(&db).await.unwrap();

    let fetched = User::find(&db, user.id).await.unwrap();
    // `User` 含 `created_at`/`updated_at` → 自动挂载 `TimestampHook`，写入时以当前
    // 时间覆盖调用方取值，并回填到 model（`insert_or_update` 的主键回填路径）。
    // 本用例验证的是「写入值经驱动往返后精度/时区不漂移」，故与回填值比较。
    assert_eq!(fetched.created_at.timestamp(), user.created_at.timestamp());
    assert_eq!(fetched.updated_at.timestamp(), user.updated_at.timestamp());
}

#[tokio::test]
async fn json_field_roundtrip() {
    let db = setup().await;

    let now = Utc::now();
    let profile = Profile {
        avatar: Some("https://example.com/avatar.png".into()),
        bio: "Hello, world".into(),
    };
    let mut user = User {
        id: 0,
        email: "json@x.com".into(),
        name: "JsonUser".into(),
        status: 1,
        profile: Some(profile.clone()),
        created_at: now,
        updated_at: now,
    };
    user.insert_or_update(&db).await.unwrap();

    let fetched = User::find(&db, user.id).await.unwrap();
    assert_eq!(fetched.profile, Some(profile));
}

#[tokio::test]
async fn nullable_json_field() {
    let db = setup().await;

    let now = Utc::now();
    let mut user = User {
        id: 0,
        email: "null@x.com".into(),
        name: "NullJson".into(),
        status: 1,
        profile: None,
        created_at: now,
        updated_at: now,
    };
    user.insert_or_update(&db).await.unwrap();

    let fetched = User::find(&db, user.id).await.unwrap();
    assert!(fetched.profile.is_none());
}

#[tokio::test]
async fn enum_field_roundtrip() {
    let db = setup().await;

    let now = Utc::now();
    let mut order = Order {
        id: 0,
        user_id: 1,
        amount: "99.99".into(),
        status: OrderStatus::Confirmed,
        created_at: now,
    };
    order.insert_or_update(&db).await.unwrap();

    let fetched = Order::find(&db, order.id).await.unwrap();
    assert_eq!(fetched.status, OrderStatus::Confirmed);
}

#[tokio::test]
async fn enum_all_variants_roundtrip() {
    let db = setup().await;

    let now = Utc::now();
    for (i, status) in [
        OrderStatus::Pending,
        OrderStatus::Confirmed,
        OrderStatus::Shipped,
    ]
    .iter()
    .enumerate()
    {
        let mut order = Order {
            id: 0,
            user_id: 1,
            amount: format!("{}.00", i),
            status: *status,
            created_at: now,
        };
        order.insert_or_update(&db).await.unwrap();

        let fetched = Order::find(&db, order.id).await.unwrap();
        assert_eq!(fetched.status, *status);
    }
}

//! 主键列重命名（`#[column(...)]` + `#[key]`）的回归测试。
//!
//! 约定：`PRIMARY_KEY` 是 Rust 字段名，SQL 中主键标识符一律取
//! `primary_key_column()`（真实列名）。本测试验证重命名主键后
//! 增删改查全链路正确。

mod common;

use alun_db::prelude::*;
use chrono::{DateTime, Utc};

#[derive(Model, Debug, Clone, PartialEq)]
#[table("devices")]
pub struct Device {
    #[key]
    #[auto]
    #[column("device_id")]
    pub id: i64,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Model, Debug, Clone, PartialEq)]
#[table("gadgets")]
pub struct Gadget {
    #[key]
    #[auto]
    pub id: i64,
    #[unique]
    #[column("serial_no")]
    pub serial: String,
    pub label: String,
}

#[tokio::test]
async fn renamed_pk_crud_roundtrip() {
    let db = common::setup_db().await;
    db.execute(
        "CREATE TABLE devices (
            device_id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .await
    .expect("create devices table");

    // 约定：PRIMARY_KEY 为字段名，primary_key_column() 返回真实列名
    assert_eq!(Device::PRIMARY_KEY, "id");
    assert_eq!(Device::primary_key_column(), "device_id");

    // save：INSERT 用真实列名，自增主键回填
    let mut device = Device {
        id: 0,
        name: "sensor-a".into(),
        created_at: Utc::now(),
    };
    device.insert_or_update(&db).await.expect("save");
    assert!(device.id > 0);

    // find / find_or_none：WHERE 用真实列名
    let fresh = Device::find(&db, device.id).await.expect("find");
    assert_eq!(fresh.name, "sensor-a");
    assert!(
        Device::find_or_none(&db, device.id)
            .await
            .expect("find_or_none")
            .is_some()
    );

    // update：SET / WHERE 均用真实列名
    let mut updated = fresh;
    updated.name = "sensor-b".into();
    updated.update(&db).await.expect("update");
    let fresh2 = Device::find(&db, device.id).await.expect("find2");
    assert_eq!(fresh2.name, "sensor-b");

    // 变更集部分更新（WHERE 用真实列名）
    updated.name = "sensor-c".into();
    Device::of(updated.id)
        .set(Device::name(), updated.name.as_str())
        .update(&db)
        .await
        .expect("update");
    let fresh3 = Device::find(&db, device.id).await.expect("find3");
    assert_eq!(fresh3.name, "sensor-c");

    // delete / delete：WHERE 用真实列名
    let affected = Device::delete_by_pk(&db, device.id).await.expect("delete");
    assert_eq!(affected, 1);
    assert!(
        Device::find_or_none(&db, device.id)
            .await
            .expect("gone")
            .is_none()
    );
}

#[tokio::test]
async fn renamed_pk_save_existing_routes_to_update() {
    let db = common::setup_db().await;
    db.execute(
        "CREATE TABLE devices (
            device_id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .await
    .expect("create devices table");

    // 非自增主键（无 #[auto]）场景由 SELECT 1 WHERE device_id 判定新旧，
    // 此处用自增模型验证 save 已存在主键时走 update 而非报错
    let mut device = Device {
        id: 0,
        name: "first".into(),
        created_at: Utc::now(),
    };
    device.insert_or_update(&db).await.expect("save first");
    let id = device.id;
    device.name = "second".into();
    device.insert_or_update(&db).await.expect("save again");

    let fresh = Device::find(&db, id).await.expect("find");
    assert_eq!(fresh.name, "second");
    assert_eq!(
        db.scalar::<i64>("SELECT COUNT(*) FROM devices")
            .await
            .expect("count"),
        1i64
    );
}

#[tokio::test]
async fn renamed_unique_column_upsert_maps_field_names() {
    let db = common::setup_db().await;
    db.execute(
        "CREATE TABLE gadgets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            serial_no TEXT NOT NULL UNIQUE,
            label TEXT NOT NULL
        )",
    )
    .await
    .expect("create gadgets table");

    // upsert 参数为字段名（serial / label），应映射为列名 serial_no / label
    let mut g1 = Gadget {
        id: 0,
        serial: "SN-1".into(),
        label: "first".into(),
    };
    g1.upsert(&db, &["serial"], &["label"])
        .await
        .expect("upsert insert");

    // 同 serial 冲突：ON CONFLICT (serial_no) DO UPDATE SET label = ?
    let mut g2 = Gadget {
        id: 0,
        serial: "SN-1".into(),
        label: "second".into(),
    };
    g2.upsert(&db, &["serial"], &["label"])
        .await
        .expect("upsert update");

    assert_eq!(
        db.scalar::<i64>("SELECT COUNT(*) FROM gadgets")
            .await
            .expect("count"),
        1i64
    );
    let row = db
        .find_first_with_params(
            "SELECT label FROM gadgets WHERE serial_no = ?",
            vec![alun_db_core::stmt::Value::Text("SN-1".into())],
        )
        .await
        .expect("query")
        .expect("row exists");
    assert_eq!(
        row.get("label"),
        Some(&alun_db_core::stmt::Value::Text("second".into()))
    );
}

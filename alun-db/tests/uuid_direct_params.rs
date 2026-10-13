//! UUID 主键 / 参数**直接**传给 `CrudExt` 静态糖的集成测试。
//!
//! 覆盖点：`Uuid` 与 `&Uuid` 都实现了 `Into<Value>`（`uuid` feature 下），
//! 因此 `find` / `find_or_none` / `find_by` / `find_in_ids` / `delete_by_pk`
//! 都不再需要 `utils::uuid_value(...)` 之类的手工 `TypeMapper` 包装。
//!
//! `uuid` feature 由本 crate 的自引用 dev 依赖统一开启（见 Cargo.toml）。

mod common;

use alun_db::prelude::*;
use uuid::Uuid;

/// 以 UUID 为主键的模型（SQLite 下 UUID 以 TEXT 承载）。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("uuid_widgets")]
struct UuidWidget {
    /// 主键：UUID（非自增）。
    #[key]
    id: Uuid,
    /// 名称。
    name: String,
    /// 归属者：普通 UUID 外键列（用于验证 WHERE 参数直传）。
    owner_id: Uuid,
}

/// 建表并返回数据库句柄。
async fn setup() -> Db {
    let db = common::setup_db().await;
    db.execute(
        "CREATE TABLE uuid_widgets (
            id TEXT NOT NULL PRIMARY KEY,
            name TEXT NOT NULL,
            owner_id TEXT NOT NULL
        )",
    )
    .await
    .expect("create uuid_widgets table");
    db
}

/// 构造确定性的测试 UUID。
///
/// 刻意不用 `Uuid::new_v4()`：那需要 `uuid` crate 的 `v4` feature，而框架的依赖
/// 只启用默认特性（UUID 生成应由调用方按需开启 `v4` / `v7`）。
fn uid(seed: u8) -> Uuid {
    let mut bytes = [0_u8; 16];
    bytes[0] = seed;
    bytes[15] = seed;
    Uuid::from_bytes(bytes)
}

/// 插入一条记录。
async fn insert(db: &Db, id: Uuid, name: &str, owner_id: Uuid) {
    UuidWidget {
        id,
        name: name.to_string(),
        owner_id,
    }
    .insert(db)
    .await
    .expect("insert widget");
}

/// UUID 主键可直接用于主键查询与 WHERE 参数，无需任何包装转换。
#[tokio::test]
async fn uuid_keys_pass_directly_to_static_sugar() {
    let db = setup().await;
    let owner = uid(1);
    let (a, b, c) = (uid(2), uid(3), uid(4));
    insert(&db, a, "a", owner).await;
    insert(&db, b, "b", owner).await;
    insert(&db, c, "c", uid(5)).await;

    // 1) 主键查询：按值与借用都直接可用（避免为了所有权 clone）
    assert!(
        UuidWidget::find_or_none(&db, a)
            .await
            .expect("find")
            .is_some()
    );
    assert!(
        UuidWidget::find_or_none(&db, &a)
            .await
            .expect("find ref")
            .is_some()
    );

    // 2) WHERE 参数元组：`Uuid` 直传（Params 依赖 Into<Value>）
    let rows = UuidWidget::find_by(&db, "owner_id = ? ORDER BY name", (owner,))
        .await
        .expect("find_by");
    assert_eq!(
        rows.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(),
        vec!["a", "b"],
        "owner 直传应命中两条"
    );

    // 3) 借用形式同样可直传
    let rows = UuidWidget::find_by(&db, "owner_id = ?", (&owner,))
        .await
        .expect("find_by ref");
    assert_eq!(rows.len(), 2);

    // 4) `Option<Uuid>` 可直接作为参数（None → NULL），无需手工分支
    let rows = UuidWidget::find_by(&db, "id = ?", (Some(a),))
        .await
        .expect("find_by option");
    assert_eq!(rows.len(), 1);
    let none_hit = UuidWidget::find_by(&db, "id = ?", (None::<Uuid>,))
        .await
        .expect("find_by none");
    assert!(none_hit.is_empty(), "NULL 不等于任何 UUID 主键");

    // 5) IN 查询：元素实现 TypeMapper
    let found = UuidWidget::find_in_ids(&db, vec![a, b])
        .await
        .expect("find_in_ids");
    assert_eq!(found.len(), 2);

    // 6) 主键删除同样直传
    assert_eq!(
        UuidWidget::delete_by_pk(&db, c)
            .await
            .expect("delete_by_pk"),
        1
    );
    assert!(
        UuidWidget::find_or_none(&db, c)
            .await
            .expect("gone")
            .is_none()
    );
}

//! 迁移系统测试：版本号升序应用、幂等、唯一版本约束。

mod common;
use alun_db::prelude::*;
use common::*;

/// 版本 1：建 widgets 表。
fn mig1() -> Migration {
    Migration::new(
        1,
        "create_widgets",
        "CREATE TABLE widgets (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
        "DROP TABLE widgets",
    )
}

/// 版本 2：建 gadgets 表。
fn mig2() -> Migration {
    Migration::new(
        2,
        "create_gadgets",
        "CREATE TABLE gadgets (id INTEGER PRIMARY KEY)",
        "DROP TABLE gadgets",
    )
}

/// 版本 3：修改 widgets（加列），演示增量演进。
fn mig3() -> Migration {
    Migration::new(
        3,
        "widgets_add_qty",
        "ALTER TABLE widgets ADD COLUMN qty INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE widgets DROP COLUMN qty",
    )
}

#[tokio::test]
async fn migrations_apply_and_are_idempotent() {
    let db = setup_db().await;

    // 首次应用：版本 1、2 逐个建表
    let n = Migrator::from_migrations([mig2(), mig1()])
        .run(&db)
        .await
        .expect("first run");
    assert_eq!(
        n, 2,
        "applies both pending migrations (unordered input sorted)"
    );

    // 表确实存在且可写
    db.execute("INSERT INTO widgets (id, name) VALUES (1, 'a')")
        .await
        .expect("widgets exists");

    // 幂等：再次运行不重复应用
    let again = Migrator::from_migrations([mig1(), mig2()])
        .run(&db)
        .await
        .expect("second run");
    assert_eq!(again, 0, "no-op when already applied");

    // 增量演进：仅应用新增版本 3
    let next = Migrator::from_migrations([mig1(), mig2(), mig3()])
        .run(&db)
        .await
        .expect("third run");
    assert_eq!(next, 1, "applies only the new version");
    db.execute("INSERT INTO widgets (id, name, qty) VALUES (2, 'b', 7)")
        .await
        .expect("widgets has new column");
}

#[tokio::test]
async fn duplicate_migration_version_is_rejected() {
    let db = setup_db().await;
    let dup = Migration::new(
        1,
        "dup",
        "CREATE TABLE x1 (id INTEGER PRIMARY KEY)",
        "DROP TABLE x1",
    );
    let err = Migrator::from_migrations([mig1(), dup])
        .run(&db)
        .await
        .expect_err("duplicate version must fail");
    assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
}

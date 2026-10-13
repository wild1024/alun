//! 迁移回滚（`Migrator::rollback`）与存量导入的集成测试。

mod common;

use alun_db::prelude::*;

/// 迁移清单：v1 建表，v2 加列（均可逆）。
fn migrations() -> Vec<Migration> {
    vec![
        Migration::new(
            1,
            "create_users",
            "CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)",
            "DROP TABLE users",
        ),
        Migration::new(
            2,
            "add_age",
            "ALTER TABLE users ADD COLUMN age INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE users DROP COLUMN age",
        ),
    ]
}

#[tokio::test]
async fn migrate_down_rolls_back_latest() {
    let db = common::setup_db().await;

    let applied = Migrator::from_migrations(migrations())
        .run(&db)
        .await
        .expect("migrate up");
    assert_eq!(applied, 2);

    // 回滚最近一次（version 2 的先回滚，LIFO）
    let rolled = Migrator::from_migrations(migrations())
        .rollback(&db)
        .await
        .expect("rollback");
    assert_eq!(rolled.map(|v| v.to_string()), Some("2".to_string()));

    // 再回滚 version 1
    let rolled = Migrator::from_migrations(migrations())
        .rollback(&db)
        .await
        .expect("rollback");
    assert_eq!(rolled.map(|v| v.to_string()), Some("1".to_string()));

    // 无迁移可回滚时返回 None
    let rolled = Migrator::from_migrations(migrations())
        .rollback(&db)
        .await
        .expect("rollback");
    assert!(rolled.is_none());
}

#[tokio::test]
async fn migrate_down_is_idempotent_on_empty() {
    let db = common::setup_db().await;
    // 未应用任何迁移时直接回滚，返回 None（并自建 schema_migrations 表）
    assert!(
        Migrator::from_migrations(migrations())
            .rollback(&db)
            .await
            .expect("rollback")
            .is_none()
    );
}

#[tokio::test]
async fn legacy_sqlx_migrations_are_imported() {
    let db = common::setup_db().await;

    // 模拟旧框架（sqlx 迁移系统）留下的存量迁移记录表与历史版本
    // 说明：`_sqlx_migrations` 为旧 sqlx 迁移系统的固定表名。
    db.execute(
        "CREATE TABLE _sqlx_migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            installed_on TEXT NOT NULL,
            success BOOLEAN NOT NULL,
            checksum BLOB NOT NULL
        )",
    )
    .await
    .expect("create legacy migrations table");
    db.execute_with_params(
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum) VALUES (1, 'create_users', '2024-01-01 00:00:00', TRUE, X'00')",
        vec![],
    )
    .await
    .expect("seed legacy migration");

    // 旧迁移 v1 已建的实体表（存量库中已存在，模拟历史产物）
    db.execute("CREATE TABLE users (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL)")
        .await
        .expect("create legacy users table");

    // 开 `import_legacy` 后首次 migrate：历史版本已导入 `schema_migrations`，不再重复应用
    let migrator = Migrator::from_migrations(migrations()).options(MigratorOptions {
        import_legacy: true,
        ..MigratorOptions::default()
    });
    let applied = migrator.run(&db).await.expect("migrate");
    assert_eq!(applied, 1, "仅应用尚未记录的新迁移（version 2）");

    // 幂等：再迁移不重复
    assert_eq!(migrator.run(&db).await.expect("migrate again"), 0);
}

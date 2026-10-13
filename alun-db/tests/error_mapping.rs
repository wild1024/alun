//! 错误映射测试（D013 原生码映射 / D014 约束标识符 / D019 驱动原因可见）。
//!
//! SQLite 路径在本机即可运行；PostgreSQL / MySQL 的码映射由
//! `alun-db-core/src/error.rs` 的单元测试覆盖（不依赖真实服务器）。

mod common;

use alun_db::prelude::*;
use alun_db_core::error::DbError;
use alun_db_core::stmt::Value;
use std::error::Error;

/// 建表：唯一约束 / 外键 / CHECK 三类约束齐备。
async fn create_tables(db: &Db) {
    db.execute(
        "CREATE TABLE err_users (id INTEGER NOT NULL PRIMARY KEY, email TEXT NOT NULL UNIQUE)",
    )
    .await
    .expect("create err_users");
    db.execute(
        "CREATE TABLE err_items (
            id INTEGER NOT NULL PRIMARY KEY,
            user_id INTEGER NOT NULL REFERENCES err_users (id),
            amount INTEGER NOT NULL CHECK (amount >= 0)
        )",
    )
    .await
    .expect("create err_items");
}

#[tokio::test]
async fn unique_violation_exposes_column_not_raw_message() {
    let db = common::setup_db().await;
    create_tables(&db).await;
    db.execute("INSERT INTO err_users (id, email) VALUES (1, 'a@x.com')")
        .await
        .expect("seed");

    let err = db
        .execute("INSERT INTO err_users (id, email) VALUES (2, 'a@x.com')")
        .await
        .expect_err("重复 email 必须失败");

    match &err {
        DbError::UniqueViolation {
            column,
            constraint,
            source,
        } => {
            // 结构化标识符：列名解析自 `UNIQUE constraint failed: err_users.email`
            assert_eq!(column.as_deref(), Some("email"), "应提取列名而非原始消息");
            assert!(constraint.is_none(), "SQLite 不提供约束名");
            assert!(source.is_some(), "底层驱动错误必须保留（source 链）");
        }
        other => panic!("期望 UniqueViolation，实际 {other:?}"),
    }
    let text = err.to_string();
    assert!(
        text.contains("column `email`"),
        "Display 展示结构化列名：{text}"
    );
    assert!(
        !text.starts_with("unique constraint violated: UNIQUE"),
        "不得把驱动原文当作字段值：{text}"
    );
    assert!(err.source().is_some());
}

#[tokio::test]
async fn foreign_key_violation_without_constraint_name_stays_none() {
    let db = common::setup_db().await;
    create_tables(&db).await;

    let err = db
        .execute("INSERT INTO err_items (id, user_id, amount) VALUES (1, 999, 1)")
        .await
        .expect_err("孤儿外键必须失败");

    match &err {
        DbError::ForeignKeyViolation { constraint, source } => {
            assert_eq!(
                constraint, &None,
                "SQLite 不提供约束名 → 必须是 None，而不是驱动原文"
            );
            assert!(source.is_some());
        }
        other => panic!("期望 ForeignKeyViolation，实际 {other:?}"),
    }
    // 驱动原文只出现在 Display 的兜底文案里，不伪装成字段值
    let text = err.to_string();
    assert!(
        text.contains("driver:"),
        "无结构化信息时 Display 退回驱动原文：{text}"
    );
}

#[tokio::test]
async fn sqlite_check_violation_maps_to_check_violation() {
    let db = common::setup_db().await;
    create_tables(&db).await;

    let err = db
        .execute_with_params(
            "INSERT INTO err_items (id, user_id, amount) VALUES (?, ?, ?)",
            vec![Value::I64(1), Value::I64(1), Value::I64(-1)],
        )
        .await
        .expect_err("CHECK 约束必须失败");

    match &err {
        DbError::CheckViolation { constraint, source } => {
            assert!(source.is_some());
            // 未命名 CHECK：SQLite 在消息里放的是**表达式**（`amount >= 0`）而不是名字
            // → 必须保持 None（绝不把表达式/原文当作标识符）
            match constraint.as_deref() {
                None => {}
                Some(name) => {
                    assert!(
                        name.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.'),
                        "字段值必须是标识符形状：{name}"
                    );
                    assert!(name.contains("err_items"), "实际 {name}");
                }
            }
        }
        other => panic!("SQLite CHECK 违规应落 CheckViolation（D013），实际 {other:?}"),
    }
}

#[tokio::test]
async fn query_display_carries_driver_reason() {
    let db = common::setup_db().await;

    let err = db
        .execute("SELECT * FROM no_such_table_probe")
        .await
        .expect_err("必须失败");

    match &err {
        DbError::Query { sql, .. } => {
            assert!(
                sql.contains("no_such_table_probe"),
                "SQLite 驱动回填 SQL：{sql}"
            );
        }
        other => panic!("期望 Query，实际 {other:?}"),
    }
    let text = err.to_string();
    assert!(text.contains("query failed"), "{text}");
    assert!(
        text.contains("no such table"),
        "Display 必须带上底层驱动原因（D019），实际 {text}"
    );
    assert!(err.source().is_some(), "source 链保留");
}

#[tokio::test]
async fn missing_table_is_detected_through_source_chain() {
    // D023 的判定函数只作用于 `source` 链；此处用「全新库 + 无 _sqlx_migrations」
    // 走一遍导入探测路径：不应把「表缺失」当成真实错误。
    let db = common::setup_db().await;
    let dir = std::path::PathBuf::from("target")
        .join("error_mapping")
        .join("empty_dir");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");

    let applied = Migrator::from_dir(&dir)
        .run(&db)
        .await
        .expect("全新库迁移应成功");
    assert_eq!(applied, 0);
    // 迁移记录表已就绪（record 表本身被创建）
    let n: i64 = db
        .scalar("SELECT COUNT(*) FROM schema_migrations")
        .await
        .expect("schema_migrations 存在");
    assert_eq!(n, 0);
}

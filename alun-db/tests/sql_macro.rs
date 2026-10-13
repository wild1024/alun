//! `sql!` 宏集成测试。
//!
//! 覆盖：插值参数化（含注入 payload）、条件片段、`#` 指令
//! （`#where` / `#and` / `#orderBy` / `#limit` / `#groupBy`）、
//! 嵌套片段组合、链式 API（find / find_one / find_first /
//! execute）、事务内执行。

mod common;
use alun_db::prelude::*;
use common::*;

/// 插入基础测试数据：两个用户（不同 status）+ 两笔订单。
async fn seed(db: &Db) {
    for (email, name, status) in [
        ("alice@example.com", "Alice", 1),
        ("bob@example.com", "Bob", 2),
    ] {
        db.execute(&format!(
            "INSERT INTO users (email, name, status, created_at, updated_at) \
             VALUES ('{email}', '{name}', {status}, \
             '2026-01-01 00:00:00.000000', '2026-01-01 00:00:00.000000')"
        ))
        .await
        .expect("seed user");
    }
    db.execute(
        "INSERT INTO orders (user_id, amount, status, created_at) VALUES \
         (1, '99.50', 'P', '2026-01-01 00:00:00.000000'), \
         (1, '120.00', 'C', '2026-01-02 00:00:00.000000')",
    )
    .await
    .expect("seed orders");
}

#[tokio::test]
async fn interpolation_is_parameterized() {
    let db = setup().await;
    seed(&db).await;

    // 正常匹配
    let rows = sql!("SELECT id, email FROM users WHERE email = ", {
        "alice@example.com"
    })
    .find(&db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);

    // 注入 payload 作为参数值被安全处理，不会匹配任何行
    let rows = sql!("SELECT id FROM users WHERE email = ", { "a' OR '1'='1" })
        .find(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 0, "injection payload must not match");
}

#[tokio::test]
async fn conditional_fragment() {
    let db = setup().await;
    seed(&db).await;

    // Some → 条件拼接
    let name_filter: Option<String> = Some("Alice".into());
    let rows = sql!(
        "SELECT id FROM users WHERE 1=1",
        if let Some(n) = &name_filter { " AND name LIKE " { n } },
    )
    .find(&db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);

    // None → 条件省略
    let name_filter: Option<String> = None;
    let rows = sql!(
        "SELECT id FROM users WHERE 1=1",
        if let Some(n) = &name_filter { " AND name LIKE " { n } },
    )
    .find(&db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2, "None filter should be omitted");
}

#[tokio::test]
async fn where_directive_with_option() {
    let db = setup().await;
    seed(&db).await;

    // #where 生成首个条件；Some 时拼接
    let status: Option<i32> = Some(1);
    let rows = sql!("SELECT id FROM users #where(status, '=', status)")
        .find(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);

    // None 时整个条件省略
    let status: Option<i32> = None;
    let rows = sql!("SELECT id FROM users WHERE 1=1 #and(status, '=', status)")
        .find(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "None condition should be omitted");
}

#[tokio::test]
async fn order_by_limit_group_by_directives() {
    let db = setup().await;
    seed(&db).await;

    let rows = sql!("SELECT id FROM users #orderBy(id desc) #limit(1)")
        .find(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let max_id: i64 = rows[0].get_typed("id").unwrap();
    assert_eq!(max_id, 2);

    let rows =
        sql!("SELECT status, COUNT(*) AS cnt FROM users #groupBy(status) #orderBy(status asc)")
            .find(&db)
            .await
            .unwrap();
    assert_eq!(rows.len(), 2);
    let cnt: i64 = rows[0].get_typed("cnt").unwrap();
    assert_eq!(cnt, 1);
}

#[tokio::test]
async fn nested_fragment_combines_params() {
    let db = setup().await;
    seed(&db).await;

    // 子查询为裸表达式片段：SQL 文本与参数成对合并
    let sub = sql!("SELECT user_id FROM orders WHERE amount > ", {
        "100".to_string()
    });
    let rows = sql!("SELECT id, email FROM users WHERE id IN (", sub, ")")
        .find(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let email: String = rows[0].get_typed("email").unwrap();
    assert_eq!(email, "alice@example.com");
}

#[tokio::test]
async fn find_one_or_none_and_execute() {
    let db = setup().await;
    seed(&db).await;

    // find_one：命中
    let row = sql!("SELECT id FROM users WHERE email = ", {
        "alice@example.com"
    })
    .find_one(&db)
    .await
    .unwrap();
    assert_eq!(row.len(), 1);

    // find_one：无行报 NotFound
    let missing = sql!("SELECT id FROM users WHERE email = ", { "none@x.com" })
        .find_one(&db)
        .await;
    assert!(matches!(
        missing,
        Err(alun_db_core::error::DbError::NotFound)
    ));

    // find_first：无行返回 None
    let none = sql!("SELECT id FROM users WHERE email = ", { "none@x.com" })
        .find_first(&db)
        .await
        .unwrap();
    assert!(none.is_none());

    // execute：写语句
    let affected = sql!("UPDATE users SET status = ", { 9i32 }, " WHERE email = ", {
        "bob@example.com"
    })
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(affected, 1);
}

#[tokio::test]
async fn executes_in_transaction() {
    let db = setup().await;
    seed(&db).await;

    db.transaction(|tx| async move {
        sql!("UPDATE users SET status = ", { 7i32 }, " WHERE email = ", {
            "alice@example.com"
        })
        .execute(&tx)
        .await?;
        // 事务内立即可见
        let row = sql!("SELECT status FROM users WHERE email = ", {
            "alice@example.com"
        })
        .find_one(&tx)
        .await?;
        let status: i64 = row.get_typed("status")?;
        assert_eq!(status, 7);
        Ok(())
    })
    .await
    .unwrap();

    // 提交后持久化
    let row = sql!("SELECT status FROM users WHERE email = ", {
        "alice@example.com"
    })
    .find_one(&db)
    .await
    .unwrap();
    let status: i64 = row.get_typed("status").unwrap();
    assert_eq!(status, 7);
}

#[tokio::test]
async fn if_para_directives_bind_conditionally() {
    let db = setup().await;
    seed(&db).await;

    let status: Option<i32> = Some(2);
    let rows = sql!(
        "SELECT id FROM users WHERE 1=1",
        " #if(status.is_some()) AND status = #para(status) #end"
    )
    .find(&db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "条件成立时拼出 status 过滤");

    // 条件不成立：整个片段（含占位符与绑定参数）都不出现
    let status: Option<i32> = None;
    let rows = sql!(
        "SELECT id FROM users WHERE 1=1",
        " #if(status.is_some()) AND status = #para(status) #end"
    )
    .find(&db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2, "条件不成立时不加过滤");

    // #else 分支
    let status: Option<i32> = None;
    let rows = sql!(
        "SELECT id FROM users WHERE 1=1",
        " #if(status.is_some()) AND status = #para(status) #else AND status = 1 #end"
    )
    .find(&db)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "#else 分支生效");
}

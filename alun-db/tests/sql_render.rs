//! 验证 SQL 渲染器输出。

use alun_db::prelude::*;
use alun_db_sql::dialects::{MysqlDialect, PostgresDialect, SqliteDialect};

#[test]
fn render_simple_select() {
    let dialect = SqliteDialect;
    let stmt = Statement::Select(SelectStmt {
        table: "users",
        columns: vec![],
        filter: None,
        order_by: vec![],
        group_by: vec![],
        having: None,
        limit: None,
        offset: None,
        joins: vec![],
        ..Default::default()
    });
    let rendered = dialect.render_statement(&stmt);
    assert_eq!(rendered.sql, r#"SELECT * FROM "users""#);
    assert!(rendered.params.is_empty());
}

#[test]
fn render_select_with_filter() {
    let dialect = SqliteDialect;
    let stmt = Statement::Select(SelectStmt {
        table: "users",
        columns: vec![],
        filter: Some(Expr::Binary {
            left: Box::new(Expr::Column {
                table: "users",
                column: "status",
            }),
            op: BinaryOp::Eq,
            right: Box::new(Expr::Value(Value::I32(1))),
        }),
        order_by: vec![],
        group_by: vec![],
        having: None,
        limit: None,
        offset: None,
        joins: vec![],
        ..Default::default()
    });
    let rendered = dialect.render_statement(&stmt);
    assert_eq!(
        rendered.sql,
        r#"SELECT * FROM "users" WHERE "users"."status" = ?"#
    );
    assert_eq!(rendered.params, vec![Value::I32(1)]);
}

#[test]
fn render_placeholder_postgres() {
    let dialect = PostgresDialect;
    let stmt = Statement::Select(SelectStmt {
        table: "users",
        columns: vec![],
        filter: Some(Expr::And(vec![
            Expr::Binary {
                left: Box::new(Expr::Column {
                    table: "users",
                    column: "status",
                }),
                op: BinaryOp::Eq,
                right: Box::new(Expr::Value(Value::I32(1))),
            },
            Expr::Binary {
                left: Box::new(Expr::Column {
                    table: "users",
                    column: "name",
                }),
                op: BinaryOp::Like,
                right: Box::new(Expr::Value(Value::Text("%张%".into()))),
            },
        ])),
        order_by: vec![],
        group_by: vec![],
        having: None,
        limit: None,
        offset: None,
        joins: vec![],
        ..Default::default()
    });
    let rendered = dialect.render_statement(&stmt);
    assert!(rendered.sql.contains("$1"));
    assert!(rendered.sql.contains("$2"));
    assert!(!rendered.sql.contains('?'));
}

#[test]
fn render_mysql_backtick() {
    let dialect = MysqlDialect;
    let stmt = Statement::Select(SelectStmt {
        table: "users",
        columns: vec![],
        filter: None,
        order_by: vec![],
        group_by: vec![],
        having: None,
        limit: None,
        offset: None,
        joins: vec![],
        ..Default::default()
    });
    let rendered = dialect.render_statement(&stmt);
    assert_eq!(rendered.sql, "SELECT * FROM `users`");
}

#[test]
fn render_insert_with_params() {
    let dialect = SqliteDialect;
    let stmt = Statement::Insert(InsertStmt {
        table: "users",
        columns: vec!["email", "name"],
        values: vec![vec![
            Expr::Value(Value::Text("a@b.com".into())),
            Expr::Value(Value::Text("Alice".into())),
        ]],
        returning: None,
        on_conflict: None,
    });
    let rendered = dialect.render_statement(&stmt);
    assert_eq!(
        rendered.sql,
        r#"INSERT INTO "users" ("email", "name") VALUES (?, ?)"#
    );
    assert_eq!(rendered.params.len(), 2);
}

#[test]
fn render_pagination() {
    let dialect = SqliteDialect;
    let stmt = Statement::Select(SelectStmt {
        table: "users",
        columns: vec![],
        filter: None,
        order_by: vec![],
        group_by: vec![],
        having: None,
        limit: Some(20),
        offset: Some(40),
        joins: vec![],
        ..Default::default()
    });
    let rendered = dialect.render_statement(&stmt);
    // LIMIT / OFFSET 内联为字面量（参数化 LIMIT 会让 SQLite 每次查询多花约 2.7 µs）
    assert!(
        rendered.sql.ends_with("LIMIT 20 OFFSET 40"),
        "{}",
        rendered.sql
    );
    assert_eq!(rendered.params.len(), 0);
}

#[test]
fn render_ilike_fallback_on_sqlite() {
    let dialect = SqliteDialect;
    let stmt = Statement::Select(SelectStmt {
        table: "users",
        columns: vec![],
        filter: Some(Expr::Binary {
            left: Box::new(Expr::Column {
                table: "users",
                column: "name",
            }),
            op: BinaryOp::ILike,
            right: Box::new(Expr::Value(Value::Text("%alice%".into()))),
        }),
        order_by: vec![],
        group_by: vec![],
        having: None,
        limit: None,
        offset: None,
        joins: vec![],
        ..Default::default()
    });
    let rendered = dialect.render_statement(&stmt);
    // SQLite 不支持 ILIKE，应降级为 LOWER(...) LIKE LOWER(...)
    assert!(rendered.sql.contains("LOWER"));
    assert!(!rendered.sql.contains("ILIKE"));
}

#[test]
fn render_ilike_native_on_postgres() {
    let dialect = PostgresDialect;
    let stmt = Statement::Select(SelectStmt {
        table: "users",
        columns: vec![],
        filter: Some(Expr::Binary {
            left: Box::new(Expr::Column {
                table: "users",
                column: "name",
            }),
            op: BinaryOp::ILike,
            right: Box::new(Expr::Value(Value::Text("%alice%".into()))),
        }),
        order_by: vec![],
        group_by: vec![],
        having: None,
        limit: None,
        offset: None,
        joins: vec![],
        ..Default::default()
    });
    let rendered = dialect.render_statement(&stmt);
    assert!(rendered.sql.contains("ILIKE"));
}

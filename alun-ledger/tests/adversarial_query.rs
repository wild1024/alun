//! M12 查询与错误对抗（用例 QRY-001..007 + ERR-001..006，规划 §6.12）。
//!
//! 判读铁律 R2/R4：查询层「注入被拒」与错误层「变体映射」都必须记录
//! **真实**形态——尤其 `column` / `constraint` 装的是驱动原始消息，
//! 而不是结构化的列名 / 约束名。

mod common;

use std::error::Error;

use alun_db::prelude::*;
use alun_db_core::stmt::Value;

use alun_ledger::services::{money, rowx};

use common::assertions::*;
use common::fixtures::*;
use common::harness::{Dialect, LedgerCtx, ctx};

// ==================== 辅助 ====================

fn col(table: &'static str, column: &'static str) -> Expr {
    Expr::Column { table, column }
}

fn eq_columns(left: Expr, right: Expr) -> Expr {
    Expr::Binary {
        left: Box::new(left),
        op: BinaryOp::Eq,
        right: Box::new(right),
    }
}

fn eq_value(left: Expr, value: Value) -> Expr {
    Expr::Binary {
        left: Box::new(left),
        op: BinaryOp::Eq,
        right: Box::new(Expr::Value(value)),
    }
}

/// 契约种子：T1 租户 + 5 账户 + 2 用户（107 = 发票客户）+ 1 张草稿发票 + 1 条目。
async fn seed_common(c: &LedgerCtx) {
    seed_tenant(c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(c, TENANT_T1, money(100, 0)).await.unwrap();
    seed_user(c, TENANT_T1, 107, "buyer@x.com").await.unwrap();
    seed_user(c, TENANT_T1, 108, "other@x.com").await.unwrap();
    seed_invoice(c, TENANT_T1, 1001, "draft").await.unwrap();
}

/// 错误全文（含底层 `source`），用于检查「是否有修正建议」。
fn full_message(err: &DbError) -> String {
    let mut msg = format!("{err}");
    let mut src: Option<&(dyn Error + 'static)> = err.source();
    while let Some(s) = src {
        msg.push_str(" | ");
        msg.push_str(&s.to_string());
        src = s.source();
    }
    msg
}

/// 现场建 `err_probe`（CHECK 约束靶子；业务表里没有 CHECK 约束）。
async fn create_err_probe(c: &LedgerCtx) {
    let ddls: &[&str] = match c.dialect {
        Dialect::SqliteMem | Dialect::SqliteFile => &[
            "DROP TABLE IF EXISTS err_probe",
            "CREATE TABLE err_probe (id INTEGER NOT NULL PRIMARY KEY, \
             amount INTEGER NOT NULL CHECK (amount >= 0))",
        ],
        Dialect::Postgres => &[
            "DROP TABLE IF EXISTS err_probe",
            "CREATE TABLE err_probe (id BIGINT NOT NULL PRIMARY KEY, \
             amount NUMERIC(38, 10) NOT NULL CHECK (amount >= 0))",
        ],
        Dialect::Mysql => &[
            "DROP TABLE IF EXISTS err_probe",
            "CREATE TABLE err_probe (id BIGINT NOT NULL PRIMARY KEY, amount DECIMAL(38, 10) NOT NULL, \
             CONSTRAINT ck_err_probe_amount CHECK (amount >= 0))",
        ],
    };
    for ddl in ddls {
        c.db()
            .execute(ddl)
            .await
            .unwrap_or_else(|e| panic!("err_probe DDL 失败 `{ddl}`: {e}"));
    }
}

// ==================== QRY-001 ====================

/// QRY-001：5 表 JOIN（`FindBuilder::join`）结果与手写 SQL 一致。
#[tokio::test]
async fn qry_001_five_table_join_matches_handwritten_sql() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;

    let rows = c
        .db()
        .query()
        .table("tenants")
        .join(
            JoinKind::Inner,
            "users",
            eq_columns(col("users", "tenant_id"), col("tenants", "id")),
        )
        .join(
            JoinKind::Inner,
            "invoices",
            eq_columns(col("invoices", "customer_id"), col("users", "id")),
        )
        .join(
            JoinKind::Inner,
            "invoice_items",
            eq_columns(col("invoice_items", "invoice_id"), col("invoices", "id")),
        )
        .join(
            JoinKind::Inner,
            "accounts",
            eq_columns(col("accounts", "tenant_id"), col("tenants", "id")),
        )
        .filter(eq_value(col("tenants", "id"), Value::I64(TENANT_T1)))
        .filter(eq_value(col("accounts", "id"), Value::I64(t1_asset())))
        .order_by("invoice_items.id", Order::Asc)
        .exec(c.db())
        .await
        .expect("5 表 JOIN");
    assert_eq!(rows.len(), 1, "T1 单客户单条目单账户 → 1 行");

    // 手写 SQL 对照（同一批数据、同一批列）
    let hand = c
        .db()
        .find_with_params(
            "SELECT t.region AS t_region, u.email AS u_email, i.total AS i_total, \
             ii.description AS ii_desc, a.code AS a_code \
             FROM tenants t \
             INNER JOIN users u ON u.tenant_id = t.id \
             INNER JOIN invoices i ON i.customer_id = u.id \
             INNER JOIN invoice_items ii ON ii.invoice_id = i.id \
             INNER JOIN accounts a ON a.tenant_id = t.id \
             WHERE t.id = ? AND a.id = ? AND u.id = ? ORDER BY ii.id ASC",
            vec![
                Value::I64(TENANT_T1),
                Value::I64(t1_asset()),
                Value::I64(107),
            ],
        )
        .await
        .expect("手写 5 表 JOIN");
    assert_eq!(hand.len(), 1);

    let builder = &rows[0];
    let naive = &hand[0];
    assert_eq!(
        rowx::text_of(builder, "region").expect("region"),
        rowx::text_of(naive, "t_region").expect("t_region")
    );
    assert_eq!(
        rowx::text_of(builder, "email").expect("email"),
        rowx::text_of(naive, "u_email").expect("u_email")
    );
    assert_eq!(
        rowx::decimal_of(builder, "total").expect("total"),
        rowx::decimal_of(naive, "i_total").expect("i_total")
    );
    assert_eq!(
        rowx::text_of(builder, "description").expect("description"),
        rowx::text_of(naive, "ii_desc").expect("ii_desc")
    );
    assert_eq!(
        rowx::text_of(builder, "code").expect("code"),
        rowx::text_of(naive, "a_code").expect("a_code")
    );
    assert_eq!(rowx::text_of(builder, "email").unwrap(), "buyer@x.com");
    assert_eq!(rowx::text_of(builder, "code").unwrap(), "1001");
    // 租户作用域正确
    assert_tenant_scope(c.db(), TENANT_T1).await;
    eprintln!(
        "[QRY-001] {}: FindBuilder 5 表 JOIN == 手写 SQL（1 行，列值逐一相等）",
        c.dialect.name()
    );
}

// ==================== QRY-002 ====================

/// QRY-002：`FULL` / `RIGHT` JOIN——renderer 原样输出；MySQL 落服务端语法错误。
#[tokio::test]
async fn qry_002_full_and_right_join_rendering_and_failure() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;

    // (1) renderer：无能力协商，原样输出 `FULL JOIN` / `RIGHT JOIN`
    for (kind, expected) in [
        (JoinKind::Full, "FULL JOIN"),
        (JoinKind::Right, "RIGHT JOIN"),
    ] {
        let stmt = Statement::Select(SelectStmt {
            table: "tenants",
            columns: vec![],
            filter: None,
            order_by: vec![],
            group_by: vec![],
            having: None,
            limit: None,
            offset: None,
            joins: vec![Join {
                kind,
                table: "users",
                on: eq_columns(col("users", "tenant_id"), col("tenants", "id")),
                scope: None,
            }],
            ..Default::default()
        });
        let rendered = c.db().dialect().render_statement(&stmt);
        assert!(
            rendered.sql.contains(expected),
            "renderer 原样输出 `{expected}`（不做方言改写）：{}",
            rendered.sql
        );
        eprintln!(
            "[QRY-002] {}: rendered = `{}`",
            c.dialect.name(),
            rendered.sql
        );
    }

    // (2) 执行：FULL JOIN（MySQL 服务端语法错误）
    let full = c
        .db()
        .query()
        .table("tenants")
        .join(
            JoinKind::Full,
            "users",
            eq_columns(col("users", "tenant_id"), col("tenants", "id")),
        )
        .filter(eq_value(col("tenants", "id"), Value::I64(TENANT_T1)))
        .exec(c.db())
        .await;
    match c.dialect {
        Dialect::Mysql => match full {
            Err(DbError::Query { sql, source }) => {
                // 各 MySQL 版本/服务器措辞不一，这里只要求"服务端错误 + 可读原因"
                let msg = full_message(&DbError::Query { sql, source });
                assert!(!msg.is_empty(), "MySQL FULL JOIN 应返回可读的服务端错误");
                eprintln!(
                    "[QRY-002] mysql: FULL JOIN → DbError::Query（服务端错误：{msg}，非 UnsupportedFeature）"
                );
            }
            other => panic!("MySQL 上 FULL JOIN 应落 Query，实际 {other:?}"),
        },
        Dialect::SqliteMem | Dialect::SqliteFile | Dialect::Postgres => {
            let rows = full.expect("SQLite ≥3.39 / PG 支持 FULL JOIN");
            assert!(
                rows.iter()
                    .any(|r| rowx::text_of(r, "region").ok().as_deref() == Some("cn")),
                "FULL JOIN 结果包含 T1 行"
            );
            eprintln!(
                "[QRY-002] {}: FULL JOIN 被本地引擎接受（{} 行）",
                c.dialect.name(),
                rows.len()
            );
        }
    }

    // (3) RIGHT JOIN：MySQL 支持（与规划中「MySQL 不支持 RIGHT JOIN」的记录不符）
    let right = c
        .db()
        .query()
        .table("tenants")
        .join(
            JoinKind::Right,
            "users",
            eq_columns(col("users", "tenant_id"), col("tenants", "id")),
        )
        .filter(eq_value(col("tenants", "id"), Value::I64(TENANT_T1)))
        .exec(c.db())
        .await;
    match right {
        Ok(rows) => {
            assert!(!rows.is_empty(), "RIGHT JOIN 应返回 T1 行");
            eprintln!(
                "[QRY-002] {}: RIGHT JOIN 执行成功（{} 行）→ 失败形态仅限 FULL JOIN",
                c.dialect.name(),
                rows.len()
            );
        }
        Err(e) => {
            assert!(
                matches!(e, DbError::Query { .. }),
                "RIGHT JOIN 失败应为服务端 Query，实际 {e:?}"
            );
            eprintln!(
                "[QRY-002] {}: RIGHT JOIN → DbError::Query：{e}",
                c.dialect.name()
            );
        }
    }
}

// ==================== QRY-003 ====================

/// QRY-003：`sql!` 子查询嵌套——参数顺序与占位符一致。
#[tokio::test]
async fn qry_003_nested_subquery_param_alignment() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;

    let nested = sql!("SELECT invoice_id FROM invoice_items WHERE quantity = ", {
        1_i64
    });
    let query = sql!(
        "SELECT email FROM users WHERE tenant_id = ",
        { TENANT_T1 },
        " AND id IN (SELECT customer_id FROM invoices WHERE id IN (",
        nested,
        "))"
    );

    let (text, params) = query.clone().into_parts();
    assert_eq!(
        text,
        "SELECT email FROM users WHERE tenant_id = ? AND id IN \
         (SELECT customer_id FROM invoices WHERE id IN \
         (SELECT invoice_id FROM invoice_items WHERE quantity = ?))",
        "嵌套片段按占位符出现顺序拼接"
    );
    assert_eq!(
        params,
        vec![Value::I64(TENANT_T1), Value::I64(1)],
        "参数顺序与占位符一致（外层先出现）"
    );

    let rows = query.find(c.db()).await.expect("nested subquery");
    assert_eq!(rows.len(), 1);
    assert_eq!(rowx::text_of(&rows[0], "email").unwrap(), "buyer@x.com");

    // 占位符按方言改写（PG `$n`；SQLite/MySQL 保持 `?`）
    let rewritten = alun_db_sql::rewrite_placeholders(&text, c.db().dialect());
    if c.dialect == Dialect::Postgres {
        assert!(
            rewritten.contains("$1") && rewritten.contains("$2"),
            "{rewritten}"
        );
        assert!(!rewritten.contains('?'));
    } else {
        assert_eq!(rewritten, text, "{} 保持 `?` 占位符", c.dialect.name());
    }
    eprintln!(
        "[QRY-003] {}: 2 处占位符与 2 个参数顺序对齐",
        c.dialect.name()
    );
}

// ==================== QRY-004 ====================

/// QRY-004：`GROUP BY + HAVING`（raw SQL）聚合结果正确。
#[tokio::test]
async fn qry_004_group_by_having_raw_sql() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;
    // T1 第二张发票（同状态），T3 一张（HAVING 过滤掉）
    seed_invoice(&c, TENANT_T1, 1002, "draft").await.unwrap();
    seed_tenant(&c, TENANT_T3, "T3").await.unwrap();
    seed_invoice(&c, TENANT_T3, 3001, "draft").await.unwrap();

    let rows = c
        .db()
        .find_with_params(
            "SELECT tenant_id, COUNT(*) AS cnt, SUM(quantity) AS qty \
             FROM invoices INNER JOIN invoice_items ON invoice_items.invoice_id = invoices.id \
             WHERE invoices.status = ? GROUP BY tenant_id HAVING COUNT(*) >= ? \
             ORDER BY tenant_id ASC",
            vec![Value::Text("draft".into()), Value::I64(2)],
        )
        .await
        .expect("GROUP BY + HAVING");
    assert_eq!(rows.len(), 1, "只有 T1 达到 HAVING 阈值");
    assert_eq!(rowx::i64_of(&rows[0], "tenant_id").unwrap(), TENANT_T1);
    assert_eq!(int_of(&rows[0], "cnt"), 2, "COUNT(*) 归一到 i64");
    assert_eq!(int_of(&rows[0], "qty"), 2, "SUM(quantity) 归一到 i64");

    // HAVING 阈值提高后无行（NULL 也被正确排除）
    let none = c
        .db()
        .find_with_params(
            "SELECT tenant_id, COUNT(*) AS cnt FROM invoices \
             WHERE status = ? GROUP BY tenant_id HAVING COUNT(*) >= ?",
            vec![Value::Text("draft".into()), Value::I64(3)],
        )
        .await
        .expect("GROUP BY + HAVING(3)");
    assert!(none.is_empty(), "阈值 3 时无租户达标");
    eprintln!(
        "[QRY-004] {}: GROUP BY/HAVING 聚合 = 手写期望",
        c.dialect.name()
    );
}

// ==================== QRY-005 ====================

/// QRY-005：标识符注入尝试必须被拒（`DbError::InvalidValue`），表仍存在。
#[tokio::test]
async fn qry_005_identifier_injection_rejected_and_table_survives() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;

    let before: i64 = count_by(
        c.db(),
        "SELECT COUNT(*) FROM invoices WHERE tenant_id = ?",
        vec![Value::I64(TENANT_T1)],
    )
    .await;

    let table_payloads = [
        "users; --",
        "users; DROP TABLE invoices",
        "users/*x*/",
        "\"users\"",
        "users id",
        "1=1",
    ];
    for payload in table_payloads {
        let res = c.db().query().table(payload).exec(c.db()).await;
        match res {
            Err(DbError::InvalidValue(msg)) => {
                assert!(msg.contains("invalid identifier"), "拒绝理由: {msg}");
            }
            other => panic!("table(`{payload}`) 应被拒，实际 {other:?}"),
        }
        eprintln!(
            "[QRY-005] {}: table(`{payload}`) → InvalidValue",
            c.dialect.name()
        );
    }

    let order_payloads = [
        "id; DROP TABLE invoices",
        "id DESC; DROP TABLE invoices",
        "id--",
        "ID ASC, (SELECT 1)",
    ];
    for payload in order_payloads {
        let res = c
            .db()
            .query()
            .table("users")
            .order_by(payload, Order::Desc)
            .exec(c.db())
            .await;
        match res {
            Err(DbError::InvalidValue(_)) => {}
            other => panic!("order_by(`{payload}`) 应被拒，实际 {other:?}"),
        }
        eprintln!(
            "[QRY-005] {}: order_by(`{payload}`) → InvalidValue",
            c.dialect.name()
        );
    }

    // 表仍在、行数未变（payload 未被执行）
    let after: i64 = count_by(
        c.db(),
        "SELECT COUNT(*) FROM invoices WHERE tenant_id = ?",
        vec![Value::I64(TENANT_T1)],
    )
    .await;
    assert_eq!(before, after, "注入尝试不得改动 invoices");
    assert_eq!(after, 1);
    eprintln!(
        "[QRY-005] {}: 注入尝试全部被拒，invoices 行数仍为 {after}",
        c.dialect.name()
    );
}

// ==================== QRY-006 ====================

/// QRY-006：`sql!` 指令体的标识符——编译期白名单 → 注入不可能；
/// 运行期只暴露「原样插入、不 quote」导致的保留字/大小写问题。
#[tokio::test]
async fn qry_006_macro_directive_identifier_quoting() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;

    // 指令体原样插入（不按方言 quote，也不改成占位符）；方向关键字统一大写，
    // 且指令自带前导空格 → 与模板尾空格叠加成双空格
    let (order_text, order_params) = sql!("SELECT id FROM users #orderBy(id desc)").into_parts();
    assert_eq!(
        order_text, "SELECT id FROM users  ORDER BY id DESC",
        "`#orderBy` 原样插入标识符（`desc` → `DESC`）"
    );
    assert!(order_params.is_empty(), "`#orderBy` 不产生绑定参数");
    let (limit_text, limit_params) = sql!("SELECT id FROM users #limit(1)").into_parts();
    assert_eq!(
        limit_text, "SELECT id FROM users  LIMIT 1",
        "`#limit` 是字面量插入，不产生绑定参数"
    );
    assert!(limit_params.is_empty());
    eprintln!("[QRY-006] 指令体原样插入：`{order_text}` / `{limit_text}`");

    // 合法标识符：正常执行
    let ok = sql!("SELECT id FROM users #orderBy(id desc)")
        .find(c.db())
        .await
        .expect("合法列名");
    assert!(!ok.is_empty());

    // 保留字列名：编译期字符集白名单放过（`order` 是合法标识符），
    // 但不 quote → 服务端按关键字解析
    let reserved = sql!("SELECT id FROM users #orderBy(order desc)")
        .find(c.db())
        .await;
    match reserved {
        Err(DbError::Query { sql, source }) => {
            if c.dialect.is_sqlite() {
                assert!(
                    sql.contains("ORDER BY order DESC"),
                    "SQLite 驱动回填失败 SQL 上下文: {sql}"
                );
            }
            eprintln!(
                "[QRY-006] {}: `#orderBy(order desc)` → Query：{source}",
                c.dialect.name()
            );
        }
        Err(other) => panic!("保留字列名应落服务端错误，实际 {other:?}"),
        Ok(rows) => {
            eprintln!(
                "[QRY-006][差异登记] {}: `ORDER BY order DESC` 被引擎接受（{} 行）——该方言允许关键字作标识符",
                c.dialect.name(),
                rows.len()
            );
        }
    }

    // 大小写变体：不 quote → 依赖方言的大小写折叠规则（列名本就小写）
    let upper = sql!("SELECT id FROM users #orderBy(ID desc)")
        .find(c.db())
        .await;
    match upper {
        Ok(rows) => eprintln!(
            "[QRY-006] {}: `#orderBy(ID desc)` 成功（{} 行，方言大小写不敏感/折叠为小写）",
            c.dialect.name(),
            rows.len()
        ),
        Err(e) => eprintln!(
            "[QRY-006] {}: `#orderBy(ID desc)` 失败：{e}",
            c.dialect.name()
        ),
    }

    // 结论登记：注入载荷无法进入 `#orderBy`（`template.rs` 编译期
    // `validate_identifier` 只放行 `[a-zA-Z_][a-zA-Z0-9_]*`）；
    // 运行期等价入口（FindBuilder）的拒绝行为见 QRY-005。
    let runtime_equivalent = c
        .db()
        .query()
        .table("users")
        .order_by("order desc", Order::Desc)
        .exec(c.db())
        .await;
    assert!(
        matches!(runtime_equivalent, Err(DbError::InvalidValue(_))),
        "运行期标识符入口必须拒绝带空格/关键字的载荷，实际 {runtime_equivalent:?}"
    );
    eprintln!(
        "[QRY-006] {}: 运行期入口拒绝同一载荷 → 注入不可能（失败形态仅限保留字/大小写）",
        c.dialect.name()
    );
}

// ==================== QRY-007 ====================

/// QRY-007：`#limit` 边界（0 / i64::MAX / u64::MAX 字面量）与 OFFSET 组合。
#[tokio::test]
async fn qry_007_limit_boundaries_and_offset() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;

    // `#limit` 是**字面量**插入（非参数绑定）；负数/非法值在编译期被
    // `template.rs` 的 `parse::<u64>()` 拒绝，运行期无法表达
    let zero = sql!("SELECT id FROM users #limit(0)")
        .find(c.db())
        .await
        .expect("limit 0");
    assert!(zero.is_empty(), "LIMIT 0 返回 0 行");

    let one = sql!("SELECT id FROM users #orderBy(id asc) #limit(1)")
        .find(c.db())
        .await
        .expect("limit 1");
    assert_eq!(one.len(), 1);

    // i64::MAX 字面量：合法且不截断
    let all = sql!("SELECT id FROM users #limit(9223372036854775807)")
        .find(c.db())
        .await
        .expect("limit i64::MAX");
    assert_eq!(all.len(), 2, "T1 有 2 个用户");

    // u64::MAX 字面量：超出驱动/服务端整数范围 → 记录真实形态
    let over = sql!("SELECT id FROM users #limit(18446744073709551615)")
        .find(c.db())
        .await;
    match over {
        Ok(rows) => {
            assert_eq!(rows.len(), 2);
            eprintln!(
                "[QRY-007] {}: `#limit(18446744073709551615)` 被接受并返回全部 {rows_len} 行（字面量静默饱和）",
                c.dialect.name(),
                rows_len = rows.len()
            );
        }
        Err(e) => {
            assert!(
                matches!(e, DbError::Query { .. } | DbError::InvalidValue(_)),
                "超 MAX 字面量应显式报错，实际 {e:?}"
            );
            eprintln!(
                "[QRY-007] {}: `#limit(18446744073709551615)` → {e}",
                c.dialect.name()
            );
        }
    }

    // FindBuilder 路径：LIMIT/OFFSET 走**参数绑定**，类型是 u64（负数不可表示）
    let page = c
        .db()
        .query()
        .table("users")
        .order_by("id", Order::Asc)
        .limit(1)
        .offset(1)
        .exec(c.db())
        .await
        .expect("limit 1 offset 1");
    assert_eq!(page.len(), 1);
    assert_eq!(
        rowx::i64_of(&page[0], "id").unwrap(),
        108,
        "OFFSET 1 跳过首个用户"
    );

    let empty = c
        .db()
        .query()
        .table("users")
        .order_by("id", Order::Asc)
        .limit(5)
        .offset(1000)
        .exec(c.db())
        .await
        .expect("offset beyond end");
    assert!(empty.is_empty(), "OFFSET 越过末尾返回空集而非错误");

    let zero_limit = c
        .db()
        .query()
        .table("users")
        .limit(0)
        .exec(c.db())
        .await
        .expect("builder limit 0");
    assert!(zero_limit.is_empty());

    // OFFSET 无 LIMIT：SQLite/MySQL 语法要求 LIMIT 先于 OFFSET
    let offset_only = c.db().query().table("users").offset(1).exec(c.db()).await;
    match (c.dialect, offset_only) {
        (Dialect::Postgres, Ok(rows)) => {
            assert_eq!(rows.len(), 1, "PG 允许无 LIMIT 的 OFFSET")
        }
        (Dialect::Postgres, Err(e)) => panic!("PG 上 OFFSET 无 LIMIT 应成功，实际 {e:?}"),
        (_, Err(DbError::Query { .. })) => eprintln!(
            "[QRY-007] {}: builder `.offset(1)` 无 LIMIT → Query（语法要求 LIMIT 先于 OFFSET）",
            c.dialect.name()
        ),
        (_, other) => panic!(
            "{} 上 OFFSET 无 LIMIT 的真实形态意外: {other:?}",
            c.dialect.name()
        ),
    }
    eprintln!(
        "[QRY-007] {}: #limit 边界 + FindBuilder limit/offset 边界记录完成",
        c.dialect.name()
    );
}

// ==================== ERR-001 ====================

/// ERR-001：唯一约束 → `UniqueViolation{column, constraint, source}` 携带**结构化标识符**。
#[tokio::test]
async fn err_001_unique_violation_carries_structured_identifiers() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;

    let err = c
        .db()
        .execute_with_params(
            "INSERT INTO users (id, tenant_id, email, name, status, role, created_at, updated_at, \
             first_name, last_name, profile, locale) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(109),
                Value::I64(TENANT_T1),
                Value::Text("buyer@x.com".into()), // 与 107 冲突（uq_users_email）
                Value::Text("dupe".into()),
                Value::Text("active".into()),
                Value::Text("member".into()),
                at(0).into(),
                at(0).into(),
                Value::Text("D".into()),
                Value::Text("U".into()),
                Value::Null,
                Value::Text("zh-CN".into()),
            ],
        )
        .await
        .expect_err("重复 email 必须失败");

    match err {
        DbError::UniqueViolation {
            column,
            constraint,
            source,
            ..
        } => {
            // 结构化标识符：SQLite 给列名，PG/MySQL 给约束名；取不到为 None
            if c.dialect.is_sqlite() {
                assert_eq!(
                    column.as_deref().map(|s| s.contains("email")),
                    Some(true),
                    "SQLite 应以列名表达冲突：column={column:?} constraint={constraint:?}"
                );
            } else {
                assert!(
                    constraint
                        .as_deref()
                        .map(|s| s.contains("uq_users_email"))
                        .unwrap_or(false),
                    "{} 应以约束名表达冲突：column={column:?} constraint={constraint:?}",
                    c.dialect.name()
                );
            }
            // 绝不回填驱动原文（原文应留在 source 链上）
            for raw in [column.as_deref(), constraint.as_deref()]
                .into_iter()
                .flatten()
            {
                assert!(
                    !raw.contains("constraint failed") && !raw.contains("Duplicate entry"),
                    "约束字段不得回填驱动原文：`{raw}`"
                );
            }
            assert!(
                source.is_some(),
                "原始驱动错误应保留在 source 链上（结构化字段与原文并存）"
            );
            eprintln!(
                "[ERR-001] {}: column={column:?} constraint={constraint:?}（结构化标识符）",
                c.dialect.name()
            );
        }
        other => panic!("期望 UniqueViolation，实际 {other:?}"),
    }
    // 失败后行数不变
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM users WHERE tenant_id = ?",
            vec![Value::I64(TENANT_T1)]
        )
        .await,
        2
    );
}

// ==================== ERR-002 ====================

/// ERR-002：外键约束 → `ForeignKeyViolation{constraint, source}`（结构化；SQLite 无名则为 None）。
#[tokio::test]
async fn err_002_foreign_key_violation_carries_structured_identifier() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;

    let err = c
        .db()
        .execute_with_params(
            "INSERT INTO invoice_items (id, invoice_id, description, quantity, unit_price, amount, tax_rate) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(900001),
                Value::I64(999_999), // 不存在的发票
                Value::Text("orphan".into()),
                Value::I64(1),
                money(1, 0).to_value(),
                money(1, 0).to_value(),
                Value::Text("0".into()),
            ],
        )
        .await
        .expect_err("孤儿条目必须失败");

    match err {
        DbError::ForeignKeyViolation { constraint, .. } => {
            // 结构化约束名；驱动不提供时必须是 None（不得回填原文）
            if let Some(name) = &constraint {
                assert!(
                    !name.contains("constraint failed") && !name.contains("foreign key"),
                    "约束字段不得回填驱动原文：`{name}`"
                );
            }
            if c.dialect.is_sqlite() {
                assert!(
                    constraint.is_none(),
                    "SQLite 不提供约束名 → 必须是 None（而非原文）：{constraint:?}"
                );
                eprintln!(
                    "[ERR-002] sqlite-mem: ForeignKeyViolation.constraint = None（驱动无约束名）"
                );
            } else {
                assert!(
                    constraint
                        .as_deref()
                        .map(|s| s.contains("fk_invoice_items_invoice"))
                        .unwrap_or(false),
                    "{} 应从结构化字段给出约束名：{constraint:?}",
                    c.dialect.name()
                );
                eprintln!(
                    "[ERR-002] {}: ForeignKeyViolation.constraint = {constraint:?}",
                    c.dialect.name()
                );
            }
        }
        other => panic!("期望 ForeignKeyViolation，实际 {other:?}"),
    }
}

// ==================== ERR-003 ====================

/// ERR-003：CHECK 约束 → `CheckViolation`（SQLite 扩展码 275 亦已映射，D013 已修复）。
#[tokio::test]
async fn err_003_check_violation_mapping() {
    let Some(c) = ctx().await else { return };
    create_err_probe(&c).await;

    let err = c
        .db()
        .execute_with_params(
            "INSERT INTO err_probe (id, amount) VALUES (?, ?)",
            vec![Value::I64(1), money(-1, 0).to_value()],
        )
        .await
        .expect_err("CHECK 约束必须失败");

    match c.dialect {
        Dialect::SqliteMem | Dialect::SqliteFile => match err {
            // SQLite 扩展码 275 已映射为 CheckViolation（D013 已修复）
            DbError::CheckViolation { constraint, .. } => {
                if let Some(name) = &constraint {
                    assert!(
                        !name.to_lowercase().contains("check constraint failed"),
                        "`constraint` 不得是驱动原文：{name}"
                    );
                }
                eprintln!("[ERR-003] sqlite-mem: CheckViolation.constraint = {constraint:?}");
            }
            other => panic!("SQLite CHECK 违规应落 CheckViolation（D013 已修复），实际 {other:?}"),
        },
        Dialect::Postgres | Dialect::Mysql => match err {
            DbError::CheckViolation { constraint, .. } => {
                if let Some(name) = &constraint {
                    assert!(
                        !name.to_lowercase().contains("check constraint"),
                        "`constraint` 不得是驱动原文：{name}"
                    );
                }
                eprintln!(
                    "[ERR-003] {}: CheckViolation.constraint = {constraint:?}",
                    c.dialect.name()
                );
            }
            other => panic!("期望 CheckViolation，实际 {other:?}"),
        },
    }
}

// ==================== ERR-004 ====================

/// ERR-004：类型不匹配 / 意外 NULL 的变体与载荷。
#[tokio::test]
async fn err_004_type_mismatch_and_unexpected_null() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;

    // TypeMismatch：整数列按文本读（`rowx::text_of`）
    let row = c
        .db()
        .find_first("SELECT id FROM users WHERE id = 107")
        .await
        .unwrap()
        .unwrap();
    match rowx::text_of(&row, "id") {
        Err(DbError::TypeMismatch { expected, actual }) => {
            assert_eq!((expected, actual), ("Text", "I64"));
            eprintln!(
                "[ERR-004] {}: text_of(I64 列) → TypeMismatch{{Text, I64}}",
                c.dialect.name()
            );
        }
        other => panic!("期望 TypeMismatch，实际 {other:?}"),
    }

    // TypeMismatch：缺失列 → ColumnNotFound（不同变体）
    match rowx::text_of(&row, "no_such_column") {
        Err(DbError::ColumnNotFound(name)) => assert_eq!(name, "no_such_column"),
        other => panic!("期望 ColumnNotFound，实际 {other:?}"),
    }

    // UnexpectedNull：金额适配器遇到 SQL NULL
    match money::decode_decimal(&Value::Null) {
        Err(DbError::UnexpectedNull(what)) => {
            assert_eq!(what, "money column", "载荷是静态列描述");
            eprintln!(
                "[ERR-004] {}: decode_decimal(NULL) → UnexpectedNull(`{what}`)",
                c.dialect.name()
            );
        }
        other => panic!("期望 UnexpectedNull，实际 {other:?}"),
    }

    // 真实场景：可空列 NULL 经 Option 读回是 None（不是 UnexpectedNull）
    let nullable = c
        .db()
        .find_first("SELECT profile FROM users WHERE id = 107")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rowx::opt_json_of(&nullable, "profile").unwrap(), None);
    assert!(
        matches!(
            String::from_value(rowx::value(&nullable, "profile").unwrap()),
            Err(DbError::TypeMismatch { .. })
        ),
        "非 Option 读取 NULL 文本列 → TypeMismatch"
    );
}

// ==================== ERR-005 ====================

/// ERR-005：`source()` 只在未命中映射码的 `Query` 上保留。
#[tokio::test]
async fn err_005_source_chain_retained_for_all_errors() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;

    // 未命中映射码 → Query{sql, source}
    let query_err = c
        .db()
        .execute("SELECT * FROM __no_such_table__")
        .await
        .expect_err("查表失败");
    match &query_err {
        DbError::Query { sql, .. } => {
            assert!(query_err.source().is_some(), "Query 保留底层驱动错误");
            if c.dialect.is_sqlite() {
                assert!(sql.contains("__no_such_table__"), "驱动回填 SQL：{sql}");
            } else {
                eprintln!(
                    "[ERR-005][差异登记] {}: Query.sql = `{sql}`（未回填 SQL 上下文）",
                    c.dialect.name()
                );
            }
        }
        other => panic!("期望 Query，实际 {other:?}"),
    }

    // 命中映射码 → 只留消息，丢弃原始 source
    let unique_err = c
        .db()
        .execute_with_params(
            "INSERT INTO users (id, tenant_id, email, name, status, role, created_at, updated_at, \
             first_name, last_name, profile, locale) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(110),
                Value::I64(TENANT_T1),
                Value::Text("buyer@x.com".into()),
                Value::Text("dupe".into()),
                Value::Text("active".into()),
                Value::Text("member".into()),
                at(0).into(),
                at(0).into(),
                Value::Text("D".into()),
                Value::Text("U".into()),
                Value::Null,
                Value::Text("zh-CN".into()),
            ],
        )
        .await
        .expect_err("重复 email");
    assert!(matches!(unique_err, DbError::UniqueViolation { .. }));
    assert!(
        unique_err.source().is_some(),
        "UniqueViolation 同样保留原始 source（结构化字段 + 原文并存）"
    );
    assert!(
        unique_err
            .to_string()
            .contains("unique constraint violated"),
        "Display 应可读且带结构化信息，实际 `{unique_err}`"
    );

    let fk_err = c
        .db()
        .execute_with_params(
            "INSERT INTO invoice_items (id, invoice_id, description, quantity, unit_price, amount, tax_rate) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Value::I64(900002),
                Value::I64(888_888),
                Value::Text("orphan2".into()),
                Value::I64(1),
                money(1, 0).to_value(),
                money(1, 0).to_value(),
                Value::Text("0".into()),
            ],
        )
        .await
        .expect_err("孤儿条目");
    assert!(matches!(fk_err, DbError::ForeignKeyViolation { .. }));
    assert!(
        fk_err.source().is_some(),
        "ForeignKeyViolation 同样保留原始 source"
    );
    eprintln!(
        "[ERR-005] {}: Query source={}；UniqueViolation source={}；ForeignKeyViolation source={}（三者均保留 source 链）",
        c.dialect.name(),
        query_err.source().is_some(),
        unique_err.source().is_some(),
        fk_err.source().is_some()
    );
}

// ==================== ERR-006 ====================

/// ERR-006：错误消息**没有**修正建议（`did you mean`）——缺陷登记。
#[tokio::test]
async fn err_006_no_correction_suggestion_in_messages() {
    let Some(c) = ctx().await else { return };
    seed_common(&c).await;

    let cases = [
        ("列名拼错", "SELECT emial FROM users"),
        ("表名拼错", "SELECT id FROM usres"),
        ("关键字拼错", "SELCT id FROM users"),
    ];
    for (label, sql_text) in cases {
        let err = c.db().find(sql_text).await.expect_err("必须失败");
        let msg = full_message(&err);
        let lower = msg.to_lowercase();
        assert!(
            !lower.contains("did you mean") && !lower.contains("suggestion"),
            "{label} 的错误消息不得含修正建议（当前框架无该能力）：{msg}"
        );
        assert!(
            matches!(err, DbError::Query { .. }),
            "{label} 应落 Query，实际 {err:?}"
        );
        eprintln!("[ERR-006][缺陷] {} {label}: {msg}", c.dialect.name());
    }

    // 结构化 ColumnNotFound 同样没有建议载荷
    let row = c
        .db()
        .find_first("SELECT id FROM users WHERE id = 107")
        .await
        .unwrap()
        .unwrap();
    match rowx::text_of(&row, "emial") {
        Err(DbError::ColumnNotFound(name)) => {
            assert_eq!(name, "emial", "载荷只有列名，无候选建议");
            assert!(!name.contains("did you mean"));
        }
        other => panic!("期望 ColumnNotFound，实际 {other:?}"),
    }
    eprintln!(
        "[ERR-006][缺陷] {}: DbError::ColumnNotFound 无候选建议字段",
        c.dialect.name()
    );
}

/// 宽容整数读取：PG/MySQL 的 COUNT/SUM 可能读回 `Decimal`/`U64`/`Text`。
fn int_of(row: &alun_db::prelude::Row, col: &str) -> i64 {
    use alun_db_core::stmt::Value;
    match rowx::value(row, col) {
        Ok(Value::I64(n)) => *n,
        Ok(Value::U32(n)) => i64::from(*n),
        Ok(Value::U64(n)) => *n as i64,
        Ok(Value::I32(n)) => i64::from(*n),
        Ok(Value::Text(s)) => s.parse().unwrap_or_default(),
        Ok(Value::Decimal(c)) => {
            rust_decimal::Decimal::try_from_i128_with_scale(c.mantissa, c.scale)
                .map(|d| d.to_string().parse::<f64>().unwrap_or(0.0) as i64)
                .unwrap_or_default()
        }
        other => panic!("无法作为整数读取 `{col}`: {other:?}"),
    }
}

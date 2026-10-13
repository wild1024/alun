//! 多租户行级隔离语义测试。
//!
//! 覆盖三条不用配置即可获得的核心保证：
//! - **越租户读不到**：`Scoped` 上下文下只能看到本租户行
//! - **无上下文 fail-closed**：租户模型在无上下文时直接报错，绝不静默放行全量
//! - **显式跨租户 / 模型级豁免**：`TenantContext::Cross` 与 `#[model(cross_tenant)]` 可用

mod common;
use alun_db::prelude::*;
use common::*;

/// 参与租户隔离的业务模型：`tenant_id` 为普通业务字段（类型无关）。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("invoices")]
pub struct Invoice {
    /// 主键
    #[key]
    pub id: i64,
    /// 标题
    pub title: String,
    /// 租户列：行级隔离依据
    #[tenant]
    pub tenant_id: i64,
}

/// 模型级豁免：全局表（如币种字典），不参与租户隔离。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("currencies")]
#[model(cross_tenant)]
pub struct Currency {
    /// 主键
    #[key]
    pub id: i64,
    /// 币种代码
    pub code: String,
}

/// 参与租户隔离的从表：用于验证**结构化联查**的从表租户注入。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("invoice_lines")]
pub struct InvoiceLine {
    /// 主键
    #[key]
    pub id: i64,
    /// 所属发票主键（与 `invoices.id` 关联）
    pub invoice_id: i64,
    /// 备注
    pub note: String,
    /// 租户列：行级隔离依据
    #[tenant]
    pub tenant_id: i64,
}

/// 建表并写入两个租户的数据（种子数据不经模型，避免写入期补租户逻辑干扰读测试）。
async fn seed(db: &Db) {
    db.execute(
        "CREATE TABLE invoices (
            id INTEGER PRIMARY KEY,
            title TEXT NOT NULL,
            tenant_id INTEGER NOT NULL
        )",
    )
    .await
    .expect("create invoices table");
    db.execute(
        "CREATE TABLE currencies (
            id INTEGER PRIMARY KEY,
            code TEXT NOT NULL
        )",
    )
    .await
    .expect("create currencies table");
    db.execute(
        "CREATE TABLE invoice_lines (
            id INTEGER PRIMARY KEY,
            invoice_id INTEGER NOT NULL,
            note TEXT NOT NULL,
            tenant_id INTEGER NOT NULL
        )",
    )
    .await
    .expect("create invoice_lines table");

    db.execute(
        "INSERT INTO invoices (id, title, tenant_id) VALUES
            (1, 't1-a', 1), (2, 't1-b', 1), (3, 't2-a', 2)",
    )
    .await
    .expect("seed invoices");
    db.execute("INSERT INTO currencies (id, code) VALUES (1, 'CNY'), (2, 'USD')")
        .await
        .expect("seed currencies");
    // 关键：line 11 挂在发票 1 下，但租户是 2 —— 用于验证从表也按租户过滤。
    // 若从表未被过滤，租户 1 会通过发票 1 关联到 line 11（跨租户泄漏）。
    db.execute(
        "INSERT INTO invoice_lines (id, invoice_id, note, tenant_id) VALUES
            (10, 1, 'L1', 1), (11, 1, 'L2', 2), (12, 2, 'L3', 1)",
    )
    .await
    .expect("seed invoice_lines");
}

/// 在指定租户上下文中执行一段查询。
async fn with_tenant<F, T>(tenant: i64, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tenant_scope(TenantContext::Scoped(Value::I64(tenant)), fut).await
}

#[tokio::test]
async fn scoped_reads_only_see_own_tenant() {
    let db = setup_db().await;
    seed(&db).await;

    // 租户 1：query / count / find_by 全部只见本租户
    with_tenant(1, async {
        let rows = Invoice::query().find(&db).await.expect("scoped find");
        assert_eq!(rows.len(), 2, "tenant 1 owns exactly two rows");
        assert!(rows.iter().all(|r| r.tenant_id == 1));

        assert_eq!(Invoice::count(&db).await.expect("scoped count"), 2);

        let by_title = Invoice::find_by(&db, "title LIKE 't%'", ())
            .await
            .expect("scoped find_by");
        assert_eq!(by_title.len(), 2);

        let page = Invoice::query()
            .paginate(&db, 1, 10)
            .await
            .expect("scoped page");
        assert_eq!(page.total, 2);
    })
    .await;

    // 租户 2：仅一行
    with_tenant(2, async {
        let rows = Invoice::query().find(&db).await.expect("scoped find");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "t2-a");
    })
    .await;
}

#[tokio::test]
async fn cross_tenant_pk_lookup_misses() {
    let db = setup_db().await;
    seed(&db).await;

    // 租户 1 按主键查租户 2 的行（id=3）→ 视作不存在
    with_tenant(1, async {
        assert!(
            Invoice::find(&db, 3).await.is_err(),
            "cross-tenant pk lookup must not resolve"
        );
        assert!(
            Invoice::find_or_none(&db, 3)
                .await
                .expect("no error")
                .is_none(),
            "cross-tenant pk lookup must yield None"
        );
        // 自己的行仍可命中
        assert_eq!(Invoice::find(&db, 1).await.expect("own row").id, 1);
    })
    .await;
}

#[tokio::test]
async fn missing_context_fails_closed() {
    let db = setup_db().await;
    seed(&db).await;

    // 无租户上下文：租户模型的任何读路径都必须报错，而非放行全量
    assert!(
        Invoice::query().find(&db).await.is_err(),
        "query must fail-closed"
    );
    assert!(Invoice::count(&db).await.is_err(), "count must fail-closed");
    assert!(
        Invoice::find(&db, 1).await.is_err(),
        "find must fail-closed"
    );
    assert!(
        Invoice::find_by(&db, "id > 0", ()).await.is_err(),
        "find_by must fail-closed"
    );
}

#[tokio::test]
async fn explicit_cross_sees_all() {
    let db = setup_db().await;
    seed(&db).await;

    let all = tenant_scope(TenantContext::Cross, async {
        Invoice::query().find(&db).await.expect("cross find")
    })
    .await;
    assert_eq!(all.len(), 3, "cross context sees every tenant");
}

#[tokio::test]
async fn cross_tenant_model_is_exempt() {
    let db = setup_db().await;
    seed(&db).await;

    // `#[model(cross_tenant)]` 模型无需任何上下文即可全量读取
    let rows = Currency::query().find(&db).await.expect("exempt find");
    assert_eq!(rows.len(), 2);
    assert_eq!(Currency::count(&db).await.expect("exempt count"), 2);
}

#[tokio::test]
async fn insert_auto_fills_tenant() {
    let db = setup_db().await;
    seed(&db).await;

    // 占位租户值（0）在 Scoped 上下文中被补齐为上下文租户
    tenant_scope(TenantContext::Scoped(Value::I64(7)), async {
        let mut inv = Invoice {
            id: 100,
            title: "new".into(),
            tenant_id: 0,
        };
        inv.insert(&db).await.expect("scoped insert");
        assert_eq!(inv.tenant_id, 7, "model tenant must be backfilled");
    })
    .await;

    let stored: i64 = db
        .scalar_with_params(
            "SELECT tenant_id FROM invoices WHERE id = ?",
            vec![Value::I64(100)],
        )
        .await
        .expect("read stored tenant");
    assert_eq!(stored, 7, "row must be persisted under the context tenant");
}

#[tokio::test]
async fn insert_rejects_forged_tenant() {
    let db = setup_db().await;
    seed(&db).await;

    // 行内租户值与上下文不一致 → 拒绝写入（防越权）
    with_tenant(1, async {
        let mut forged = Invoice {
            id: 101,
            title: "forged".into(),
            tenant_id: 2,
        };
        assert!(
            forged.insert(&db).await.is_err(),
            "a forged tenant value must be rejected"
        );
    })
    .await;

    // 无上下文插入同样 fail-closed
    let mut orphan = Invoice {
        id: 102,
        title: "no-ctx".into(),
        tenant_id: 0,
    };
    assert!(
        orphan.insert(&db).await.is_err(),
        "insert without a tenant context must fail-closed"
    );
}

#[tokio::test]
async fn insert_many_fills_tenant() {
    let db = setup_db().await;
    seed(&db).await;

    let rows = vec![
        Invoice {
            id: 200,
            title: "b1".into(),
            tenant_id: 0,
        },
        Invoice {
            id: 201,
            title: "b2".into(),
            tenant_id: 0,
        },
    ];
    // `insert_many` 不触发 Hook，租户补齐在渲染链内完成
    with_tenant(5, async {
        Invoice::insert_many(&rows, &db)
            .await
            .expect("batch insert");
    })
    .await;

    let count: i64 = db
        .scalar_with_params(
            "SELECT COUNT(*) FROM invoices WHERE tenant_id = ?",
            vec![Value::I64(5)],
        )
        .await
        .expect("count tenant 5");
    assert_eq!(count, 2, "every batch row must carry the context tenant");
}

#[tokio::test]
async fn update_and_delete_are_scoped() {
    let db = setup_db().await;
    seed(&db).await;

    with_tenant(1, async {
        // 跨租户整行更新（id=3 属于租户 2）→ NotFound
        let foreign = Invoice {
            id: 3,
            title: "hijack".into(),
            tenant_id: 1,
        };
        assert!(
            foreign.update(&db).await.is_err(),
            "cross-tenant update must not affect rows"
        );

        // 把本租户的行改到别的租户 → 拒绝
        let mover = Invoice {
            id: 1,
            title: "moved".into(),
            tenant_id: 2,
        };
        assert!(
            mover.update(&db).await.is_err(),
            "moving a row across tenants must be rejected"
        );

        // 本租户整行更新生效
        let own = Invoice {
            id: 1,
            title: "renamed".into(),
            tenant_id: 1,
        };
        own.update(&db).await.expect("own update");

        // 跨租户删除 → 0 行受影响
        let foreign_del = Invoice {
            id: 3,
            title: "x".into(),
            tenant_id: 1,
        };
        assert_eq!(
            foreign_del.delete(&db).await.expect("cross delete"),
            0,
            "cross-tenant delete must affect no rows"
        );
    })
    .await;

    // 校验：租户 2 的行未被改动或删除
    let title: String = db
        .scalar_with_params(
            "SELECT title FROM invoices WHERE id = ?",
            vec![Value::I64(3)],
        )
        .await
        .expect("row 3 survives");
    assert_eq!(title, "t2-a");
}

#[tokio::test]
async fn changeset_update_is_scoped() {
    let db = setup_db().await;
    seed(&db).await;

    with_tenant(1, async {
        // 跨租户：id=3 属于租户 2 → 0 行受影响
        let hijack = Invoice::of(3)
            .set(Invoice::title(), "hijack")
            .update(&db)
            .await
            .expect("changeset cross update");
        assert_eq!(hijack, 0);

        // 本租户：id=1 → 1 行受影响
        let own = Invoice::of(1)
            .set(Invoice::title(), "mine")
            .update(&db)
            .await
            .expect("changeset own update");
        assert_eq!(own, 1);
    })
    .await;
}

#[tokio::test]
async fn raw_sql_requires_cross_context() {
    let db = setup_db().await;
    seed(&db).await;

    // Scoped 上下文下原生 SQL 无法自动追加租户条件 → 拒绝
    with_tenant(1, async {
        assert!(
            Invoice::sql(sql!("SELECT * FROM invoices"))
                .find(&db)
                .await
                .is_err(),
            "raw SQL on a tenant model must be rejected under a scoped context"
        );
    })
    .await;

    // 显式跨租户后放行
    let rows = tenant_scope(TenantContext::Cross, async {
        Invoice::sql(sql!("SELECT * FROM invoices"))
            .find(&db)
            .await
            .expect("raw SQL allowed under cross context")
    })
    .await;
    assert_eq!(rows.len(), 3);

    // 非租户模型的原始 SQL 不受影响
    let currencies = Currency::sql(sql!("SELECT * FROM currencies"))
        .find(&db)
        .await
        .expect("cross-tenant model raw SQL");
    assert_eq!(currencies.len(), 2);
}

// ═══════════════════════════════════════════════════════════════════════
// 原生 SQL 的租户绑定通道（`/*tenant*/` 占位 + 显式完整性承诺）
//
// 背景：租户模型的原生 SQL 原先被一律拒绝，导致多租户下 JOIN / 聚合报表
// 完全无法实现。现在改为「占位 + 显式承诺」两步放行：
//   1. 用 `/*tenant:别名*/` 标注**每一张**租户表（框架据此注入条件）；
//   2. 调用 `.raw_tenant_annotated()` 显式承诺"已标注完整"。
//
// 为什么强制第二步：框架无法从任意 SQL 文本推断是否漏标——只标注主表、
// 漏标 JOIN 从表时，从表不过滤但守卫会因主表占位而放行 = 静默跨租户泄漏。
// 把"完整性"从框架的隐式假设提升为调用点可见、可 grep 的显式声明。
//
// 结构化联查（`ModelQuery::join::<U>`）由框架枚举从表模型，**不存在漏标风险**，
// 应优先使用；见文末 `structured_join_*` 用例。
//
// 占位展开为**独立谓词**（带括号），需自行用 `AND` 连接。
// ═══════════════════════════════════════════════════════════════════════

/// Scoped 上下文 + 租户占位 + 显式承诺 → 只返回本租户数据。
#[tokio::test]
async fn raw_sql_tenant_marker_scopes_reads() {
    let db = setup_db().await;
    seed(&db).await;

    with_tenant(1, async {
        let rows = Invoice::sql(sql!(
            "SELECT * FROM invoices WHERE 1 = 1 AND /*tenant*/ ORDER BY id"
        ))
        .raw_tenant_annotated()
        .find(&db)
        .await
        .expect("带占位 + 显式承诺的原生 SQL 应被放行");

        assert_eq!(rows.len(), 2, "tenant 1 应只见自己的两行：{rows:?}");
        assert!(rows.iter().all(|r| r.tenant_id == 1));
    })
    .await;
}

/// 带别名的占位（JOIN 场景）：**每个别名都必须被过滤**。
///
/// 用 `ON 1 = 1` 制造笛卡尔积：若只过滤了 `a` 而漏了 `b`，
/// 租户 1 会看到 2×3=6 行；两者都过滤才是 2×2=4 行。
#[tokio::test]
async fn raw_sql_tenant_marker_filters_every_alias() {
    let db = setup_db().await;
    seed(&db).await;

    with_tenant(1, async {
        let rows = Invoice::sql(sql!(
            "SELECT a.id, a.title, a.tenant_id
               FROM invoices a
               JOIN invoices b ON 1 = 1
              WHERE 1 = 1
                AND /*tenant:a*/
                AND /*tenant:b*/"
        ))
        .raw_tenant_annotated()
        .find(&db)
        .await
        .expect("双别名均标注占位 → 放行");

        assert_eq!(
            rows.len(),
            4,
            "a 与 b 都必须按租户过滤（2×2=4），实际 {} 行说明有别名漏过滤",
            rows.len()
        );
        assert!(rows.iter().all(|r| r.tenant_id == 1));
    })
    .await;
}

/// 占位替换后，**既有绑定参数的顺序必须保持正确**。
///
/// 断言依据：占位注入的租户值若错位，就会与 title / id 比较，结果必然异常。
/// 这里在占位**前后各放一个绑定参数**，覆盖「插入位置在序列中间」的情形。
#[tokio::test]
async fn raw_sql_tenant_marker_preserves_bind_order() {
    let db = setup_db().await;
    seed(&db).await;

    let excluded = "t1-a".to_string();
    let min_id = 0i64;

    with_tenant(1, async {
        let rows = Invoice::sql(sql!(
            "SELECT * FROM invoices WHERE title <> ", { excluded },
            " AND /*tenant*/ AND id >= ", { min_id },
            " ORDER BY id"
        ))
        .raw_tenant_annotated()
        .find(&db)
        .await
        .expect("绑定顺序与占位混用时也应放行");

        assert_eq!(rows.len(), 1, "租户 1 排除 t1-a 后只剩 1 行：{rows:?}");
        assert_eq!(rows[0].title, "t1-b");
    })
    .await;
}

/// 显式跨租户 + 占位 → 占位不产生过滤（语义等价于恒真）。
#[tokio::test]
async fn raw_sql_tenant_marker_under_cross_is_unfiltered() {
    let db = setup_db().await;
    seed(&db).await;

    let rows = tenant_scope(TenantContext::Cross, async {
        Invoice::sql(sql!("SELECT * FROM invoices WHERE 1 = 1 AND /*tenant*/"))
            .find(&db)
            .await
            .expect("跨租户 + 占位应放行")
    })
    .await;

    assert_eq!(rows.len(), 3, "跨租户下占位不应过滤任何行");
}

/// 字符串字面量里的 `/*tenant*/` 只是普通文本，**不得**被当作占位。
///
/// 若扫描器不区分引号内内容，会把该字符串替换成谓词并注入租户值，
/// 从而静默改写业务语义。此处预期：没有被识别出占位 → 守卫拒绝。
#[tokio::test]
async fn tenant_marker_inside_string_literal_is_not_a_marker() {
    let db = setup_db().await;
    seed(&db).await;

    with_tenant(1, async {
        let result = Invoice::sql(sql!(
            "SELECT * FROM invoices WHERE title <> '/*tenant*/'"
        ))
        .find(&db)
        .await;

        assert!(
            result.is_err(),
            "引号内的 /*tenant*/ 不是占位，应因缺少真实租户绑定而被守卫拒绝"
        );
    })
    .await;
}

/// 占位语法错误必须报错，而不是被当作普通注释放过。
#[tokio::test]
async fn malformed_tenant_marker_is_rejected() {
    let db = setup_db().await;
    seed(&db).await;

    with_tenant(1, async {
        // 别名前有空格
        let result = Invoice::sql(sql!(
            "SELECT * FROM invoices WHERE 1 = 1 AND /*tenant: a */"
        ))
        .find(&db)
        .await;
        assert!(result.is_err(), "非法占位语法应报错");
    })
    .await;
}

/// 未标注占位的原生 SQL 在 Scoped 上下文下**仍然被拒绝**（fail-closed 未松动）。
#[tokio::test]
async fn raw_sql_without_marker_is_still_rejected_under_scope() {
    let db = setup_db().await;
    seed(&db).await;

    with_tenant(1, async {
        let err = Invoice::sql(sql!("SELECT * FROM invoices"))
            .find(&db)
            .await
            .expect_err("无占位的原生 SQL 必须被拒绝");
        let msg = format!("{err}");
        assert!(
            msg.contains("not verifiably bound") || msg.contains("raw_tenant_annotated"),
            "错误信息应提示如何安全化（结构化联查 / 占位 + 显式承诺），实际：{msg}"
        );
    })
    .await;
}

// ═══════════════════════════════════════════════════════════════════════
// 安全回归：部分标注不得再被静默放行（修复跨租户泄漏）
// ═══════════════════════════════════════════════════════════════════════

/// **只标注部分租户表**、且未显式承诺完整性 → 必须拒绝。
///
/// 这是修复前的真实漏洞：早期守卫只要看到**任意一个**占位就放行，
/// 于是下面这条 SQL（标了 `a` 漏了 `b`）会让 `b` 的行完全不过滤却通过守卫。
/// 现在要求显式 `raw_tenant_annotated()`，未承诺者一律拒绝——漏洞在
/// 「机制层」被关闭，而不是依赖使用者自觉。
#[tokio::test]
async fn raw_sql_partial_annotation_is_rejected_without_assertion() {
    let db = setup_db().await;
    seed(&db).await;

    with_tenant(1, async {
        let result = Invoice::sql(sql!(
            "SELECT a.id, a.title, a.tenant_id
               FROM invoices a
               JOIN invoices b ON 1 = 1
              WHERE 1 = 1
                AND /*tenant:a*/" // ← 漏标 b，且未调用 raw_tenant_annotated()
        ))
        .find(&db)
        .await;

        let err = result.expect_err("部分标注 + 未承诺完整性 → 必须拒绝");
        let msg = format!("{err}");
        assert!(
            msg.contains("not verifiably bound"),
            "应给出可行动的错误（结构化联查 / 显式承诺），实际：{msg}"
        );
    })
    .await;
}

/// 显式承诺后，部分标注被**视为调用方已核对**（放行）。
///
/// 框架无法校验 SQL 的完备性，故 `raw_tenant_annotated()` 是"责任转移"：
/// 调用方盖章负责。本用例锁定这一语义，避免将来有人误以为框架会自动兜底。
#[tokio::test]
async fn raw_sql_partial_annotation_is_allowed_with_assertion() {
    let db = setup_db().await;
    seed(&db).await;

    with_tenant(1, async {
        let rows = Invoice::sql(sql!(
            "SELECT a.id, a.title, a.tenant_id
               FROM invoices a
               JOIN invoices b ON 1 = 1
              WHERE 1 = 1
                AND /*tenant:a*/"
        ))
        .raw_tenant_annotated()
        .find(&db)
        .await
        .expect("显式承诺后放行（完整性由调用方负责）");

        // 6 行 = a 被过滤到 2 行，b 未过滤（3 行）→ 2×3
        assert_eq!(rows.len(), 6, "b 未标注 → 保留全部 3 行：{rows:?}");
    })
    .await;
}

// ═══════════════════════════════════════════════════════════════════════
// 结构化联查：从表租户由框架注入（**不会漏标**的安全通道）
// ═══════════════════════════════════════════════════════════════════════

/// `ModelQuery::join::<U>` 会为声明了 `#[tenant]` 的从表自动追加租户条件。
///
/// 数据设计（关键在 line 11）：发票 1 属于租户 1，但它的 line 11 属于租户 2。
/// - 若从表**被过滤**：租户 1 只能通过发票 1 关联到 line 10 → 2 行（inv1↔10, inv2↔12）；
/// - 若从表**未过滤**：还会关联到 line 11 → 3 行（跨租户泄漏）。
#[tokio::test]
async fn structured_join_injects_tenant_on_joined_table() {
    let db = setup_db().await;
    seed(&db).await;

    let on = Expr::Binary {
        left: Box::new(Expr::Column {
            table: "invoices",
            column: "id",
        }),
        op: BinaryOp::Eq,
        right: Box::new(Expr::Column {
            table: "invoice_lines",
            column: "invoice_id",
        }),
    };

    with_tenant(1, async {
        let rows = Invoice::query()
            .join::<InvoiceLine>(JoinKind::Inner, on)
            .find(&db)
            .await
            .expect("结构化联查在租户上下文内应放行");

        assert_eq!(
            rows.len(),
            2,
            "从表 invoice_lines 必须按租户过滤（2 行）；3 行说明发生了跨租户泄漏：{rows:?}"
        );
        assert!(rows.iter().all(|r| r.tenant_id == 1));
    })
    .await;
}

/// 结构化联查在**无租户上下文**下 fail-closed（错误推迟到执行期上报）。
#[tokio::test]
async fn structured_join_without_tenant_context_fails_closed() {
    let db = setup_db().await;
    seed(&db).await;

    let on = Expr::Binary {
        left: Box::new(Expr::Column {
            table: "invoices",
            column: "id",
        }),
        op: BinaryOp::Eq,
        right: Box::new(Expr::Column {
            table: "invoice_lines",
            column: "invoice_id",
        }),
    };

    // 注意：不在 with_tenant 内——无上下文
    let err = Invoice::query()
        .join::<InvoiceLine>(JoinKind::Inner, on)
        .find(&db)
        .await
        .expect_err("无租户上下文的结构化联查必须 fail-closed");
    assert!(
        format!("{err}").contains("tenant"),
        "错误应指明缺少租户上下文，实际：{err}"
    );
}

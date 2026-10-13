//! M4 多租户隔离（用例 TEN-001..TEN-007）。
//!
//! 框架**不提供**自动租户注入（缺口 G1/G2：无 `#[alun::tenant_scope]`、
//! 无请求级租户上下文）。项目侧的约定是：JWT 的 `sub` 形如 `u{user}:t{tenant}`，
//! 端点统一经 `api::identity_of` 取租户，服务层每个查询显式追加 `tenant_id`。
//!
//! L3（真实中间件链 + HTTP）由 `e2e_*` 用例覆盖；本文件在**服务层 + 端点纯函数**
//! 上验证同一不变量（不需要全局 `db()`，可并行）。

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use alun::{TokenClaims, TokenType};
use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use rust_decimal::Decimal;

use alun_ledger::api::identity_of;
use alun_ledger::models::{Invoice, InvoiceStatus};
use alun_ledger::services::{
    error::map_db_err, invoice as invoice_svc, ledger, tenant as tenant_svc,
};

use common::assertions::{assert_tenant_scope, count_by};
use common::fixtures::*;
use common::harness::{Dialect, ctx};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 构造 JWT 声明（`sub = u{user}:t{tenant}`）。
fn claims(sub: &str) -> TokenClaims {
    TokenClaims {
        jti: None,
        sub: sub.to_string(),
        username: None,
        roles: Vec::new(),
        permissions: Vec::new(),
        token_type: Some(TokenType::Access),
        exp: 4_000_000_000,
        iat: 0,
        extra: Default::default(),
    }
}

// ─────────────────────────── TEN-001 ───────────────────────────

/// TEN-001：T1 读 T2 的发票/转账 → `NotFound`（不泄漏存在性），端点映射为 404。
#[tokio::test]
async fn ten_001_cross_tenant_read_is_not_found() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_tenant(&c, TENANT_T2, "T2").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(100, 0)).await.unwrap();
    seed_accounts(&c, TENANT_T2, money(100, 0)).await.unwrap();
    seed_invoice(&c, TENANT_T1, 1001, "draft").await.unwrap();
    seed_invoice(&c, TENANT_T2, 2001, "draft").await.unwrap();

    // T2 的一笔转账
    ledger::post_transfer(
        c.db(),
        ledger::TransferCommand {
            transfer_id: 2002,
            debit_entry_id: 20021,
            credit_entry_id: 20022,
            tenant_id: TENANT_T2,
            from_account: account_id(TENANT_T2, 1),
            to_account: account_id(TENANT_T2, 2),
            amount: money(1, 0),
            at: at(0),
        },
    )
    .await
    .expect("T2 transfer");

    // 发票：跨租户详情读 → NotFound（而非 T2 的行）
    let err = invoice_svc::load(c.db(), TENANT_T1, 2001)
        .await
        .expect_err("跨租户读必须失败");
    assert!(matches!(err, DbError::NotFound), "实际 {err:?}");
    assert_eq!(
        map_db_err(&err).status,
        404,
        "端点等价路径：NotFound → 404（不泄漏存在性）"
    );

    // 转账：同样 404 语义
    let err = ledger::load(c.db(), TENANT_T1, 2002)
        .await
        .expect_err("跨租户读转账必须失败");
    assert!(matches!(err, DbError::NotFound), "实际 {err:?}");
    assert_eq!(map_db_err(&err).status, 404);

    // 本租户可读（对照组，证明不是「一律 404」）
    assert_eq!(
        invoice_svc::load(c.db(), TENANT_T1, 1001).await.unwrap().id,
        1001
    );
    assert_eq!(
        ledger::load(c.db(), TENANT_T2, 2002).await.unwrap().id,
        2002
    );

    // 金额/账户跨租户读也被作用域挡住
    let err =
        alun_ledger::services::money::account_balance(c.db(), TENANT_T1, account_id(TENANT_T2, 1))
            .await
            .expect_err("跨租户账户余额必须失败");
    assert!(matches!(err, DbError::NotFound), "实际 {err:?}");
}

// ─────────────────────────── TEN-002 ───────────────────────────

/// TEN-002：列表接口只返回本租户行；库内无越租户数据。
#[tokio::test]
async fn ten_002_lists_are_scoped_and_scope_scan_clean() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_accounts(&c, TENANT_T1, money(500, 0)).await.unwrap();
    for (i, id) in [1001, 1002, 1003].iter().enumerate() {
        seed_invoice(&c, TENANT_T1, *id, "draft").await.unwrap();
        seed_user(&c, TENANT_T1, 500 + i as i64, &format!("u{i}@t1.test"))
            .await
            .unwrap();
    }
    seed_subscription(&c, TENANT_T1, 7001, at(0), at(3600), "active")
        .await
        .unwrap();

    // 只有 T1 数据时，作用域扫描必须全表零越界
    assert_tenant_scope(c.db(), TENANT_T1).await;

    // 列表接口（服务层等价）只含 T1
    let invoices = tenant_svc::list_invoices(c.db(), TENANT_T1).await.unwrap();
    assert_eq!(
        invoices.iter().map(|i| i.id).collect::<Vec<_>>(),
        vec![1001, 1002, 1003]
    );
    assert!(invoices.iter().all(|i| i.tenant_id == TENANT_T1));
    let users = tenant_svc::list_users(c.db(), TENANT_T1).await.unwrap();
    assert_eq!(users.len(), 3);
    assert!(users.iter().all(|u| u.tenant_id == TENANT_T1));
    assert_eq!(
        tenant_svc::list_subscriptions(c.db(), TENANT_T1)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        tenant_svc::account_ids(c.db(), TENANT_T1)
            .await
            .unwrap()
            .len(),
        5
    );
    assert_eq!(
        tenant_svc::count_invoices(c.db(), TENANT_T1).await.unwrap(),
        3
    );

    // 其它租户的列表为空（不是「返回全部」）
    assert!(
        tenant_svc::list_invoices(c.db(), TENANT_T2)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        tenant_svc::count_invoices(c.db(), TENANT_T2).await.unwrap(),
        0
    );
    assert!(
        tenant_svc::account_ids(c.db(), TENANT_T2)
            .await
            .unwrap()
            .is_empty()
    );
}

// ─────────────────────────── TEN-003 ───────────────────────────

/// TEN-003：管理员入口显式跨租户；普通入口仍被作用域限制。
#[tokio::test]
async fn ten_003_admin_path_sees_all_tenants() {
    let Some(c) = ctx().await else { return };
    for (tid, name) in [(TENANT_T1, "T1"), (TENANT_T2, "T2"), (TENANT_T3, "T3")] {
        seed_tenant(&c, tid, name).await.unwrap();
        seed_accounts(&c, tid, money(10, 0)).await.unwrap();
        seed_invoice(&c, tid, tid * 1000 + 1, "draft")
            .await
            .unwrap();
    }

    // 管理员：全部租户
    let all = tenant_svc::admin_list_tenants(c.db()).await.unwrap();
    assert_eq!(
        all.iter().map(|t| t.id).collect::<BTreeSet<_>>(),
        [TENANT_T1, TENANT_T2, TENANT_T3].into_iter().collect()
    );
    // 管理员：跨租户聚合
    assert_eq!(tenant_svc::admin_invoice_total(c.db()).await.unwrap(), 3);

    // 普通用户路径：只见本租户（与 admin 对照）
    assert_eq!(
        tenant_svc::count_invoices(c.db(), TENANT_T2).await.unwrap(),
        1
    );
    assert_eq!(
        tenant_svc::list_invoices(c.db(), TENANT_T2)
            .await
            .unwrap()
            .iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        vec![TENANT_T2 * 1000 + 1]
    );

    // 非法身份 → 401（无租户可用）
    assert_eq!(identity_of(&claims("garbage")).unwrap_err().status, 401);
}

// ─────────────────────────── TEN-004 ───────────────────────────

/// TEN-004：多租户并发访问互不串扰（缩放为 20 租户）。
///
/// 规划为 100 租户 / 池 50，属 pg/mysql 规模；SQLite 内存库池容量为 1，
/// 因此这里缩放到 20 个租户，断言「每个租户只看到/改到自己那一行」。
#[tokio::test]
async fn ten_004_scaled_multi_tenant_non_interference() {
    let Some(c) = ctx().await else { return };
    const N: i64 = 20;

    for t in 1..=N {
        seed_tenant(&c, t, &format!("T{t}")).await.unwrap();
        seed_accounts(&c, t, money(1000 + t, 0)).await.unwrap();
    }

    let mut handles = Vec::new();
    for t in 1..=N {
        let db = c.db().clone();
        handles.push(tokio::spawn(async move {
            let acct = account_id(t, 1);
            // 读：只应看到自己租户的余额
            let balance = alun_ledger::services::money::account_balance(&db, t, acct)
                .await
                .expect("tenant balance");
            // 写：带租户作用域的 UPDATE
            let affected = db
                .execute_with_params(
                    "UPDATE accounts SET name = ? WHERE id = ? AND tenant_id = ?",
                    vec![
                        Value::Text(format!("t{t}-renamed")),
                        Value::I64(acct),
                        Value::I64(t),
                    ],
                )
                .await
                .expect("scoped update");
            let seen: String = db
                .scalar_with_params::<String>(
                    "SELECT name FROM accounts WHERE tenant_id = ? AND id = ?",
                    vec![Value::I64(t), Value::I64(acct)],
                )
                .await
                .expect("read back");
            (t, balance, affected, seen)
        }));
    }

    let mut balances = Vec::new();
    for h in handles {
        let (t, balance, affected, seen) = h.await.expect("task must not panic");
        assert_eq!(affected, 1, "租户 {t} 的有作用域更新应恰好命中 1 行");
        assert_eq!(seen, format!("t{t}-renamed"), "租户 {t} 读回自己的写入");
        balances.push((t, balance));
    }
    for (t, balance) in balances {
        assert_eq!(
            alun_ledger::services::money::money_str(balance),
            money(1000 + t, 0).normalize().to_string(),
            "租户 {t} 余额被串改"
        );
    }

    // 全局校验：改名恰好 N 行（每租户 1 行），且每租户只拥有自己的那一行
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM accounts WHERE name LIKE 't%-renamed'",
            vec![]
        )
        .await,
        N,
        "跨租户写入次数应等于租户数"
    );
    for t in 1..=N {
        assert_eq!(
            count_by(
                c.db(),
                "SELECT COUNT(*) FROM accounts WHERE tenant_id = ? AND name LIKE 't%-renamed'",
                vec![Value::I64(t)]
            )
            .await,
            1,
            "租户 {t} 应恰好有 1 行被改"
        );
        assert_eq!(
            count_by(
                c.db(),
                "SELECT COUNT(*) FROM accounts WHERE tenant_id = ?",
                vec![Value::I64(t)]
            )
            .await,
            5,
            "租户 {t} 账户数不被他人影响"
        );
    }
    eprintln!("[TEN-004] {N} 租户并发读/写无串扰（SQLite 池容量 1，非真并发）");
}

// ─────────────────────────── TEN-005 ───────────────────────────

/// TEN-005：静态守卫——`src/api/**.rs` 中每个查询调用点都必须携带租户 id
/// （或被显式登记为跨租户入口）。
///
/// 局限（coarse textual check）：只看**行文本 + 后 4 行窗口**，不做数据流分析；
/// 服务函数内部的 SQL 不在此覆盖范围内（由 TEN-001/002 的运行时断言覆盖）；
/// 经结构体字段（`TransferCommand.tenant_id`、`Invoice.tenant_id`）传递的租户
/// 无法被文本识别，故列入显式例外表——这些例外即本仓库的 `#[cross_tenant]` 等价物。
#[test]
fn ten_005_static_guard_api_queries_are_tenant_scoped() {
    /// 需要携带租户维度的调用点特征。
    const NEEDLES: [&str; 26] = [
        "SELECT",
        "list_users(",
        "list_invoices(",
        "list_subscriptions(",
        "account_ids(",
        "count_invoices(",
        "invoice::load(",
        "ledger::load(",
        "account_balance(",
        "mrr_by_month(",
        "transition(",
        "post_transfer(",
        "create_draft(",
        "admin_list_tenants(",
        "admin_invoice_total(",
        "due_subscriptions(",
        // 查询调用点（2026-09 重构后的命名族）。
        //
        // 刻意不写裸 `find(`：那会误伤 `Iterator::find(|…| …)`（如本目录
        // `tenants.rs` / `subscriptions.rs` 在内存里找一月/一租户的写法）。
        // 数据库查询一律以执行器作实参，故按「调用形态 + 执行器」匹配：
        "::find(",    // 模型静态查询：`Tenant::find(db(), id)`
        ".find(db()", // sql!/ModelQuery 终态：`sql!(…).find(db())`
        ".find(&tx",  // 同上，事务内
        "find_by(",   // where_or_field 查询族
        "find_first_by(",
        "find_first(",
        "find_one(",
        "find_one_or_null(",
        "find_or_none(",
        "find_in(",
    ];
    /// 显式跨租户 / 无租户维度的例外（文件名, 行内片段, 理由）。
    const ALLOWED: [(&str, &str, &str); 6] = [
        (
            "subscriptions.rs",
            "admin_list_tenants(",
            "管理员跨租户报表入口",
        ),
        (
            "subscriptions.rs",
            "due_subscriptions(",
            "全局到期扫描（系统任务）",
        ),
        (
            "subscriptions.rs",
            "\"SELECT 1\"",
            "/health 探活，无租户维度",
        ),
        ("tenants.rs", "::find(", "租户根实体自身（无 tenant_id 列）"),
        (
            "invoices.rs",
            "create_draft(",
            "实体 Invoice 自带 tenant_id",
        ),
        (
            "transfers.rs",
            "post_transfer(",
            "命令 TransferCommand 自带 tenant_id",
        ),
    ];

    let api_dir = manifest_dir().join("src").join("api");
    let mut checked = 0usize;
    let mut allowed_hits = 0usize;
    for entry in std::fs::read_dir(&api_dir).expect("src/api") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path).expect("read api file");
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            // 跳过函数定义行（形参列表里当然没有 tenant_id）
            if trimmed.starts_with("pub async fn ")
                || trimmed.starts_with("async fn ")
                || trimmed.starts_with("fn ")
                || trimmed.starts_with("//")
            {
                continue;
            }
            if !NEEDLES.iter().any(|n| line.contains(n)) {
                continue;
            }
            if let Some((_f, _snippet, reason)) = ALLOWED
                .iter()
                .find(|(f, snippet, _)| *f == file_name && line.contains(snippet))
            {
                allowed_hits += 1;
                eprintln!("[TEN-005-allow] {file_name}:{} 豁免（{reason}）", i + 1);
                continue;
            }
            let window_end = (i + 5).min(lines.len());
            let window = lines[i..window_end].join("\n");
            assert!(
                window.contains("tenant_id"),
                "{file_name}:{} 查询调用点未携带租户 id（无 #[tenant_scope]，框架不会自动注入）:\n{}",
                i + 1,
                window
            );
            checked += 1;
        }
    }

    assert!(
        checked >= 10,
        "扫描到的受作用域调用点只有 {checked} 个，守卫可能已失效"
    );
    assert_eq!(
        allowed_hits,
        ALLOWED.len(),
        "例外表存在失效条目（命中 {allowed_hits}/{}）——被豁免的调用点必须仍真实存在",
        ALLOWED.len()
    );
    eprintln!(
        "[TEN-005] 扫描 {checked} 个受作用域调用点全部携带 tenant_id；\
         {allowed_hits} 个显式跨租户例外（共 {} 条登记）",
        ALLOWED.len()
    );
}

// ─────────────────────────── TEN-006 ───────────────────────────

/// TEN-006：租户级 schema 变更（PG schema-per-tenant）——SQLite 不适用。
#[tokio::test]
async fn ten_006_schema_per_tenant_not_applicable_on_sqlite() {
    let Some(c) = ctx().await else { return };
    if !c.require(Dialect::Postgres) {
        eprintln!(
            "[TEN-006] {} 无 schema-per-tenant 概念（SQLite 单库单命名空间 / MySQL schema==database）→ 跳过",
            c.dialect.name()
        );
        return;
    }

    // PG：T1 的 schema 变更不影响 T2（T2 建在 public）
    for tenant in ["tenant_t1", "tenant_t2"] {
        c.db()
            .execute(&format!("CREATE SCHEMA IF NOT EXISTS {tenant}"))
            .await
            .expect("create schema");
    }
    c.db()
        .execute("CREATE TABLE IF NOT EXISTS tenant_t1.orders (id BIGINT NOT NULL PRIMARY KEY)")
        .await
        .expect("create t1 table");
    c.db()
        .execute("CREATE INDEX IF NOT EXISTS idx_t1_orders ON tenant_t1.orders (id)")
        .await
        .expect("create t1 index");

    // T2 的 schema 不含 T1 的表
    let rows = c
        .db()
        .find_with_params(
            "SELECT table_name FROM information_schema.tables WHERE table_schema = ?",
            vec![Value::Text("tenant_t2".into())],
        )
        .await
        .expect("list t2 tables");
    assert!(
        !rows.iter().any(|r| {
            r.get("table_name")
                .map(|v| format!("{v:?}").contains("orders"))
                .unwrap_or(false)
        }),
        "T2 schema 不应出现 T1 的表"
    );
}

// ─────────────────────────── TEN-007 ───────────────────────────

/// TEN-007：请求体/路径里的 `tenant_id` 被忽略，一律以 JWT 派生值为准。
#[tokio::test]
async fn ten_007_forged_tenant_id_is_ignored_in_favour_of_jwt() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_tenant(&c, TENANT_T2, "T2").await.unwrap();

    // JWT 决定租户
    let (user_id, tenant_id) = identity_of(&claims("u7:t1")).expect("valid sub");
    assert_eq!((user_id, tenant_id), (7, TENANT_T1));

    // 请求体伪造 tenant_id = 2（端点从不读取该字段，服务层用 JWT 派生值）
    let forged_body_tenant_id = TENANT_T2;
    assert_ne!(
        forged_body_tenant_id, tenant_id,
        "伪造值必须与 JWT 派生值不同，否则用例无效"
    );
    let inv = Invoice {
        id: 9001,
        tenant_id, // ← 来自 identity_of，而非请求体
        customer_id: 1,
        status: InvoiceStatus::Draft,
        total: Decimal::ZERO,
        metadata: None,
        issued_at: None,
        paid_at: None,
        created_at: at(0),
        note: None,
    };
    invoice_svc::create_draft(c.db(), inv, &[], 0)
        .await
        .expect("create draft");

    // 落在 T1；T2 看不到、库内也没有 tenant_id=2 的该行
    assert_eq!(
        invoice_svc::load(c.db(), TENANT_T1, 9001).await.unwrap().id,
        9001
    );
    assert!(matches!(
        invoice_svc::load(c.db(), TENANT_T2, 9001).await,
        Err(DbError::NotFound)
    ));
    assert_eq!(
        count_by(
            c.db(),
            "SELECT COUNT(*) FROM invoices WHERE id = 9001 AND tenant_id = ?",
            vec![Value::I64(TENANT_T2)]
        )
        .await,
        0
    );

    // 路径/参数不能改变租户：T1 的 JWT 用 T2 的账户 id 也读不到
    let err =
        alun_ledger::services::money::account_balance(c.db(), tenant_id, account_id(TENANT_T2, 1))
            .await
            .expect_err("跨租户账户 id 必须 404");
    assert!(matches!(err, DbError::NotFound));

    // 畸形 sub → 401（不给默认租户）
    for bad in ["garbage", "u7:t1:junk", "u:t1", "u7:t"] {
        let e = identity_of(&claims(bad)).expect_err("malformed sub must be rejected");
        assert_eq!(e.status, 401, "sub=`{bad}` 应 401，实际 {e:?}");
    }

    // 源码守卫：请求体结构体不得携带 tenant_id；读请求体的 handler 必须经 identity_of
    let api_dir = manifest_dir().join("src").join("api");
    let mut json_handlers = 0usize;
    for entry in std::fs::read_dir(&api_dir).expect("src/api") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        let text = std::fs::read_to_string(&path).expect("read api file");
        assert!(
            !text.contains("req.tenant_id"),
            "{file_name} 从请求体读取 tenant_id（应以 JWT 声明为准）"
        );

        // 按 handler 切分：读 Json/Path/Query 的 handler 必须在体内调用 identity_of
        let chunks: Vec<&str> = text.split("pub async fn ").skip(1).collect();
        for chunk in chunks {
            let head = chunk.split("#[alun::").next().unwrap_or(chunk);
            let reads_request =
                head.contains("Json(") || head.contains("Path(") || head.contains("Query(");
            if !reads_request {
                continue;
            }
            // 租户根实体自身的创建/查询（无 tenant_id 维度）显式豁免
            if file_name == "tenants.rs"
                && (head.starts_with("create_tenant") || head.starts_with("get_tenant"))
            {
                continue;
            }
            json_handlers += 1;
            assert!(
                head.contains("identity_of("),
                "{file_name}: handler `{}…` 读请求体却未用 identity_of 派生租户",
                head.lines().next().unwrap_or("")
            );
        }
    }
    assert!(
        json_handlers >= 8,
        "覆盖的请求体 handler 只 {json_handlers} 个"
    );
    eprintln!("[TEN-007] JWT 派生租户为准；{json_handlers} 个请求体 handler 均经 identity_of");
}

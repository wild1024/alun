//! L3 E2E：真中间件链 + REST 全流程（用例 API-001..012、TEN-001/007）。
//!
//! 每个方言二进制内 `set_config + set_db` 只发生一次（进程级 `OnceLock`），
//! 因此所有场景在**同一个测试函数**内顺序执行。
//!
//! Router 获取方式：`App::layer_from_fn`（回调在 `build_middleware_chain`
//! 之后执行，拿到的是完整链后的 `Router`，`Router: Clone`）→ 用 `oneshot` 断言，
//! 同时 `start()` 继续在 `127.0.0.1:0` 上服务（用例结束 `abort`）。

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use crate::common::harness::{Dialect, ctx_for};

/// 测试用 JWT 密钥。
pub const JWT_SECRET: &str = "e2e-test-secret-0123456789abcdef";

/// 启动的应用句柄。
pub struct TestApp {
    /// 完整中间件链后的 Router。
    pub router: axum::Router,
    /// 框架按 `[upload].max_size_mb` 推导的请求体上限（字节）。
    ///
    /// 取自与运行时同源的 `UploadConfig::max_request_body_bytes()`，供「超限 413」
    /// 用例构造刚好越界的请求体，避免测试里再抄一份上限口径。
    pub body_limit: usize,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for TestApp {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn dialect_db_type(d: Dialect) -> (&'static str, String) {
    match d {
        Dialect::SqliteMem | Dialect::SqliteFile => ("sqlite", super::sqlite_file_path()),
        Dialect::Postgres => ("postgres", "alun_test".to_string()),
        Dialect::Mysql => ("mysql", "alun_test".to_string()),
    }
}

/// 启动 App 并捕获 Router。
pub async fn boot(d: Dialect) -> TestApp {
    let (db_type, name) = dialect_db_type(d);
    let mut cfg = alun_config::AppConfig {
        app_name: "Alun Ledger E2E".into(),
        profile: "test".into(),
        ..Default::default()
    };
    cfg.server.listen = "127.0.0.1:0".into();
    cfg.database.enabled = true;
    cfg.database.r#type = db_type.to_string();
    cfg.database.name = name;
    cfg.database.host = "localhost".into();
    cfg.database.port = match d {
        Dialect::Postgres => Some(5432),
        Dialect::Mysql => Some(3306),
        _ => None,
    };
    cfg.database.user = "alun".into();
    cfg.database.password = "alun".into();
    cfg.database.migration.enabled = false;
    cfg.middleware.request_id = true;
    cfg.middleware.request_log = false;
    cfg.middleware.auth.enabled = true;
    cfg.middleware.auth.jwt_secret = JWT_SECRET.into();
    cfg.middleware.auth.ignore_paths = vec!["/health".into()];
    // 权限规则由 `#[alun::permission(...)]` 提供，但**必须**打开全局开关才会挂载
    // PermissionCheckLayer（`alun-web/src/app.rs:678-681`）。
    cfg.middleware.permission.enabled = true;
    // 限流：窗口内 100 次，作为最后一个场景验证 429
    cfg.middleware.rate_limit.enabled = true;
    cfg.middleware.rate_limit.requests_per_window = 100;
    cfg.middleware.rate_limit.window_secs = 60;

    // 幂等层（API-012）：由**配置**驱动装配（D017 已修复），无需业务侧手工挂载。
    // 装配条件：`alun-web` 的 `cache` feature + 全局缓存已初始化（`[cache] type=local`）。
    cfg.cache.r#type = "local".into();
    cfg.cache.max_capacity = 10_000;
    cfg.cache.default_ttl = 86_400;
    cfg.middleware.idempotency.enabled = true;
    cfg.middleware.idempotency.ttl_secs = 86_400;

    let (tx, rx) = tokio::sync::oneshot::channel::<axum::Router>();
    // 测试服用默认 `[upload].max_size_mb`；上限口径与运行时同源（框架据此装配
    // DefaultBodyLimit），此处取出来供超限用例复用。
    let body_limit = cfg.upload.max_request_body_bytes();
    let app = alun::App::with_config(cfg)
        .expect("App::with_config")
        .scan()
        // 回调仅用于捕获 Router（不再往链上手工挂层）
        .layer_from_fn(move |router| {
            let _ = tx.send(router.clone());
            router
        });
    let handle = tokio::spawn(async move {
        if let Err(e) = app.start().await {
            eprintln!("[e2e] App::start 退出: {e}");
        }
    });
    let router = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
        .await
        .expect("等待 Router 超时")
        .expect("Router 回调未触发");
    TestApp {
        router,
        body_limit,
        handle,
    }
}

/// 一次请求。
pub async fn call(
    app: &TestApp,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = app.router.clone().oneshot(request).await.expect("oneshot");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 32 * 1024 * 1024)
        .await
        .expect("body");
    let value: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    (status, value, headers)
}

/// 带自定义请求头的一次请求（幂等键用例用）。
pub async fn call_with_header(
    app: &TestApp,
    method: &str,
    uri: &str,
    token: Option<&str>,
    header: (&str, &str),
    body: Option<Value>,
) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header.0, header.1);
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let request = match body {
        Some(v) => builder
            .header("content-type", "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let response = app.router.clone().oneshot(request).await.expect("oneshot");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 32 * 1024 * 1024)
        .await
        .expect("body");
    let value: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    (status, value, headers)
}

/// 用 `JWT::from_config()` 签发访问令牌（`sub = "u{user}:t{tenant}"`）。
pub fn token(user: i64, tenant: i64, permissions: &[&str]) -> String {
    token_with_roles(user, tenant, &["user"], permissions)
}

/// 签发令牌（可指定角色；`admin` 角色会被框架判为超级管理员，绕过权限校验）。
pub fn token_with_roles(user: i64, tenant: i64, roles: &[&str], permissions: &[&str]) -> String {
    let jwt = alun::JWT::from_config();
    let roles: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
    let perms: Vec<String> = permissions.iter().map(|p| p.to_string()).collect();
    jwt.create_access_token(
        &format!("u{user}:t{tenant}"),
        Some("tester"),
        &roles,
        &perms,
    )
    .expect("token")
}

/// 运行全部 E2E 场景。
pub async fn run_all(d: Dialect) -> bool {
    let Some(ctx) = ctx_for(d).await else {
        return false;
    };
    // 夹具：T1 的 5 个账户 + 租户
    crate::common::fixtures::seed_tenant(&ctx, 1, "T1")
        .await
        .unwrap();
    crate::common::fixtures::seed_accounts(&ctx, 1, crate::common::fixtures::money(2000, 0))
        .await
        .unwrap();
    crate::common::fixtures::seed_tenant(&ctx, 2, "T2")
        .await
        .unwrap();

    let app = boot(d).await;
    let t1 = token(1, 1, &["tenant:read"]);
    let t2 = token(2, 2, &[]);
    let admin = token_with_roles(9, 1, &["user"], &["tenant:admin"]);
    let mut report: Vec<(String, String, String)> = Vec::new();
    let mut check = |id: &str, ok: bool, detail: String| {
        report.push((
            id.into(),
            if ok { "PASS".into() } else { "FAIL".into() },
            detail,
        ));
        ok
    };
    let mut all_ok = true;

    // API-001 / OBS-004：健康检查（无 token，ignored path）
    let (st, body, _) = call(&app, "GET", "/health", None, None).await;
    all_ok &= check(
        "API-001a/OBS-004",
        st == StatusCode::OK && body["code"] == 0,
        format!("{st} {body}"),
    );

    // API-002：未认证 → 401，统一错误体 {code,msg}
    let (st, body, _) = call(&app, "GET", "/api/me", None, None).await;
    all_ok &= check(
        "API-002",
        st == StatusCode::UNAUTHORIZED
            && body.get("code").is_some()
            && body.get("msg").is_some()
            && body.get("data").is_none(),
        format!("{st} {body}"),
    );

    // API-007：伪造 token
    let (st, _, _) = call(&app, "GET", "/api/me", Some("not-a-jwt"), None).await;
    all_ok &= check("API-007", st == StatusCode::UNAUTHORIZED, format!("{st}"));

    // refresh token 当 access 用
    let jwt = alun::JWT::from_config();
    let refresh = jwt.create_refresh_token("u1:t1").unwrap();
    let (st, _, _) = call(&app, "GET", "/api/me", Some(&refresh), None).await;
    all_ok &= check("API-007b", st == StatusCode::UNAUTHORIZED, format!("{st}"));

    // API-001：创建租户
    let (st, body, _) = call(
        &app,
        "POST",
        "/api/tenants",
        Some(&t1),
        Some(json!({"id": 9001, "name": "E2E-Tenant", "plan": "pro"})),
    )
    .await;
    all_ok &= check(
        "API-001b",
        st == StatusCode::OK && body["data"]["id"] == 9001,
        format!("{st} {body}"),
    );

    // API-004：唯一约束冲突 → 409（字段携带驱动原始消息）
    let (st, body, _) = call(
        &app,
        "POST",
        "/api/tenants",
        Some(&t1),
        Some(json!({"id": 9002, "name": "E2E-Tenant-2", "plan": "pro"})),
    )
    .await;
    let _ = (st, body);
    let (st, body, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&t1),
        Some(json!({"id": 7001, "email": "dup@example.com", "name": "Alice One"})),
    )
    .await;
    all_ok &= check("API-001c", st == StatusCode::OK, format!("{st} {body}"));
    let (st, body, _) = call(
        &app,
        "POST",
        "/api/users",
        Some(&t1),
        Some(json!({"id": 7002, "email": "dup@example.com", "name": "Alice Two"})),
    )
    .await;
    all_ok &= check(
        "API-004",
        st == StatusCode::CONFLICT && body.get("msg").is_some(),
        format!("{st} {body}"),
    );

    // TEN-001：跨租户读取 → 404（不泄漏存在性）
    let (st, _, _) = call(&app, "GET", "/api/users/7001", Some(&t2), None).await;
    all_ok &= check("TEN-001", st == StatusCode::NOT_FOUND, format!("{st}"));

    // API-003：分页规整（page=0 → 1；page_size 钳到 1000）
    let (st, body, _) = call(
        &app,
        "GET",
        "/api/users?page=0&page_size=10000",
        Some(&t1),
        None,
    )
    .await;
    all_ok &= check(
        "API-003",
        st == StatusCode::OK && body["data"]["page"] == 1 && body["data"]["page_size"] == 1000,
        format!("{st} {body}"),
    );
    // page=-1 → 反序列化失败（axum 400）
    let (st, _, _) = call(&app, "GET", "/api/users?page=-1", Some(&t1), None).await;
    all_ok &= check("API-003b", st == StatusCode::BAD_REQUEST, format!("{st}"));

    // API-001d/005：创建发票 + 非法状态流转
    let (st, body, _) = call(
        &app,
        "POST",
        "/api/invoices",
        Some(&t1),
        Some(json!({
            "id": 8001, "customer_id": 11,
            "metadata": {"tier": "gold", "note": "e2e"},
            "items": [{"id": 80011, "description": "plan fee", "quantity": 2, "unit_price": "10.00"}]
        })),
    )
    .await;
    all_ok &= check("API-001d", st == StatusCode::OK, format!("{st} {body}"));
    let (st, _, _) = call(
        &app,
        "PATCH",
        "/api/invoices/8001/status",
        Some(&t1),
        Some(json!({"status": "paid"})),
    )
    .await;
    all_ok &= check(
        "API-005b",
        st == StatusCode::BAD_REQUEST,
        format!("draft→paid 应拒绝: {st}"),
    );
    let (st, body, _) = call(
        &app,
        "PATCH",
        "/api/invoices/8001/status",
        Some(&t1),
        Some(json!({"status": "issued"})),
    )
    .await;
    // value_map(serde) 后，JSON 响应与落库使用同一编码（D008 已修复）
    all_ok &= check(
        "API-001e",
        st == StatusCode::OK && body["data"]["status"] == "issued",
        format!("{st} {body}"),
    );
    let (st, body, _) = call(
        &app,
        "PATCH",
        "/api/invoices/8001/status",
        Some(&t1),
        Some(json!({"status": "paid"})),
    )
    .await;
    all_ok &= check(
        "API-001f",
        st == StatusCode::OK && body["data"]["paid_at"].is_string(),
        format!("{st} {body}"),
    );

    // API-005c：方法约束——声明为 PATCH 的路径不得被 PUT 命中（D006 修复后新增 patch 宏）
    let (st, _, _) = call(
        &app,
        "PUT",
        "/api/invoices/8001/status",
        Some(&t1),
        Some(json!({"status": "issued"})),
    )
    .await;
    all_ok &= check(
        "API-005c",
        st == StatusCode::METHOD_NOT_ALLOWED,
        format!("同路径 PUT 应为 405，实际 {st}"),
    );

    // API-001g：转账 + 余额
    let (st, body, _) = call(
        &app,
        "POST",
        "/api/transfers",
        Some(&t1),
        Some(json!({
            "transfer_id": 6001, "debit_entry_id": 60011, "credit_entry_id": 60012,
            "from_account": 1001, "to_account": 1002, "amount": "100.00"
        })),
    )
    .await;
    all_ok &= check("API-001g", st == StatusCode::OK, format!("{st} {body}"));
    let (st, body, _) = call(&app, "GET", "/api/accounts/1001/balance", Some(&t1), None).await;
    all_ok &= check(
        "API-001h",
        st == StatusCode::OK && body["data"]["balance"] == "1900",
        format!("{st} {body}"),
    );
    // 余额不足 → 400
    let (st, _, _) = call(
        &app,
        "POST",
        "/api/transfers",
        Some(&t1),
        Some(json!({
            "transfer_id": 6002, "debit_entry_id": 60021, "credit_entry_id": 60022,
            "from_account": 1001, "to_account": 1002, "amount": "999999.00"
        })),
    )
    .await;
    all_ok &= check("API-001i", st == StatusCode::BAD_REQUEST, format!("{st}"));

    // API-001j：MRR 报表 + 审计
    let (st, body, _) = call(&app, "GET", "/api/reports/mrr", Some(&t1), None).await;
    all_ok &= check(
        "API-001j",
        st == StatusCode::OK && body["data"].is_array(),
        format!("{st} {body}"),
    );
    let (st, body, _) = call(&app, "GET", "/api/audit-logs", Some(&t1), None).await;
    all_ok &= check(
        "API-001k",
        st == StatusCode::OK && body["data"]["total"].as_i64().unwrap_or(0) > 0,
        format!("{st} {body}"),
    );

    // API-008：权限（403 / 200）
    let (st, _, _) = call(&app, "GET", "/api/admin/tenants", Some(&t1), None).await;
    all_ok &= check("API-008", st == StatusCode::FORBIDDEN, format!("{st}"));
    let (st, body, _) = call(&app, "GET", "/api/admin/tenants", Some(&admin), None).await;
    all_ok &= check("API-008b", st == StatusCode::OK, format!("{st} {body}"));

    // API-010：请求 ID 必须同时出现在请求头与响应头，且两者一致（D004 已修复）
    let (_, _, headers) = call(&app, "GET", "/api/me", Some(&t1), None).await;
    let rid = headers.get("x-request-id").cloned();
    all_ok &= check(
        "API-010",
        rid.is_some(),
        format!("响应头 x-request-id = {:?}（修复后必须存在）", rid),
    );
    // 显式带上请求 ID → 响应头应原样回带（关联性）
    let (_, _, echoed) = call_with_header(
        &app,
        "GET",
        "/api/me",
        Some(&t1),
        ("x-request-id", "e2e-fixed-id"),
        None,
    )
    .await;
    all_ok &= check(
        "API-010b",
        echoed
            .get("x-request-id")
            .map(|v| v.to_str().unwrap_or_default())
            == Some("e2e-fixed-id"),
        format!("响应头应回带请求 ID，实际 {:?}", echoed.get("x-request-id")),
    );

    // API-011：请求体超过框架按 `[upload].max_size_mb` 推导的上限 → 413。
    //
    // 上限口径由框架统一提供（`UploadConfig::max_request_body_bytes`，装配为
    // `DefaultBodyLimit`），故此处直接取运行时同源值构造「刚好越界」的请求体：
    // 不再依赖 axum 内建的 2 MiB 限制，也避免测试里再抄一份上限公式。
    let big = json!({
        "id": 9100,
        "name": "x".repeat(app.body_limit + 1024),
        "plan": "pro",
    });
    let (st, _, _) = call(&app, "POST", "/api/tenants", Some(&t1), Some(big)).await;
    all_ok &= check(
        "API-011",
        st == StatusCode::PAYLOAD_TOO_LARGE,
        format!("{st}（框架请求体上限 {} 字节）", app.body_limit),
    );

    // API-006：未知路径 404
    let (st, _, _) = call(&app, "GET", "/api/nope", Some(&t1), None).await;
    all_ok &= check("API-006", st == StatusCode::NOT_FOUND, format!("{st}"));

    // API-012：幂等键重放（写路由 + `x-idempotency-key` → 返回缓存响应）
    let body = json!({"id": 9200, "name": "Idem-Tenant", "plan": "pro"});
    let (st1, v1, _) = call_with_header(
        &app,
        "POST",
        "/api/tenants",
        Some(&t1),
        ("x-idempotency-key", "op-9200"),
        Some(body.clone()),
    )
    .await;
    let (st2, v2, _) = call_with_header(
        &app,
        "POST",
        "/api/tenants",
        Some(&t1),
        ("x-idempotency-key", "op-9200"),
        Some(body),
    )
    .await;
    all_ok &= check(
        "API-012",
        st1 == StatusCode::OK && st2 == StatusCode::OK && v1 == v2,
        format!("首次 {st1} / 重放 {st2}（响应体一致 = 命中缓存）"),
    );

    // API-009：限流（最后一个场景：耗尽窗口）
    let mut got_429 = None;
    for i in 0..130 {
        let (st, _, _) = call(&app, "GET", "/api/me", Some(&t1), None).await;
        if st == StatusCode::TOO_MANY_REQUESTS {
            got_429 = Some(i);
            break;
        }
    }
    // 窗口内先前的请求也计入配额，因此这里断言“配额耗尽后持续 429”
    let mut still_429 = true;
    for _ in 0..3 {
        let (st, _, _) = call(&app, "GET", "/api/me", Some(&t1), None).await;
        still_429 &= st == StatusCode::TOO_MANY_REQUESTS;
    }
    all_ok &= check(
        "API-009",
        got_429.is_some() && still_429,
        format!(
            "本循环第 {:?} 次触发 429（窗口 100/60s，含先前请求；耗尽后保持 429={still_429}）",
            got_429.map(|i| i + 1)
        ),
    );

    eprintln!("\n=== E2E 结果（{}）===", d.name());
    for (id, status, detail) in &report {
        eprintln!("{status:4} {id:14} {detail}");
    }
    eprintln!("=== 全部通过: {all_ok} ===\n");
    all_ok
}

//! PostgreSQL **schema-per-tenant** 端到端测试。
//!
//! 验证开启 `[tenant].schema_per_tenant` 后，借出连接会按当前租户执行
//! `SET search_path`，使不同租户的同名表落在各自 schema 中、互不可见。
//!
//! 依赖 docker compose 提供的实例（postgres:17-alpine）：
//!
//! ```text
//! docker compose up -d
//! cargo test -p alun-db --features postgres --test pg_schema_per_tenant
//! ```
//!
//! 连接串可通过 `ALUN_TEST_PG_URL` 覆盖；连接失败时打印原因并跳过。
//! 仅 PostgreSQL 有意义；SQLite / MySQL 无 schema 概念，本测试不覆盖。
//!
//! 两个 schema 开关是**进程级全局状态**，故本文件只放一个测试函数，
//! 避免同进程内并行测例互相干扰。

#![cfg(feature = "postgres")]

use alun_db::prelude::*;
use alun_db_core::stmt::Value;

/// docker-compose.yml 中的默认连接串。
const DEFAULT_PG_URL: &str = "postgres://alun:alun@localhost:5432/alun_test";

/// 连接探测上限：端口代理指向未就绪容器时不能无限阻塞。
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 不可用原因缓存：探测失败后直接跳过，不再重复等待超时。
static PG_DOWN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// 连接测试库。失败/超时返回 `None`（跳过测试）。
async fn setup_pg() -> Option<Db> {
    if let Some(msg) = PG_DOWN.get() {
        eprintln!("跳过 PG 测试（{msg}）");
        return None;
    }
    let url = std::env::var("ALUN_TEST_PG_URL").unwrap_or_else(|_| DEFAULT_PG_URL.to_string());
    let msg = match tokio::time::timeout(PROBE_TIMEOUT, Db::connect(&url)).await {
        Ok(Ok(db)) => return Some(db),
        Ok(Err(e)) => format!("无法连接 {url}：{e}"),
        Err(_) => format!("连接 {url} 超时（>{}s）", PROBE_TIMEOUT.as_secs()),
    };
    let _ = PG_DOWN.set(msg.clone());
    eprintln!("跳过 PG 测试（{msg}）");
    None
}

/// 单个 schema 名（前缀注解固定为 `alun_sp`）。
fn schema_name(tenant_id: i64) -> String {
    format!("alun_sp{tenant_id}")
}

/// 建租户 schema 及其内表（DDL 显式限定 schema，与当前 `search_path` 无关）。
async fn prepare_schema(db: &Db, tenant_id: i64) {
    let schema = schema_name(tenant_id);
    let _ = db
        .execute(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))
        .await;
    db.execute(&format!("CREATE SCHEMA \"{schema}\""))
        .await
        .expect("create tenant schema");
    db.execute(&format!(
        "CREATE TABLE \"{schema}\".sp_users (id BIGSERIAL PRIMARY KEY, name TEXT NOT NULL)"
    ))
    .await
    .expect("create tenant table");
}

/// 在指定租户上下文中写入一行并读回该租户可见的全部行。
///
/// 同一个 future 内先写后读，覆盖非事务写路径与读路径的 `search_path` 注入；
/// 返回值是该租户 schema 内可见的 `name` 列表。
async fn write_and_read(
    db: &Db,
    tenant_id: i64,
    name: &str,
) -> alun_db_core::error::Result<Vec<String>> {
    tenant_scope(TenantContext::Scoped(Value::I64(tenant_id)), async {
        db.execute_with_params(
            "INSERT INTO sp_users (name) VALUES (?)",
            vec![Value::Text(name.to_string())],
        )
        .await?;
        let rows = db.find("SELECT name FROM sp_users ORDER BY id").await?;
        rows.into_iter()
            .map(|row| row.get_typed::<String>("name"))
            .collect()
    })
    .await
}

/// schema-per-tenant：租户各自 `SET search_path`，同名表隔离；跨租户/事务行为正确。
#[tokio::test]
async fn schema_per_tenant_isolates_rows() {
    let Some(db) = setup_pg().await else { return };

    // 预置两个租户的 schema 与同名表
    prepare_schema(&db, 1).await;
    prepare_schema(&db, 2).await;
    // 默认 search_path（public）下不应存在同名表，否则「跨租户可见性」断言失真
    let _ = db.execute("DROP TABLE IF EXISTS public.sp_users").await;

    // 开启 schema-per-tenant
    set_tenant_enabled(true);
    set_tenant_schema_enabled(true);
    set_tenant_schema_prefix("alun_sp");

    // 1. 两个租户各写一行：写入落到各自 schema，读回只见自己的行
    let tenant1 = write_and_read(&db, 1, "alice")
        .await
        .expect("tenant 1 write/read");
    assert_eq!(
        tenant1,
        vec!["alice".to_string()],
        "租户 1 只应看到自己的行"
    );

    let tenant2 = write_and_read(&db, 2, "bob")
        .await
        .expect("tenant 2 write/read");
    assert_eq!(tenant2, vec!["bob".to_string()], "租户 2 只应看到自己的行");

    // 2. 事务路径同样应用 search_path（`SET` 在 `BEGIN` 之前）
    tenant_scope(TenantContext::Scoped(Value::I64(1)), async {
        db.transaction(|tx| {
            Box::pin(async move {
                tx.execute_with_params(
                    "INSERT INTO sp_users (name) VALUES (?)",
                    vec![Value::Text("alice2".to_string())],
                )
                .await?;
                Ok(())
            })
        })
        .await
    })
    .await
    .expect("tenant 1 transaction");
    let tenant1_after = tenant_scope(TenantContext::Scoped(Value::I64(1)), async {
        let rows = db.find("SELECT name FROM sp_users ORDER BY id").await?;
        rows.into_iter()
            .map(|row| row.get_typed::<String>("name"))
            .collect::<alun_db_core::error::Result<Vec<String>>>()
    })
    .await
    .expect("tenant 1 select after tx");
    assert_eq!(
        tenant1_after,
        vec!["alice".to_string(), "alice2".to_string()]
    );

    // 3. 显式跨租户 → 回退默认 search_path（public 无该表），证明按租户切换而非全局污染
    let cross = tenant_scope(TenantContext::Cross, async {
        db.find("SELECT name FROM sp_users").await
    })
    .await;
    assert!(
        cross.is_err(),
        "跨租户上下文不应命中任何租户 schema，实际 {cross:?}"
    );

    // 4. 关闭 schema 开关后不再切换（回到默认 search_path，同样命中不到租户表）
    set_tenant_schema_enabled(false);
    let disabled = tenant_scope(TenantContext::Scoped(Value::I64(1)), async {
        db.find("SELECT name FROM sp_users").await
    })
    .await;
    assert!(
        disabled.is_err(),
        "schema 开关关闭时不应应用租户 search_path，实际 {disabled:?}"
    );

    // 清理：恢复默认开关与 schema 前缀，删除测试 schema
    set_tenant_schema_prefix("");
    set_tenant_enabled(true);
    for tenant_id in [1, 2] {
        let schema = schema_name(tenant_id);
        let _ = db
            .execute(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))
            .await;
    }
}

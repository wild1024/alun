//! 多租户开关语义测试：总开关（`enabled`）与写路径灰度开关（`write_enabled`）。
//!
//! 覆盖迁移流程所需的三种状态：
//! - **总开关关闭**：`#[tenant]` 属性完全惰性——读不隔离、写不补齐、不 fail-closed；
//! - **总开关开启 + 写路径关闭**：读路径照常隔离，写路径退化为「不注入、不校验」；
//! - **两者都开启**：读隔离 + 写补齐 / 越权拦截（默认行为）。
//!
//! 两个开关都是**进程级**全局状态，为避免同进程内并行测例互相干扰，本文件只放
//! **一个**测试函数（独立测试二进制 + 单测例），并在结束时恢复默认值。

mod common;
use alun_db::prelude::*;
use common::*;

/// 参与租户隔离的业务模型。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("invoices")]
struct Invoice {
    /// 主键
    #[key]
    id: i64,
    /// 标题
    title: String,
    /// 租户列
    #[tenant]
    tenant_id: i64,
}

/// 建表并写入两个租户各一行。
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
    db.execute("INSERT INTO invoices (id, title, tenant_id) VALUES (1, 't1-a', 1), (3, 't2-a', 2)")
        .await
        .expect("seed invoices");
}

/// 在指定租户上下文中执行一段逻辑。
async fn with_tenant<F, T>(tenant: i64, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    tenant_scope(TenantContext::Scoped(Value::I64(tenant)), fut).await
}

/// 读取某行的租户列（绕过模型读路径，避免受作用域影响）。
async fn stored_tenant(db: &Db, id: i64) -> i64 {
    db.scalar_with_params::<i64>(
        "SELECT tenant_id FROM invoices WHERE id = ?",
        vec![Value::I64(id)],
    )
    .await
    .expect("read stored tenant")
}

/// 读取某行的标题（绕过模型读路径）。
async fn stored_title(db: &Db, id: i64) -> String {
    db.scalar_with_params::<String>(
        "SELECT title FROM invoices WHERE id = ?",
        vec![Value::I64(id)],
    )
    .await
    .expect("read stored title")
}

#[tokio::test]
async fn tenant_switches_gate_reads_and_writes() {
    let db = setup_db().await;
    seed(&db).await;

    // ── 阶段 A：默认（总开关开、写路径开）──
    set_tenant_enabled(true);
    set_tenant_write_enabled(true);

    with_tenant(1, async {
        // 读隔离：只看到租户 1 的行
        let rows = Invoice::query().find(&db).await.expect("scoped find");
        assert_eq!(rows.len(), 1, "读路径应隔离");
        assert_eq!(rows[0].title, "t1-a");
    })
    .await;

    // 写补齐：占位值（0）被补成上下文租户
    with_tenant(1, async {
        let mut inv = Invoice {
            id: 100,
            title: "filled".into(),
            tenant_id: 0,
        };
        inv.insert(&db).await.expect("scoped insert");
        assert_eq!(inv.tenant_id, 1, "写路径应补齐租户");
    })
    .await;
    assert_eq!(stored_tenant(&db, 100).await, 1);

    // 写越权拦截：跨租户整行更新被拒
    with_tenant(1, async {
        let foreign = Invoice {
            id: 3,
            title: "hijack".into(),
            tenant_id: 1,
        };
        assert!(foreign.update(&db).await.is_err(), "跨租户更新应被拒绝");
    })
    .await;
    assert_eq!(stored_title(&db, 3).await, "t2-a", "租户 2 的行不应被改动");

    // ── 阶段 B：总开关开、写路径关闭（迁移观察期）──
    set_tenant_write_enabled(false);

    with_tenant(1, async {
        // 读路径不受写开关影响，仍然隔离（看不到租户 2 的行 id=3）
        let rows = Invoice::query().find(&db).await.expect("scoped find");
        assert!(
            rows.iter().all(|r| r.tenant_id == 1) && rows.iter().all(|r| r.id != 3),
            "写路径灰度不应影响读隔离：{rows:?}"
        );

        // 写路径惰性：占位值不再被补齐
        let mut inv = Invoice {
            id: 101,
            title: "no-fill".into(),
            tenant_id: 0,
        };
        inv.insert(&db).await.expect("insert without fill");
        assert_eq!(inv.tenant_id, 0, "写路径关闭时不得补齐租户");
    })
    .await;
    assert_eq!(stored_tenant(&db, 101).await, 0, "写路径关闭时按原值落库");

    with_tenant(1, async {
        // 写路径惰性：不追加租户谓词 → 跨租户更新按主键命中
        let foreign = Invoice {
            id: 3,
            title: "hijacked".into(),
            tenant_id: 1,
        };
        foreign
            .update(&db)
            .await
            .expect("update without tenant predicate");
    })
    .await;
    assert_eq!(
        stored_title(&db, 3).await,
        "hijacked",
        "写路径关闭时跨租户更新不再被拦（与未开启隔离逐字一致）"
    );

    // ── 阶段 C：总开关关闭（`#[tenant]` 完全惰性，零回归）──
    set_tenant_write_enabled(true);
    set_tenant_enabled(false);

    // 无上下文读取不再 fail-closed，且不做隔离（能看到租户 2 的行 id=3）
    let all = Invoice::query().find(&db).await.expect("inert read");
    assert!(
        all.iter().any(|r| r.id == 3),
        "总开关关闭时应读到全量（含其它租户的行）：{all:?}"
    );

    // 无上下文写入不补齐、不报错
    let mut orphan = Invoice {
        id: 102,
        title: "inert".into(),
        tenant_id: 0,
    };
    orphan.insert(&db).await.expect("inert insert");
    assert_eq!(stored_tenant(&db, 102).await, 0);

    // 原生 SQL 守卫同样放行
    Invoice::sql(sql!("SELECT * FROM invoices"))
        .find(&db)
        .await
        .expect("raw SQL allowed when tenant feature is off");

    // ── 恢复默认，避免影响后续（若将来有共享进程的用例）──
    set_tenant_enabled(true);
    set_tenant_write_enabled(true);
}

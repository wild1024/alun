//! 多租户作用域（用例 TEN-001..007 的服务层部分）。
//!
//! 框架**不提供**自动租户注入（缺口 G2），所有列表/详情查询都在服务层显式
//! 追加 `tenant_id` 条件；管理员跨租户查询走单独入口。

use alun_db::prelude::*;
use alun_db_core::stmt::Value;

use crate::models::{Invoice, Subscription, Tenant, User};
use crate::services::invoice as invoice_svc;
use crate::services::rowx;

/// 租户内发票列表（显式作用域）。
pub async fn list_invoices(db: &Db, tenant_id: i64) -> Result<Vec<Invoice>> {
    let rows = db
        .find_with_params(
            "SELECT * FROM invoices WHERE tenant_id = ? ORDER BY id",
            vec![Value::I64(tenant_id)],
        )
        .await?;
    rows.iter().map(invoice_svc::invoice_from_row).collect()
}

/// 租户内发票计数。
pub async fn count_invoices(db: &Db, tenant_id: i64) -> Result<i64> {
    db.scalar_with_params::<i64>(
        "SELECT COUNT(*) FROM invoices WHERE tenant_id = ?",
        vec![Value::I64(tenant_id)],
    )
    .await
}

/// 租户内用户列表。
pub async fn list_users(db: &Db, tenant_id: i64) -> Result<Vec<User>> {
    let rows = db
        .find_with_params(
            "SELECT * FROM users WHERE tenant_id = ? ORDER BY id",
            vec![Value::I64(tenant_id)],
        )
        .await?;
    rows.into_iter().map(User::from_row).collect()
}

/// 租户内订阅列表。
pub async fn list_subscriptions(db: &Db, tenant_id: i64) -> Result<Vec<Subscription>> {
    let rows = db
        .find_with_params(
            "SELECT * FROM subscriptions WHERE tenant_id = ? ORDER BY id",
            vec![Value::I64(tenant_id)],
        )
        .await?;
    rows.into_iter().map(Subscription::from_row).collect()
}

/// 租户内账户 id 列表。
pub async fn account_ids(db: &Db, tenant_id: i64) -> Result<Vec<i64>> {
    let rows = db
        .find_with_params(
            "SELECT id FROM accounts WHERE tenant_id = ? ORDER BY id",
            vec![Value::I64(tenant_id)],
        )
        .await?;
    rows.iter().map(|r| rowx::i64_of(r, "id")).collect()
}

/// 管理员跨租户查询（显式放开作用域）。
pub async fn admin_list_tenants(db: &Db) -> Result<Vec<Tenant>> {
    Tenant::query().find(db).await
}

/// 管理员跨租户发票总数。
pub async fn admin_invoice_total(db: &Db) -> Result<i64> {
    db.scalar("SELECT COUNT(*) FROM invoices").await
}

/// 扫描全部业务表，检查是否存在越租户数据（供 `assert_tenant_scope` 断言助手使用）。
///
/// 返回「(表名, 越界行数)」列表；所有计数应为 0。
pub async fn scope_violations(db: &Db, tenant_id: i64) -> Result<Vec<(String, i64)>> {
    let tables = [
        "users",
        "accounts",
        "transfers",
        "ledger_entries",
        "invoices",
        "subscriptions",
    ];
    let mut out = Vec::new();
    for table in tables {
        let sql = format!("SELECT COUNT(*) FROM {table} WHERE tenant_id <> ?");
        let n = db
            .scalar_with_params::<i64>(&sql, vec![Value::I64(tenant_id)])
            .await?;
        out.push((table.to_string(), n));
    }
    Ok(out)
}

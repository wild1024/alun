//! 审计落库。
//!
//! `Hook` 是同步且无 IO 的 trait（`alun-db-core/src/traits/hook.rs:18`），
//! 不能在 Hook 内写库；因此审计写入由服务层在**同一个显式事务**内完成
//! ——这也让「审计失败 → 业务回滚」成立（用例 AUD-006）。

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use alun_db_core::traits::TypeMapper;
use chrono::{DateTime, Utc};

use crate::models::AuditLog;
use crate::services::faults::{self, FailPoint};

/// 一条审计记录。
pub struct AuditRecord<'a> {
    /// 审计行主键（显式指定，保证测试确定性）。
    pub id: i64,
    /// 租户。
    pub tenant_id: i64,
    /// 实体类型（如 `invoice`）。
    pub entity_type: &'a str,
    /// 实体主键。
    pub entity_id: i64,
    /// 动作（如 `issue`）。
    pub action: &'a str,
    /// 变更前快照。
    pub before: Option<&'a serde_json::Value>,
    /// 变更后快照。
    pub after: Option<&'a serde_json::Value>,
    /// 操作人。
    pub actor_id: i64,
    /// 发生时间（由调用方传入，测试可控）。
    pub at: DateTime<Utc>,
}

fn json_param(v: Option<&serde_json::Value>) -> Value {
    match v {
        Some(j) => Value::Json(j.to_string()),
        None => Value::Null,
    }
}

/// 在事务内写入一条审计记录。
pub async fn record(tx: &Transaction, rec: &AuditRecord<'_>) -> Result<()> {
    faults::check(FailPoint::AuditWrite)?;
    tx.execute_with_params(
        "INSERT INTO audit_logs (id, tenant_id, entity_type, entity_id, action, before_json, after_json, actor_id, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        vec![
            Value::I64(rec.id),
            Value::I64(rec.tenant_id),
            Value::Text(rec.entity_type.to_string()),
            Value::I64(rec.entity_id),
            Value::Text(rec.action.to_string()),
            json_param(rec.before),
            json_param(rec.after),
            Value::I64(rec.actor_id),
            rec.at.to_value(),
        ],
    )
    .await?;
    Ok(())
}

/// 读取某实体的审计条数（用于断言）。
pub async fn count_for(db: &Db, entity_type: &str, entity_id: i64) -> Result<i64> {
    db.scalar_with_params::<i64>(
        "SELECT COUNT(*) FROM audit_logs WHERE entity_type = ? AND entity_id = ?",
        vec![Value::Text(entity_type.to_string()), Value::I64(entity_id)],
    )
    .await
}

/// 读取某实体的审计记录（按 id 升序）。
pub async fn list_for(db: &Db, entity_type: &str, entity_id: i64) -> Result<Vec<AuditLog>> {
    let rows = db
        .find_with_params(
            "SELECT * FROM audit_logs WHERE entity_type = ? AND entity_id = ? ORDER BY id",
            vec![Value::Text(entity_type.to_string()), Value::I64(entity_id)],
        )
        .await?;
    rows.into_iter().map(AuditLog::from_row).collect()
}

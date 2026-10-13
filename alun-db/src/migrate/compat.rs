//! 旧框架（`_sqlx_migrations`）存量记录的一次性导入。
//!
//! jinhu / facai 这两个真实生产库此前跑在旧框架上，其迁移引擎是
//! `sqlx::migrate::Migrator`，库内预期已有 `_sqlx_migrations` 表。新库接入时
//! 必须先把这张表的历史版本并入 `schema_migrations`，否则新系统会**重复执行**
//! 已应用的建表语句而失败。
//!
//! ## 触发时机
//!
//! 仅在 `schema_migrations` 为空时探测并导入（见 [`history::ensure_ready`]）；
//! `_sqlx_migrations` 不存在或为空则原样跳过——全新库无需兼容。
//!
//! ## 映射规则
//!
//! 旧表只有 `version`（BIGINT）与 `description`，映射到新表：
//!
//! | 新列 | 取值 |
//! |---|---|
//! | `kind` | 恒为 `versioned` |
//! | `name` | `description` 去除首尾空白；为空时用 `legacy` |
//! | `version` | 旧 `version` 的十进制字符串 |
//! | `checksum` | `NULL`（历史文件不可回溯，首次运行补录） |
//! | `applied_seq` | 旧 `version`（旧 runner 按版本升序应用，作为应用顺序的近似） |
//! | `success` | `true`（旧表只记录成功项） |
//! | `source` | `legacy:_sqlx_migrations` |

use std::collections::BTreeMap;

use alun_db_core::error::{DbError, Result};
use alun_db_core::stmt::Value;
use alun_db_core::traits::TypeMapper;

use crate::db::Db;

use super::history::{self, RecordWrite};
use super::kind::{AppliedVia, MigrationKind};
use super::version::Version;

/// 旧框架（sqlx 迁移系统）的历史迁移表名。
const LEGACY_MIGRATIONS_TABLE: &str = "_sqlx_migrations";

/// 导入行在 `source` 列上的标记。
const LEGACY_SOURCE: &str = "legacy:_sqlx_migrations";

/// 旧表 `description` 为空时使用的占位名称。
const LEGACY_FALLBACK_NAME: &str = "legacy";

/// 一次性导入旧框架（sqlx）的历史迁移记录。
///
/// 逐行读-写而非 `INSERT ... SELECT`：名称按列宽截断在 Rust 侧完成，且写入走
/// `VALUES (?, ...)`——PostgreSQL 由目标列推断参数类型，避免
/// `INSERT ... SELECT ?` 中参数被推断成 `text` 而目标列是 `bigint` 的规划期错误。
///
/// 同一版本可能出现多条描述（旧表无 `(version)` 唯一约束），按版本去重后导入；
/// 全部写入在单个事务中完成，任一行失败则整体回滚，不留半份历史。
///
/// # 参数
///
/// - `db`：数据库句柄。
///
/// # 返回
///
/// `Ok(true)`：执行了导入；`Ok(false)`：旧表不存在或为空，未导入任何记录。
///
/// # 错误
///
/// 旧表存在但读取失败（非「表缺失」）、`version` 列类型不符、事务写入失败时
/// 返回 [`DbError`]。
pub(crate) async fn import_legacy_sqlx_migrations(db: &Db) -> Result<bool> {
    // 探测旧表是否存在：表不存在（全新库）则跳过。
    // 此处是对「表缺失」这一特定失败模式的显式分类，其余错误如实传播。
    match db
        .find_with_params(&format!("SELECT 1 FROM {LEGACY_MIGRATIONS_TABLE}"), vec![])
        .await
    {
        Ok(_) => {}
        Err(e) if history::is_missing_table(&e) => return Ok(false),
        Err(e) => return Err(e),
    }

    let legacy = db
        .find_with_params(
            &format!("SELECT DISTINCT version, description FROM {LEGACY_MIGRATIONS_TABLE}"),
            vec![],
        )
        .await?;
    if legacy.is_empty() {
        return Ok(false);
    }

    let mut by_version: BTreeMap<i64, String> = BTreeMap::new();
    for row in legacy {
        let version = i64::from_value(
            row.get("version")
                .ok_or_else(|| DbError::ColumnNotFound("version".into()))?,
        )?;
        let description = match row.get("description") {
            Some(Value::Null) | None => String::new(),
            Some(value) => String::from_value(value)?,
        };
        let name = legacy_name(&description);
        by_version.entry(version).or_insert(name);
    }

    db.transaction(|tx| async move {
        for (version, name) in by_version {
            history::upsert_in_tx(
                &tx,
                &RecordWrite {
                    kind: MigrationKind::Versioned,
                    name,
                    version: Some(Version::new(version.to_string())),
                    // 历史文件不可回溯计算指纹，留空由首次运行补录
                    checksum: None,
                    // 旧框架执行过这些迁移，因此它们的 `down` 可以安全执行
                    applied_via: AppliedVia::Imported,
                    // 旧 runner 按版本升序应用，以版本号近似应用顺序
                    seq: version,
                    execution_ms: None,
                    is_success: true,
                    source: Some(LEGACY_SOURCE.to_string()),
                },
            )
            .await?;
        }
        Ok(())
    })
    .await?;

    Ok(true)
}

/// 由旧表描述生成新表名称：去除首尾空白，空则用占位名。
fn legacy_name(description: &str) -> String {
    let trimmed = description.trim();
    if trimmed.is_empty() {
        LEGACY_FALLBACK_NAME.to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧表描述为空 / 全空白时回退到占位名，否则原样使用。
    #[test]
    fn falls_back_to_placeholder_name() {
        assert_eq!(legacy_name(""), "legacy");
        assert_eq!(legacy_name("   "), "legacy");
        assert_eq!(legacy_name(" create_users "), "create_users");
    }
}

//! 数据库迁移工具 —— 基于 sqlx 内置 Migrator
//!
//! 设计要点：
//! - 使用 sqlx::migrate::Migrator 执行迁移，自动维护 `_sqlx_migrations` 表
//! - 迁移文件命名格式：`<VERSION>_<DESCRIPTION>.sql`（VERSION 为正整数）
//! - 支持迁移版本追踪、checksum 校验、事务回滚
//! - 启动时 config.migration.enabled = true 自动执行未跑的迁移

use crate::{Db, DbResult, DbError};
use alun_config::MigrationConfig;
use std::path::Path;
use tracing::{info, warn};
use sqlx::migrate::Migrator as SqlxMigrator;

/// 迁移记录
#[derive(Debug, Clone)]
pub struct MigrationRecord {
    /// 迁移版本号（文件名前缀数字，如 "1"、"2"）
    pub version: String,
    /// 迁移描述（文件名中下划线后的部分，如 "base tables init"）
    pub name: String,
    /// 执行时间
    pub executed_at: Option<String>,
    /// 是否成功
    pub success: bool,
}

/// 迁移管理器 —— 基于 sqlx 内置 Migrator 执行数据库迁移
///
/// 使用 sqlx::migrate::Migrator 执行迁移，具备以下特性：
/// - 自动创建 `_sqlx_migrations` 表追踪迁移
/// - 计算每个迁移文件的 checksum，防止已执行迁移被篡改
/// - 按版本号顺序执行未执行的迁移
/// - 支持事务回滚（需提供 `.down.sql` 文件）
pub struct Migrator {
    /// 数据库连接
    db: Db,
    /// 迁移配置
    config: MigrationConfig,
}

impl Migrator {
    /// 创建迁移管理器
    ///
    /// # 参数
    ///
    /// - `db`: 数据库连接实例
    /// - `config`: 迁移配置（包含路径和启用状态）
    pub fn new(db: Db, config: MigrationConfig) -> Self {
        Self { db, config }
    }

    /// 执行所有未执行的迁移（按版本号顺序）
    ///
    /// 若 `config.enabled` 为 `false`，直接返回空列表。
    /// 使用 sqlx 内置 Migrator 执行迁移，自动维护 `_sqlx_migrations` 表。
    ///
    /// # 返回值
    ///
    /// - `Ok(Vec<MigrationRecord>)`: 已执行的迁移记录列表
    /// - `Err(DbError)`: 迁移失败（如 SQL 语法错误、文件不存在等）
    pub async fn run(&self) -> DbResult<Vec<MigrationRecord>> {
        if !self.config.enabled {
            return Ok(vec![]);
        }

        let migration_path = Path::new(&self.config.path);
        info!("使用 sqlx Migrator 执行迁移, path={}", self.config.path);

        let start = std::time::Instant::now();
        let migrator = SqlxMigrator::new(migration_path.to_path_buf())
            .await
            .map_err(|e| DbError::Other(format!("创建 sqlx Migrator 失败: {}", e)))?;

        self.db.run_sqlx_migrator(&migrator).await?;

        let elapsed = start.elapsed();
        info!("sqlx Migrator 执行完成, 耗时: {}ms", elapsed.as_millis());

        // 查询已执行的迁移列表作为返回值
        let executed = self.get_sqlx_executed_migrations().await?;
        info!("迁移完成，共 {} 个迁移", executed.len());
        Ok(executed)
    }

    /// 回滚最近一个迁移
    ///
    /// 使用 sqlx Migrator 的 undo 功能回滚最近一次迁移。
    /// 需要提供对应的 `.down.sql` 回滚文件。
    ///
    /// # 返回值
    ///
    /// - `Ok(Some(MigrationRecord))`: 回滚成功的迁移记录
    /// - `Ok(None)`: 没有可回滚的迁移
    /// - `Err(DbError)`: 回滚失败
    pub async fn rollback(&self) -> DbResult<Option<MigrationRecord>> {
        if !self.config.enabled {
            return Ok(None);
        }

        let executed = self.get_sqlx_executed_migrations().await?;
        if executed.is_empty() {
            info!("没有可回滚的迁移");
            return Ok(None);
        }

        let last = executed.last().expect("executed vec is non-empty (checked above)").clone();
        info!("回滚迁移: version={}", last.version);

        let migration_path = Path::new(&self.config.path);
        let migrator = SqlxMigrator::new(migration_path.to_path_buf())
            .await
            .map_err(|e| DbError::Other(format!("创建 sqlx Migrator 失败: {}", e)))?;

        // sqlx Migrator 的 undo 方法回滚指定数量的迁移
        self.db.undo_sqlx_migrator(&migrator, 1).await?;
        warn!("迁移 {} 已回滚", last.version);
        Ok(Some(last))
    }

    /// 查询 sqlx `_sqlx_migrations` 表中已执行的迁移
    ///
    /// # 返回值
    ///
    /// - `Ok(Vec<MigrationRecord>)`: 已执行的迁移记录列表（按版本号升序）
    /// - `Err(DbError)`: 查询失败
    async fn get_sqlx_executed_migrations(&self) -> DbResult<Vec<MigrationRecord>> {
        let rows = self.db.query(
            "SELECT version, description, installed_on, success FROM _sqlx_migrations ORDER BY version",
            &[],
        ).await?;

        let mut records = Vec::new();
        for row in &rows {
            // Row 基于 HashMap<String, serde_json::Value>，需用 as_* 方法取值
            let version = row.get("version")
                .and_then(|v| v.as_i64())
                .map(|v| v.to_string())
                .unwrap_or_default();
            let name = row.get("description")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            let executed_at = row.get("installed_on")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let success = row.get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            records.push(MigrationRecord { version, name, executed_at, success });
        }
        Ok(records)
    }
}

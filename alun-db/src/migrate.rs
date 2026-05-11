//! 数据库迁移工具 —— 对标 aifei 的 Generator.sql 迁移
//!
//! 设计要点：
//! - 扫描 migrations/ 目录下按时间戳命名的 .sql 文件
//! - 自动建 _migrations 追踪表记录已执行迁移
//! - 启动时 auto_migrate = true 自动执行未跑的迁移
//! - 支持 up/down 双向迁移

use crate::{Db, DbResult, DbError};
use alun_config::MigrationConfig;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// 迁移记录
#[derive(Debug, Clone)]
pub struct MigrationRecord {
    /// 迁移版本（文件名前缀，如 20240501120000）
    pub version: String,
    /// 迁移文件名
    pub name: String,
    /// 执行时间
    pub executed_at: Option<String>,
    /// 是否成功
    pub success: bool,
}

/// 迁移管理器 —— 扫描 `.sql` 文件并执行/回滚迁移
///
/// 支持 up/down 双向迁移，自动维护 `_migrations` 追踪表。
pub struct Migrator {
    /// 数据库连接
    db: Db,
    /// 迁移配置
    config: MigrationConfig,
}

impl Migrator {
    /// 创建迁移管理器
    pub fn new(db: Db, config: MigrationConfig) -> Self {
        Self { db, config }
    }

    /// 执行所有未执行的迁移（按文件名顺序）
    ///
    /// 若 `config.enabled` 为 `false`，直接返回空列表。
    /// 执行过程中任一迁移失败则立即中断并返回错误。
    pub async fn run(&self) -> DbResult<Vec<MigrationRecord>> {
        if !self.config.enabled {
            return Ok(vec![]);
        }

        self.ensure_migrations_table().await?;

        let files = self.discover_migration_files().await;
        if files.is_empty() {
            info!("没有发现待执行的迁移文件");
            return Ok(vec![]);
        }

        let executed = self.get_executed_migrations().await?;
        let mut results = Vec::new();

        for file in &files {
            let version = extract_version(file);
            if executed.contains(&version) {
                continue;
            }

            let result = self.execute_migration(file, &version).await;
            match result {
                Ok(record) => results.push(record),
                Err(e) => {
                    warn!("迁移 {} 执行失败: {}", version, e);
                    return Err(e);
                }
            }
        }

        info!("迁移完成，共执行 {} 个迁移", results.len());
        Ok(results)
    }

    /// 回滚最近一个迁移（需对应的 .down.sql 文件）
    pub async fn rollback(&self) -> DbResult<Option<MigrationRecord>> {
        let executed = self.get_executed_migrations().await?;
        if executed.is_empty() {
            info!("没有可回滚的迁移");
            return Ok(None);
        }

        let last = executed.last().expect("executed vec is non-empty (checked above)").clone();
        let down_file = self.find_down_file(&last);

        match down_file {
            Some(path) => {
                info!("回滚迁移: {}", last);
                let sql = tokio::fs::read_to_string(&path).await
                    .map_err(|e| DbError::Other(format!("读取迁移文件失败: {}", e)))?;
                self.db.execute(&sql, &[]).await?;
                self.mark_migration_rolled_back(&last).await?;
                Ok(Some(MigrationRecord { version: last, name: String::new(), executed_at: None, success: true }))
            }
            None => {
                warn!("迁移 {} 没有对应的 down 文件", last);
                Ok(None)
            }
        }
    }

    /// 确保 `_migrations` 追踪表存在，不存在则自动创建
    async fn ensure_migrations_table(&self) -> DbResult<()> {
        let sql = r#"
            CREATE TABLE IF NOT EXISTS _migrations (
                version VARCHAR(255) PRIMARY KEY,
                name VARCHAR(512) NOT NULL DEFAULT '',
                executed_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                success BOOLEAN DEFAULT TRUE
            )
        "#;
        self.db.execute(sql, &[]).await?;
        info!("迁移追踪表 _migrations 已就绪");
        Ok(())
    }

    /// 扫描迁移目录下所有 `.up.sql` 文件，按文件名排序
    async fn discover_migration_files(&self) -> Vec<PathBuf> {
        let dir = Path::new(&self.config.path);
        if !dir.exists() {
            return vec![];
        }

        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("sql") {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if name.ends_with(".up.sql") {
                        files.push(path);
                    }
                }
            }
        }
        files.sort();
        files
    }

    /// 查询已成功执行的迁移版本列表
    async fn get_executed_migrations(&self) -> DbResult<Vec<String>> {
        let rows = self.db.query(
            "SELECT version FROM _migrations WHERE success = TRUE ORDER BY version", &[],
        ).await?;
        Ok(rows.iter().filter_map(|r| r.get_as::<String>("version")).collect())
    }

    /// 读取并执行单个 `.up.sql` 迁移文件，完成后记录到 `_migrations` 表
    async fn execute_migration(&self, file: &Path, version: &str) -> DbResult<MigrationRecord> {
        let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        info!("执行迁移: {}", name);

        let sql = tokio::fs::read_to_string(file).await
            .map_err(|e| DbError::Other(format!("读取迁移文件失败: {}", e)))?;

        self.db.execute(&sql, &[]).await?;

        self.db.execute(
            "INSERT INTO _migrations (version, name, success) VALUES ($1, $2, TRUE)", &[version, &name],
        ).await?;

        Ok(MigrationRecord { version: version.to_string(), name, executed_at: None, success: true })
    }

    /// 标记迁移已回滚（从 `_migrations` 表中删除对应记录）
    async fn mark_migration_rolled_back(&self, version: &str) -> DbResult<()> {
        self.db.execute(
            "DELETE FROM _migrations WHERE version = $1", &[version],
        ).await?;
        Ok(())
    }

    /// 查找指定版本对应的 `.down.sql` 回滚文件
    fn find_down_file(&self, version: &str) -> Option<PathBuf> {
        let dir = Path::new(&self.config.path);
        if !dir.exists() {
            return None;
        }

        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with(version) && name.ends_with(".down.sql") {
                    return Some(path);
                }
            }
        }
        None
    }
}

fn extract_version(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            if let Some(idx) = n.find('_') {
                n[..idx].to_string()
            } else {
                n.replace(".up.sql", "")
            }
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_version() {
        let path = Path::new("20240501120000_create_users_table.up.sql");
        assert_eq!(extract_version(path), "20240501120000");
    }
}

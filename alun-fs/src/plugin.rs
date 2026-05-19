//! 文件存储插件
//!
//! 通过 `BackendRegistry` 管理多个 `StorageBackend`，
//! 根据请求上下文路由到对应后端。
//!
//! 参考 alun_task::TaskPlugin 的设计模式。

use std::sync::Arc;

use alun_core::plugin::Plugin;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::backend::StorageBackend;
use super::registry::BackendRegistry;
use super::types::FsPluginConfig;

/// 文件存储操作结果类型
pub type StoreResult<T> = Result<T, String>;

/// 文件元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMeta {
    /// UUID v4 文件唯一标识
    pub file_id: String,
    /// 原始文件名（含扩展名）
    pub original_name: String,
    /// 存储相对路径
    pub stored_path: String,
    /// 文件大小（字节）
    pub size: u64,
    /// MIME 类型（如 `image/png`）
    pub content_type: String,
    /// 创建时间
    pub created_at: DateTime<Utc>,
}

/// 多后端文件存储插件
///
/// 通过 `BackendRegistry` 管理多个 `StorageBackend` 实例。
/// 根据 `backend_type`（从请求上下文或 bucket 配置推断）路由到对应后端。
/// 同时实现 `alun_core::Plugin` trait，支持通过 `PluginManager` 管理生命周期。
pub struct FsPlugin {
    /// 后端注册中心
    registry: BackendRegistry,
    /// 全局配置
    config: FsPluginConfig,
}

impl FsPlugin {
    /// 创建本地文件存储插件（向后兼容的快捷构造方法）
    ///
    /// # 参数
    /// - `root_dir` - 本地文件存储根目录
    pub fn new_local(root_dir: &str) -> Self {
        debug_assert!(!root_dir.is_empty(), "root_dir 不得为空");

        let local_fs = super::local::LocalFs::new(root_dir);
        let mut registry = BackendRegistry::new();
        registry.register(
            local_fs,
            super::types::BackendConfig {
                backend_type: "local".into(),
                root_path: root_dir.to_string(),
                ..Default::default()
            },
        );
        registry.with_default("local");

        Self {
            registry,
            config: FsPluginConfig::default(),
        }
    }

    /// 创建文件存储插件
    ///
    /// # 参数
    /// - `config` - 全局运行时配置（建议从 config.toml 的 `[fs]` section 读取）
    /// - `registry` - 已注册后端的注册中心
    pub fn new(config: FsPluginConfig, registry: BackendRegistry) -> Self {
        Self { config, registry }
    }

    /// 获取注册中心引用
    pub fn registry(&self) -> &BackendRegistry {
        &self.registry
    }

    /// 获取全局配置引用
    pub fn config(&self) -> &FsPluginConfig {
        &self.config
    }

    /// 按指定后端类型写入文件
    ///
    /// # 参数
    /// - `backend_type` - 目标后端类型。None 时使用默认后端
    /// - `original_name` - 原始文件名（含扩展名）
    /// - `data` - 文件字节内容
    ///
    /// # 返回
    /// - `Ok(FileMeta)` - 写入成功，返回元数据
    /// - `Err(String)` - 写入失败
    pub async fn write_to(
        &self,
        backend_type: Option<&str>,
        original_name: &str,
        data: &[u8],
    ) -> StoreResult<FileMeta> {
        let backend = self.resolve_backend(backend_type)?;
        backend.write(original_name, data).await
    }

    /// 写入文件（使用默认后端，向后兼容）
    ///
    /// # 参数
    /// - `filename` - 原始文件名（含扩展名）
    /// - `data` - 文件字节内容
    pub async fn write(&self, filename: &str, data: &[u8]) -> StoreResult<FileMeta> {
        self.write_to(None, filename, data).await
    }

    /// 读取文件内容
    ///
    /// # 参数
    /// - `stored_path` - 存储相对路径
    pub async fn read(&self, stored_path: &str) -> StoreResult<Vec<u8>> {
        let backend = self.resolve_backend(None)?;
        backend.read(stored_path).await
    }

    /// 删除文件（不存在不报错）
    ///
    /// # 参数
    /// - `stored_path` - 存储相对路径
    pub async fn delete(&self, stored_path: &str) -> StoreResult<()> {
        let backend = self.resolve_backend(None)?;
        backend.delete(stored_path).await
    }

    /// 检查文件是否存在
    ///
    /// # 参数
    /// - `stored_path` - 存储相对路径
    pub async fn exists(&self, stored_path: &str) -> bool {
        match self.resolve_backend(None) {
            Ok(b) => b.exists(stored_path).await,
            Err(_) => false,
        }
    }

    /// 对所有已注册后端执行健康检查
    ///
    /// # 返回
    /// `Vec<(backend_type, Result)>` 列表
    pub async fn health_check(&self) -> Vec<(String, StoreResult<()>)> {
        self.registry.health_check_all().await
    }

    /// 解析目标后端：按 backend_type 查找，回退到默认
    fn resolve_backend(&self, backend_type: Option<&str>) -> StoreResult<&Arc<dyn StorageBackend>> {
        if let Some(bt) = backend_type {
            if let Some(b) = self.registry.get(bt) {
                return Ok(b);
            }
        }
        self.registry
            .default_backend()
            .ok_or_else(|| "未配置默认存储后端".to_string())
    }
}

#[async_trait]
impl Plugin for FsPlugin {
    fn name(&self) -> &str {
        "fs"
    }

    async fn start(&self) -> alun_core::Result<()> {
        info!(
            "FsPlugin 启动，已注册后端: {:?}, 默认: {:?}",
            self.registry.backend_types(),
            self.registry.default_backend().map(|b| b.backend_type())
        );

        let results = self.registry.health_check_all().await;
        for (bt, result) in &results {
            match result {
                Ok(_) => info!("后端 {} 健康检查通过", bt),
                Err(e) => tracing::warn!("后端 {} 健康检查失败: {}", e, bt),
            }
        }
        Ok(())
    }

    async fn stop(&self) -> alun_core::Result<()> {
        info!("FsPlugin 停止");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn test_new_local_create_and_read() {
        let dir = std::env::temp_dir().join("alun_fs_plugin_test");
        let _ = fs::create_dir_all(&dir);

        let plugin = FsPlugin::new_local(dir.to_str().unwrap());
        let meta = plugin.write("hello.txt", b"hello plugin").await.unwrap();
        assert_eq!(meta.original_name, "hello.txt");

        let data = plugin.read(&meta.stored_path).await.unwrap();
        assert_eq!(data, b"hello plugin");

        plugin.delete(&meta.stored_path).await.unwrap();

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_new_local_delete_nonexistent_shouldbe_ok() {
        let dir = std::env::temp_dir().join("alun_fs_plugin_test2");
        let _ = fs::create_dir_all(&dir);

        let plugin = FsPlugin::new_local(dir.to_str().unwrap());
        assert!(plugin.delete("nonexistent/path.dat").await.is_ok());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_plugin_name() {
        let dir = std::env::temp_dir().join("alun_fs_plugin_test3");
        let _ = fs::create_dir_all(&dir);
        let plugin = FsPlugin::new_local(dir.to_str().unwrap());
        assert_eq!(plugin.name(), "fs");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    #[should_panic(expected = "root_dir")]
    fn test_new_local_empty_root_panics() {
        FsPlugin::new_local("");
    }
}
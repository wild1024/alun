use alun_core::plugin::Plugin;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

use super::local::LocalFs;

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

/// 文件存储后端枚举
#[derive(Clone)]
pub enum FsBackend {
    /// 本地文件系统
    Local(Arc<LocalFs>),
}

/// 文件存储插件（实现 `alun_core::Plugin`，可注册到 PluginManager）
pub struct FsPlugin {
    /// 存储后端
    backend: FsBackend,
}

impl FsPlugin {
    /// 创建本地文件存储插件
    pub fn new_local(root_dir: &str) -> Self {
        let fs = LocalFs::new(root_dir);
        Self {
            backend: FsBackend::Local(Arc::new(fs)),
        }
    }

    /// 获取文件存储后端引用
    pub fn backend(&self) -> &FsBackend { &self.backend }

    /// 获取本地文件系统引用（仅 Local 后端有效）
    pub fn local(&self) -> Option<&Arc<LocalFs>> {
        match &self.backend {
            FsBackend::Local(fs) => Some(fs),
        }
    }

    /// 写入文件（自动按日期和 UUID 命名）
    pub async fn write(&self, filename: &str, data: &[u8]) -> StoreResult<FileMeta> {
        match &self.backend {
            FsBackend::Local(fs) => fs.write_with_name(filename, data).await,
        }
    }

    /// 读取文件内容
    pub async fn read(&self, path: &str) -> StoreResult<Vec<u8>> {
        match &self.backend {
            FsBackend::Local(fs) => fs.read(path).await,
        }
    }

    /// 删除文件（不存在不报错）
    pub async fn delete(&self, path: &str) -> StoreResult<()> {
        match &self.backend {
            FsBackend::Local(fs) => fs.delete(path).await,
        }
    }

    /// 检查文件是否存在
    pub async fn exists(&self, path: &str) -> bool {
        match &self.backend {
            FsBackend::Local(fs) => fs.exists(path).await,
        }
    }
}

#[async_trait]
impl Plugin for FsPlugin {
    fn name(&self) -> &str { "fs" }

    async fn start(&self) -> alun_core::Result<()> {
        info!("FsPlugin 启动 (local)");
        Ok(())
    }

    async fn stop(&self) -> alun_core::Result<()> {
        info!("FsPlugin 停止");
        Ok(())
    }
}

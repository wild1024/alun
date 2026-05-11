use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, error};
use uuid::Uuid;
use chrono::Utc;

use super::plugin::{FileMeta, StoreResult};

/// 本地文件系统存储后端
///
/// 支持按相对路径读写/删除文件，自动创建目录，自动推算 MIME 类型。
pub struct LocalFs {
    /// 存储根目录（绝对路径）
    root_dir: PathBuf,
}

impl LocalFs {
    /// 创建本地文件存储实例
    ///
    /// `root_dir` 相对路径会被展开为基于 `current_dir` 的绝对路径。
    pub fn new(root_dir: &str) -> Self {
        let path = PathBuf::from(root_dir);
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        };

        info!("LocalFs 初始化, root={}", path.display());
        Self { root_dir: path }
    }

    /// 获取存储根目录路径
    pub fn root_dir(&self) -> &Path { &self.root_dir }

    /// 写入文件（relative_path 保留原始名称，UUID 作为 file_id）
    pub async fn write(&self, relative_path: &str, data: &[u8]) -> StoreResult<FileMeta> {
        let full_path = self.resolve(relative_path);
        self.ensure_parent(&full_path).await?;

        let mut file = fs::File::create(&full_path).await.map_err(|e| {
            error!("文件创建失败 {}: {}", full_path.display(), e);
            format!("文件创建失败: {}", e)
        })?;

        file.write_all(data).await.map_err(|e| {
            error!("文件写入失败 {}: {}", full_path.display(), e);
            format!("文件写入失败: {}", e)
        })?;

        let metadata = fs::metadata(&full_path).await.map_err(|e| {
            format!("获取元数据失败: {}", e)
        })?;

        let meta = FileMeta {
            file_id: Uuid::new_v4().to_string(),
            original_name: relative_path.to_string(),
            stored_path: relative_path.to_string(),
            size: metadata.len(),
            content_type: mime_guess(relative_path),
            created_at: Utc::now(),
        };

        info!("文件存储成功: {} ({} bytes)", relative_path, meta.size);
        Ok(meta)
    }

    /// 写入文件（自动按日期分目录：`YYYY/MM/DD/uuid.ext`）
    ///
    /// 比 `write` 更安全——上层无需关心路径命名。
    pub async fn write_with_name(&self, original_name: &str, data: &[u8]) -> StoreResult<FileMeta> {
        let ext = Path::new(original_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin");

        let file_id = Uuid::new_v4().to_string();
        let stored_name = format!("{}.{}", file_id, ext);
        let date_path = Utc::now().format("%Y/%m/%d").to_string();
        let relative_path = format!("{}/{}", date_path, stored_name);

        let full_path = self.resolve(&relative_path);
        self.ensure_parent(&full_path).await?;

        let mut file = fs::File::create(&full_path).await.map_err(|e| {
            format!("文件创建失败: {}", e)
        })?;

        file.write_all(data).await.map_err(|e| {
            format!("文件写入失败: {}", e)
        })?;

        let meta = FileMeta {
            file_id,
            original_name: original_name.to_string(),
            stored_path: relative_path,
            size: data.len() as u64,
            content_type: mime_guess(original_name),
            created_at: Utc::now(),
        };

        info!("文件存储成功: {} -> {} ({} bytes)", original_name, meta.stored_path, meta.size);
        Ok(meta)
    }

    /// 读取文件全部内容
    ///
    /// 文件不存在返回 `Err("文件不存在: ...")`。
    pub async fn read(&self, relative_path: &str) -> StoreResult<Vec<u8>> {
        let full_path = self.resolve(relative_path);

        if !full_path.exists() {
            return Err(format!("文件不存在: {}", relative_path));
        }

        let mut file = fs::File::open(&full_path).await.map_err(|e| {
            format!("文件打开失败: {}", e)
        })?;

        let mut data = Vec::new();
        file.read_to_end(&mut data).await.map_err(|e| {
            format!("文件读取失败: {}", e)
        })?;

        Ok(data)
    }

    /// 删除文件（不存在不报错）
    pub async fn delete(&self, relative_path: &str) -> StoreResult<()> {
        let full_path = self.resolve(relative_path);

        if !full_path.exists() {
            return Ok(());
        }

        fs::remove_file(&full_path).await.map_err(|e| {
            format!("文件删除失败: {}", e)
        })?;

        info!("文件已删除: {}", relative_path);
        Ok(())
    }

    /// 检查文件是否存在
    pub async fn exists(&self, relative_path: &str) -> bool {
        self.resolve(relative_path).exists()
    }

    fn resolve(&self, relative_path: &str) -> PathBuf {
        let path = Path::new(relative_path);
        let path = if path.is_absolute() {
            path.strip_prefix("/").unwrap_or(path).to_path_buf()
        } else {
            path.to_path_buf()
        };
        self.root_dir.join(path)
    }

    async fn ensure_parent(&self, full_path: &Path) -> StoreResult<()> {
        if let Some(parent) = full_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    format!("目录创建失败: {}", e)
                })?;
            }
        }
        Ok(())
    }
}

fn mime_guess(filename: &str) -> String {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");

    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" => "application/javascript",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "mp3" => "audio/mpeg",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }.into()
}

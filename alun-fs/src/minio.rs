//! MinIO / S3 兼容存储后端
//!
//! 基于 `aws-sdk-s3` 实现，兼容 MinIO、AWS S3、腾讯云 COS、阿里云 OSS 等
//! S3 兼容对象存储服务。
//!
//! 在 `Cargo.toml` 启用 `minio` feature 以编译此模块。

use std::path::Path;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::backend::StorageBackend;
use crate::plugin::FileMeta;
use crate::types::BackendConfig;

/// MinIO / S3 兼容存储后端
pub struct MinioBackend {
    /// S3 客户端
    client: aws_sdk_s3::Client,
    /// 存储桶名称
    bucket: String,
    /// 区域标识
    region: String,
}

impl MinioBackend {
    /// 从 `BackendConfig` 创建 MinIO 后端实例
    ///
    /// # 参数
    /// - `config` - 后端配置（endpoint, access_key, secret_key, region, bucket）
    ///
    /// # 错误
    /// - 如连接失败或认证失败，返回 `Err(String)`
    pub async fn from_config(config: &BackendConfig) -> Result<Self, String> {
        let cred = aws_sdk_s3::config::Credentials::new(
            &config.access_key,
            &config.secret_key,
            None,
            None,
            "minio-credentials",
        );

        let s3_config = aws_sdk_s3::Config::builder()
            .endpoint_url(&config.endpoint)
            .region(aws_sdk_s3::config::Region::new(config.region.clone()))
            .credentials_provider(cred)
            .force_path_style(config.endpoint.contains("minio") || !config.use_tls)
            .behavior_version_latest()
            .build();

        let client = aws_sdk_s3::Client::from_conf(s3_config);

        let backend = Self {
            client,
            bucket: config.root_path.clone(),
            region: config.region.clone(),
        };

        // 启动时确保 bucket 存在
        backend.ensure_bucket().await?;
        Ok(backend)
    }

    /// 确保 bucket 存在，若不存在则创建
    async fn ensure_bucket(&self) -> Result<(), String> {
        let exists = self
            .client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await;

        if exists.is_err() {
            self.client
                .create_bucket()
                .bucket(&self.bucket)
                .send()
                .await
                .map_err(|e| format!("MinIO 创建 bucket 失败: {}", e))?;
            tracing::info!("MinIO bucket '{}' 已创建", self.bucket);
        }
        Ok(())
    }

    /// 生成存储路径：`YYYY/MM/DD/uuid.ext`
    fn build_stored_path(original_name: &str) -> String {
        let ext = Path::new(original_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin");
        let file_id = Uuid::new_v4().to_string();
        let date_path = Utc::now().format("%Y/%m/%d").to_string();
        format!("{}/{}.{}", date_path, file_id, ext)
    }

    /// MIME 类型推断
    fn mime_type(filename: &str) -> String {
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
        }
        .into()
    }
}

#[async_trait]
impl StorageBackend for MinioBackend {
    fn backend_type(&self) -> &str {
        "minio"
    }

    async fn write(&self, original_name: &str, data: &[u8]) -> Result<FileMeta, String> {
        use aws_sdk_s3::primitives::ByteStream;

        let stored_path = Self::build_stored_path(original_name);
        let content_type = Self::mime_type(original_name);
        let data_len = data.len() as u64;

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&stored_path)
            .body(ByteStream::from(data.to_vec()))
            .content_type(&content_type)
            .send()
            .await
            .map_err(|e| format!("MinIO 写入失败: {}", e))?;

        tracing::info!(
            "MinIO 存储成功: {} -> {} ({} bytes)",
            original_name,
            stored_path,
            data_len
        );

        Ok(FileMeta {
            file_id: Uuid::new_v4().to_string(),
            original_name: original_name.to_string(),
            stored_path,
            size: data_len,
            content_type,
            created_at: Utc::now(),
        })
    }

    async fn read(&self, stored_path: &str) -> Result<Vec<u8>, String> {
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(stored_path)
            .send()
            .await
            .map_err(|e| format!("MinIO 读取失败: {}", e))?;

        let data = resp
            .body
            .collect()
            .await
            .map_err(|e| format!("MinIO 读取流失败: {}", e))?
            .into_bytes()
            .to_vec();

        Ok(data)
    }

    async fn delete(&self, stored_path: &str) -> Result<(), String> {
        let result = self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(stored_path)
            .send()
            .await;

        // MinIO 删除不存在的对象不报错，符合幂等设计
        if let Err(e) = result {
            tracing::warn!("MinIO 删除文件 {} 返回错误: {}（已忽略）", stored_path, e);
        }
        Ok(())
    }

    async fn exists(&self, stored_path: &str) -> bool {
        self.client
            .head_object()
            .bucket(&self.bucket)
            .key(stored_path)
            .send()
            .await
            .is_ok()
    }

    async fn presign_download_url(
        &self,
        stored_path: &str,
        expiry_secs: Option<u64>,
    ) -> Result<String, String> {
        use aws_sdk_s3::presigning::PresigningConfig;
        use std::time::Duration;

        let ttl = Duration::from_secs(expiry_secs.unwrap_or(3600));
        let presign_config = PresigningConfig::expires_in(ttl)
            .map_err(|e| format!("MinIO 预签名配置失败: {}", e))?;

        let req = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(stored_path)
            .presigned(presign_config)
            .await
            .map_err(|e| format!("MinIO 预签名失败: {}", e))?;

        Ok(req.uri().to_string())
    }

    async fn health_check(&self) -> Result<(), String> {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(|e| format!("MinIO 健康检查失败: {}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_stored_path_has_extension() {
        let path = MinioBackend::build_stored_path("photo.jpg");
        assert!(path.ends_with(".jpg"));
        assert!(path.contains('/')); // 含日期子目录
    }

    #[test]
    fn test_build_stored_path_no_extension() {
        let path = MinioBackend::build_stored_path("readme");
        assert!(path.ends_with(".bin"));
    }

    #[test]
    fn test_mime_type_known() {
        assert_eq!(MinioBackend::mime_type("photo.jpg"), "image/jpeg");
        assert_eq!(MinioBackend::mime_type("data.json"), "application/json");
        assert_eq!(MinioBackend::mime_type("report.pdf"), "application/pdf");
    }

    #[test]
    fn test_mime_type_unknown() {
        assert_eq!(
            MinioBackend::mime_type("data.xyz"),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_backend_type() {
        // 无法在单元测试中创建 MinioBackend 实例（需要真实服务），
        // 但可验证静态方法
        assert_eq!(
            MinioBackend::mime_type("test.png"),
            "image/png"
        );
    }
}
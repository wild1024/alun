//! 存储后端统一行为特质
//!
//! 定义所有存储后端（LocalFs、MinioBackend、S3Backend）必须实现的接口契约。
//! 框架层通过此 trait 操作文件，不关心具体存储介质。
//!
//! 参考 alun_task::TaskHandler 的设计模式：定义契约，业务实现。

use async_trait::async_trait;

use super::plugin::FileMeta;

/// 存储后端统一行为特质
///
/// 每个存储后端实现此 trait 以提供文件的读写删查能力。
///
/// ## 示例
///
/// ```ignore
/// struct MyBackend;
///
/// #[async_trait]
/// impl StorageBackend for MyBackend {
///     fn backend_type(&self) -> &str { "custom" }
///     async fn write(&self, name: &str, data: &[u8]) -> Result<FileMeta, String> { ... }
///     async fn read(&self, path: &str) -> Result<Vec<u8>, String> { ... }
///     async fn delete(&self, path: &str) -> Result<(), String> { ... }
///     async fn exists(&self, path: &str) -> bool { ... }
/// }
/// ```
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// 返回后端类型标识（如 "local", "minio", "s3"）
    fn backend_type(&self) -> &str;

    /// 写入文件并返回元数据
    ///
    /// # 参数
    /// - `original_name` - 原始文件名（含扩展名）。后端自动处理路径命名（如按日期+UUID 分目录）
    /// - `data` - 文件字节内容
    ///
    /// # 返回
    /// - `Ok(FileMeta)` - 写入成功，返回元数据（含 stored_path, file_id, size, content_type）
    /// - `Err(String)` - 写入失败原因
    async fn write(&self, original_name: &str, data: &[u8]) -> Result<FileMeta, String>;

    /// 按指定相对路径写入文件
    ///
    /// 与 `write` 不同，此方法**完全由调用方指定存储路径**，
    /// 后端不做日期分目录/UUID 重命名等自动处理，仅按给定路径写入。
    ///
    /// 用于需要精确控制物理存储位置的场景，例如：
    /// - 公开/私有文件物理隔离：`public/YYYY/MM/DD/uuid.ext` vs `private/YYYY/MM/DD/uuid.ext`
    /// - 按业务子目录归档：`avatars/2026/...`、`covers/...`
    ///
    /// # 参数
    /// - `relative_path` - 存储相对路径（相对于后端根目录，如 `public/2026/08/06/uuid.ext`）
    /// - `data` - 文件字节内容
    ///
    /// # 返回
    /// - `Ok(FileMeta)` - 写入成功，返回元数据（stored_path 即为传入的 relative_path）
    /// - `Err(String)` - 写入失败原因
    async fn write_at(&self, relative_path: &str, data: &[u8]) -> Result<FileMeta, String>;

    /// 读取文件全部内容
    ///
    /// # 参数
    /// - `stored_path` - 存储相对路径（由 write 返回的 FileMeta.stored_path）
    ///
    /// # 返回
    /// - `Ok(Vec<u8>)` - 文件字节内容
    /// - `Err(String)` - 读取失败原因
    async fn read(&self, stored_path: &str) -> Result<Vec<u8>, String>;

    /// 删除文件
    ///
    /// 文件不存在时应返回 `Ok(())` 而非 `Err`（幂等设计）。
    ///
    /// # 参数
    /// - `stored_path` - 存储相对路径
    async fn delete(&self, stored_path: &str) -> Result<(), String>;

    /// 检查文件是否存在
    ///
    /// # 参数
    /// - `stored_path` - 存储相对路径
    async fn exists(&self, stored_path: &str) -> bool;

    /// 生成预签名下载 URL（可选实现）
    ///
    /// 本地文件系统返回空字符串，MinIO/S3 生成带签名的临时 URL。
    ///
    /// # 参数
    /// - `stored_path` - 存储相对路径
    /// - `expiry_secs` - URL 有效期（秒），None 表示使用默认值（建议 3600）
    ///
    /// # 返回
    /// - `Ok(String)` - 预签名 URL
    /// - `Err(String)` - 生成失败原因
    async fn presign_download_url(
        &self,
        _stored_path: &str,
        _expiry_secs: Option<u64>,
    ) -> Result<String, String> {
        Ok(String::new())
    }

    /// 健康检查（可选实现）
    ///
    /// 启动时验证存储后端连接是否正常。
    /// 默认返回 `Ok(())`，各后端按需重写。
    async fn health_check(&self) -> Result<(), String> {
        Ok(())
    }
}
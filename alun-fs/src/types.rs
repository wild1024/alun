//! 类型定义：后端配置、插件配置、编译期注册条目
//!
//! 参考 alun_task::types 的双层配置模型：
//! - `BackendConfig` — 每种 backend_type 一份（类比 TaskConfig）
//! - `FsPluginConfig` — 全局运行时配置（类比 TaskWorkerConfig）
//! - `BackendEntry` — linkme 分布式切片条目（类比 TaskHandlerEntry）

use crate::backend::StorageBackend;

/// 单个存储后端的配置
///
/// 每种 backend_type（"local", "minio", "s3"）对应一份配置。
/// 配置来源：可以从 `sys_storage_bucket` 表读取，也可从 config.toml 读取。
#[derive(Debug, Clone)]
pub struct BackendConfig {
    /// 后端类型标识（"local", "minio", "s3" 等）
    pub backend_type: String,
    /// 服务端点（本地路径 或 http(s)://地址）
    pub endpoint: String,
    /// 区域标识（MinIO/S3 签名计算时使用）
    pub region: String,
    /// 访问密钥
    pub access_key: String,
    /// 秘密密钥
    pub secret_key: String,
    /// 是否启用 TLS
    pub use_tls: bool,
    /// 存储根路径（LocalFs 用）或 bucket 名称（MinIO/S3 用）
    pub root_path: String,
    /// 是否为默认后端
    pub is_default: bool,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            backend_type: "local".into(),
            endpoint: "uploads".into(),
            region: String::new(),
            access_key: String::new(),
            secret_key: String::new(),
            use_tls: false,
            root_path: "uploads".into(),
            is_default: true,
        }
    }
}

/// FsPlugin 的全局运行时配置
///
/// 从 config.toml 的 `[fs]` section 读取。
/// 参考 alun_task::TaskWorkerConfig — 全局运行时参数。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct FsPluginConfig {
    /// 默认后端类型
    #[serde(default = "default_backend_type")]
    pub default_backend_type: String,
    /// 本地存储根目录（LocalFs 回退用）
    #[serde(default = "default_root_dir")]
    pub local_root_dir: String,
    /// 上传文件大小上限（字节），默认 50MB
    #[serde(default = "default_max_size")]
    pub max_file_size_bytes: u64,
    /// 预签名 URL 默认有效期（秒）
    #[serde(default = "default_presign_ttl")]
    pub presign_url_ttl_secs: u64,
}

fn default_backend_type() -> String { "local".into() }
fn default_root_dir() -> String { "uploads".into() }
fn default_max_size() -> u64 { 52_428_800 }
fn default_presign_ttl() -> u64 { 3600 }

impl Default for FsPluginConfig {
    fn default() -> Self {
        Self {
            default_backend_type: default_backend_type(),
            local_root_dir: default_root_dir(),
            max_file_size_bytes: default_max_size(),
            presign_url_ttl_secs: default_presign_ttl(),
        }
    }
}

/// 编译期后端注册条目
///
/// 配合 `#[storage_backend]` 宏 + linkme 分布式切片使用。
/// 业务项目通过宏注解声明后端，编译期自动收集到此切片。
///
/// 参考 alun_task::TaskHandlerEntry。
#[derive(Debug, Clone)]
pub struct BackendEntry {
    /// 后端类型标识（如 "minio", "s3"）
    pub backend_type: &'static str,
    /// 后端构造函数（返回 Box<dyn StorageBackend>）
    pub constructor_fn: fn() -> Box<dyn StorageBackend>,
    /// 后端配置函数（返回 BackendConfig）
    pub config_fn: fn() -> BackendConfig,
}

/// 存储后端分布式切片
///
/// `#[storage_backend]` 宏注解的后端实例在此汇集。
/// 使用 linkme 在链接期自动收集，启动时通过 `BackendRegistry::from_discovered()` 一键注册。
///
/// 参考 alun_task::TASK_HANDLERS。
#[linkme::distributed_slice]
pub static STORAGE_BACKENDS: [BackendEntry] = [..];
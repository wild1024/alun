---
id: reference.crates.fs
title: `alun-fs` — 多后端文件存储
summary: `alun-fs` — 多后端文件存储——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [fs, reference, minio, storage]
status: active
owner: alun-core-team
source_of_truth: [alun-fs/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-fs` — 多后端文件存储

> 本文件是 [`crates.md`](../crates.md) 的 `fs` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。

## `alun-fs`

**路径**: `alun-fs/src/`

多后端文件存储框架，提供统一的 `StorageBackend` trait 抽象，支持本地文件系统、MinIO、AWS S3 等存储后端。核心设计理念：**Trait 定义契约 + Registry 管理实例 + Plugin 组装门面** —— 业务层零感知存储介质差异，切换后端仅需改配置。

## 目录结构

| 模块 | 职责 |
| ---- | ---- |
| `backend` | `StorageBackend` trait（统一存储接口：backend_type / write / write_at / read / delete / exists / presign_download_url / health_check） |
| `registry` | `BackendRegistry`（按 backend_type 管理多后端实例，支持运行时注册 + `inventory` 编译期自动发现） |
| `types` | `BackendConfig`（每种后端的配置）、`FsPluginConfig`（全局运行时配置）、`BackendEntry`（`inventory` 注册条目） |
| `local` | `LocalFs`（本地文件系统实现，按日期分目录 YYYY/MM/DD/uuid.ext，自动 MIME 推断） |
| `minio` | `MinioBackend`（MinIO / S3 兼容实现，条件编译 `feature = "minio"`，支持预签名 URL） |
| `plugin` | `FsPlugin`（多后端门面，实现 `alun_core::Plugin`，统一生命周期管理） |

## 公共 API

| 导出项 | 说明 |
| ------ | ---- |
| `StorageBackend` | 存储后端统一行为 trait（crate 根导出）：`backend_type()`、`write(original_name, data)`、`write_at(relative_path, data)`、`read(stored_path)`、`delete(stored_path)`、`exists(stored_path)`；`presign_download_url()` / `health_check()` 有默认实现 |
| `LocalFs` | 本地文件系统后端（crate 根导出）：`new(root_dir)`、`root_dir()`、`write_at()`、`write_with_name()`、`read()`、`delete()`、`exists()` |
| `FsPlugin` | 多后端文件存储插件门面（crate 根导出）：`new_local(root_dir)`、`new(FsPluginConfig, BackendRegistry)`、`registry()`、`config()`、`write()`、`write_to(backend_type, …)`、`write_at()`、`read()`、`delete()`、`exists()`、`health_check()` |
| `FileMeta` | 文件元信息结构体（crate 根导出：`file_id` / `original_name` / `stored_path` / `size` / `content_type` / `created_at`） |
| `StoreResult<T>` | 存储操作结果类型别名 `Result<T, String>`（crate 根导出） |
| `alun_fs::registry::BackendRegistry` | 后端注册中心（**子模块路径，crate 根不导出**）：`new()`、`register(backend, config)`、`from_discovered()`、`with_default(type)`、`get()`、`default_backend()`、`get_config()`、`backend_types()`、`len()`/`is_empty()`、`health_check_all()` |
| `alun_fs::types::BackendConfig` | 单个后端配置（**子模块路径**）：`backend_type` / `endpoint` / `region` / `access_key` / `secret_key` / `use_tls` / `root_path` / `is_default` |
| `alun_fs::types::FsPluginConfig` | 全局运行时配置（**子模块路径**）：`default_backend_type` / `local_root_dir` / `max_file_size_bytes` / `presign_url_ttl_secs` |
| `alun_fs::types::{BackendEntry, discovered_storage_backends}` | `inventory` 注册条目 / 遍历函数（**子模块路径**） |
| `alun_fs::minio::MinioBackend` | MinIO / S3 兼容后端（**子模块路径**，需 `minio` feature）：`from_config(&BackendConfig)` |

> 没有 `#[storage_backend]` 宏，也没有 `FsBackend` 类型；自定义后端需自行在 crate 内用 `alun_fs::types::BackendEntry` + `inventory::submit!` 声明条目，或用 `BackendRegistry::register` 运行时注册。
> 走编译期注册这一条路需要应用自己声明 `inventory` 依赖（门面只为**宏生成**的注册项转发 `inventory`）；想零额外依赖就用 `BackendRegistry::register`。

## 使用示例

```rust,ignore
// ── 基本用法：local 后端（向后兼容） ──
use alun_fs::{FsPlugin, LocalFs, FileMeta};

// 一行创建本地存储插件
let plugin = FsPlugin::new_local("uploads");

// 写入文件（自动按 YYYY/MM/DD/uuid.ext 存储）
let meta = plugin.write("report.pdf", &file_data).await?;
// meta.stored_path → "2026/05/19/a1b2c3d4.pdf"
// meta.file_id → UUID v4

// 读取文件
let data = plugin.read(&meta.stored_path).await?;

// 删除文件（幂等）
plugin.delete(&meta.stored_path).await?;

// ── 多后端：注册中心 + trait（注意子模块路径） ──
use alun_fs::registry::BackendRegistry;
use alun_fs::types::{BackendConfig, FsPluginConfig};
use alun_fs::StorageBackend;
use std::sync::Arc;

let config = FsPluginConfig::default();

let mut registry = BackendRegistry::new();
// 注册 local 后端
registry.register(
    LocalFs::new("uploads"),
    BackendConfig { backend_type: "local".into(), root_path: "uploads".into(), ..Default::default() },
);
// 汇总编译期注册的后端，并指定默认后端
registry.from_discovered().with_default("local");

let plugin = FsPlugin::new(config, registry);

// 按 backend_type 写入到指定后端
let meta = plugin.write_to(Some("local"), "avatar.png", &img_data).await?;

// ── 自定义后端：impl StorageBackend trait ──
struct MyOssBackend { /* ... */ }

#[async_trait]
impl StorageBackend for MyOssBackend {
    fn backend_type(&self) -> &str { "my-oss" }
    async fn write(&self, name: &str, data: &[u8]) -> Result<FileMeta, String> { /* ... */ }
    async fn write_at(&self, path: &str, data: &[u8]) -> Result<FileMeta, String> { /* ... */ }
    async fn read(&self, path: &str) -> Result<Vec<u8>, String> { /* ... */ }
    async fn delete(&self, path: &str) -> Result<(), String> { /* ... */ }
    async fn exists(&self, path: &str) -> bool { /* ... */ }
    async fn presign_download_url(&self, path: &str, ttl: Option<u64>) -> Result<String, String> { /* ... */ }
}
// 想让 `from_discovered()` 发现它，需自行声明到切片：
// inventory::submit! { alun_fs::types::BackendEntry { … } }
// static MY_OSS: alun_fs::types::BackendEntry = alun_fs::types::BackendEntry { … };
// 也可以用 registry.register(MyOssBackend { … }, cfg) 直接运行时注册。

// ── 配置示例（config.toml） ──
// [fs]
// default_backend_type = "local"
// local_root_dir = "uploads"
// max_file_size_bytes = 52428800
// presign_url_ttl_secs = 3600

// MinIO 后端（需启用 minio feature，子模块路径）
#[cfg(feature = "minio")]
use alun_fs::minio::MinioBackend;
#[cfg(feature = "minio")]
let minio = MinioBackend::from_config(&BackendConfig {
    backend_type: "minio".into(),
    endpoint: "http://localhost:9000".into(),
    region: "us-east-1".into(),
    access_key: "minioadmin".into(),
    secret_key: "minioadmin".into(),
    root_path: "my-bucket".into(),
    ..Default::default()
}).await?;

// 注册到 registry
registry.register(minio, minio_config).with_default("minio");
```

***

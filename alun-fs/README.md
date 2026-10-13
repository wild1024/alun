# alun-fs

Alun 文件存储抽象：本地文件系统 + 多后端（MinIO / S3）。

> 门面侧 `features = ["fs"]` 即可用；通过 `FsPlugin` 注册后端，再用
> `alun::alun_fs::registry::BackendRegistry` 管理多个后端。

## 主要 API

crate 根：

- `trait StorageBackend`（async）—— `backend_type()`、`write(original_name, data)`、`write_at(relative_path, data)`、
  `read(stored_path)`、`delete(stored_path)`、`exists(stored_path)`；
  `presign_download_url` 与 `health_check` 有默认实现。
  （**没有** `put` / `get`：写是 `write` / `write_at`，读是 `read`。）
- `LocalFs` —— `new(root_dir)`、`root_dir()`，以及自己的 `write_at` / `write_with_name` / `read` /
  `delete` / `exists`；`backend_type()` 为 `"local"`
- `FileMeta` —— `file_id`、`original_name`、`stored_path`、`size`、`content_type`、`created_at`
- `StoreResult<T> = Result<T, String>`
- `FsPlugin` —— `new_local(root_dir)`、`new(FsPluginConfig, BackendRegistry)`、`registry()`、`config()`、
  `write(filename, data)`、`write_at(relative_path, data)`、`write_to(backend_type, original_name, data)`、
  `read`、`delete`、`exists`、`health_check()`；实现 `Plugin`，名字为 `"fs"`

需要完整路径时：

- `registry::BackendRegistry` —— `new()`、`register(backend, BackendConfig)`、`from_discovered()`、
  `with_default(backend_type)`、`get(backend_type)`、`default_backend()`、`get_config()`、`backend_types()`、
  `len()`、`is_empty()`、`health_check_all()`
- `types::BackendConfig` —— `backend_type` / `endpoint` / `region` / `access_key` / `secret_key` /
  `use_tls` / `root_path` / `is_default`（默认退化为本地 `uploads` 后端）
- `types::FsPluginConfig` —— `default_backend_type` / `local_root_dir` / `max_file_size_bytes` /
  `presign_url_ttl_secs`
- `types::BackendEntry` 与编译期注册表（`inventory`，供 `BackendRegistry::from_discovered()` 自动发现）
- `minio::MinioBackend` —— 需 feature `minio`；`from_config(&BackendConfig)`，`backend_type()` 为 `"minio"`

## 示例

```rust,no_run
use alun_fs::{FileMeta, FsPlugin};

let fs = FsPlugin::new_local("uploads");

let meta: FileMeta = fs.write("avatar.png", b"\x89PNG...").await?;
let bytes = fs.read(&meta.stored_path).await?;
let exists = fs.exists(&meta.stored_path).await;
fs.delete(&meta.stored_path).await?;
```

显式注册后端（多桶 / 混合本地与对象存储）：

```rust,ignore
use alun_fs::registry::BackendRegistry;
use alun_fs::types::{BackendConfig, FsPluginConfig};
use alun_fs::{FsPlugin, LocalFs};

let mut registry = BackendRegistry::new();
registry.register(LocalFs::new("uploads"), BackendConfig::default());

let fs = FsPlugin::new(FsPluginConfig::default(), registry);
let meta = fs.write_to(Some("local"), "report.csv", b"a,b\n1,2\n").await?;
```

> ⚠️ `BackendRegistry::register(backend, config)` 的**第一个参数是后端实例**（不是 `Arc`），
> 第二个才是配置；同名 `backend_type` 会后者覆盖。

## Feature

| Feature | 默认 | 作用 |
|---------|------|------|
| `local` | 是 | 本地文件系统后端（`LocalFs` 本身始终可用） |
| `minio` | 否 | 拉入 `aws-sdk-s3` / `aws-config`，编译 `minio::MinioBackend` |

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)

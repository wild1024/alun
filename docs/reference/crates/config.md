---
id: reference.crates.config
title: `alun-config` — 配置系统
summary: `alun-config` — 配置系统——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [config, reference, toml, env]
status: active
owner: alun-core-team
source_of_truth: [alun-config/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-config` — 配置系统

> 本文件是 [`crates.md`](../crates.md) 的 `config` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun-config/src/`

## 关键结构体

### `AppConfig` （[lib.rs](../../../alun-config/src/lib.rs)）

完整应用配置的根结构体，包含 `server`、`log`、`database`、`redis`、`cache`、`middleware`、`router`、`plugins`、`upload`、`download`、`template`、`static_files`、`custom` 等子配置。

### `ConfigManager` （[lib.rs](../../../alun-config/src/lib.rs)）

配置管理器。

| 方法                                     | 说明                   |
| -------------------------------------- | -------------------- |
| `ConfigManager::load(config_dir)`      | 从目录加载配置，自动检测 profile |
| `get()`                                | 获取静态配置引用             |
| `get_dynamic(key)`                     | 获取动态配置值              |
| `set_dynamic(key, value)`              | 设置运行时动态配置            |
| `remove_dynamic(key)`                  | 删除动态配置               |
| `ConfigManager::generate_default(dir)` | 生成默认配置文件             |

## 配置子结构体

| 结构体                               | 说明                                                                           |
| --------------------------------- | ---------------------------------------------------------------------------- |
| `ServerConfig`                    | 监听地址配置                                                                       |
| `LogConfig`                       | 日志级别/格式/目录                                                                   |
| `DatabaseConfig`                  | 数据库类型/连接/连接池/迁移                                                              |
| `RedisConfig`                     | Redis 连接                                                                     |
| `CacheConfig`                     | 缓存类型/容量/TTL                                                                  |
| `MiddlewareConfig`                | 中间件总开关（含 `request_log`/`auth`/`cors`/`compression`/`rate_limit`/`security_headers`/`permission` 子配置） |
| `RequestLogConfig`                | 请求日志（排除路径等）                                                              |
| `AuthMiddlewareConfig`            | JWT 密钥、access/refresh 过期时间、`ignore_paths`                                |
| `CorsConfig` / `CompressConfig`   | 跨域 / 压缩开关                                                          |
| `RateLimitConfig`                 | 限流窗口与配额                                                               |
| `SecurityHeadersConfig`           | 安全响应头开关                                                               |
| `PermissionConfig` + `PermissionRule` | 配置文件的路径级权限规则（`path` / `methods` / `permission`）+ `enabled` 开关；**`#[alun::permission]` 宏注解的规则不受 `enabled` 影响，始终挂载**（`enabled = false` 时打 warn 提示） |
| `RouterConfig`                    | 全局路由前缀、404 自定义处理                                                             |
| `NotFoundConfig`                  | 404 响应开关与提示消息                                                            |
| `PluginsConfig`                   | 插件启用列表 + `notification`/`async_task`/`scheduler`/`serial` 子配置                     |
| `NotificationConfig`              | SMTP 邮件通知配置                                                              |
| `AsyncTaskConfig`                 | 后台任务队列配置（worker 数等）                                                        |
| `SchedulerConfig`                 | 定时任务配置                                                                       |
| `SerialConfig` + `SerialRuleConfig` | 单号生成后端（`memory`/`redis`/`postgres`）与静态规则列表                                 |
| `MigrationConfig`                 | 迁移开关/文件目录/自动迁移                                                               |
| `UploadConfig` / `DownloadConfig` | 上传/下载路径配置                                                                    |
| `TemplateConfig`                  | 模板文件目录                                                                       |
| `StaticConfig`                    | 静态文件服务配置                                                                     |

**上传/下载/静态文件配置详情**：

`UploadConfig`：

| 字段            | 类型       | 默认值         | 说明         |
| ------------- | -------- | ----------- | ---------- |
| `path`        | `String` | `"uploads"` | 上传文件存储目录   |
| `max_size_mb` | `u64`    | `10`        | 最大文件大小（MB） |

`DownloadConfig`：

| 字段     | 类型       | 默认值           | 说明       |
| ------ | -------- | ------------- | -------- |
| `path` | `String` | `"downloads"` | 下载文件存储目录 |

`StaticConfig`：

| 字段        | 类型       | 默认值        | 说明         |
| --------- | -------- | ---------- | ---------- |
| `path`    | `String` | `"static"` | 静态文件目录     |
| `enabled` | `bool`   | `false`    | 是否启用静态文件服务 |

`RouterConfig`：

| 字段          | 类型               | 默认值  | 说明       |
| ----------- | ---------------- | ---- | -------- |
| `prefix`    | `String`         | `""` | 全局路由前缀   |
| `not_found` | `NotFoundConfig` | —    | 404 处理配置 |

`NotFoundConfig`：

| 字段        | 类型       | 默认值          | 说明                                |
| --------- | -------- | ------------ | --------------------------------- |
| `enabled` | `bool`   | `true`       | 是否启用自定义 404 响应（返回 JSON 格式的统一错误响应） |
| `message` | `String` | `"请求的资源不存在"` | 自定义 404 提示消息                      |

> **说明**：当 `not_found.enabled = true` 且未启用静态文件服务 (`static_files.enabled = false`) 时，未匹配路由的请求将返回 JSON 格式的 `{"code":404, "msg":"…"}` 统一错误响应。若同时启用了静态文件服务，则 ServeDir 优先作为 fallback。

## 环境配置模块 `env`

| 函数                         | 说明                                           |
| -------------------------- | -------------------------------------------- |
| `detect_profile()`         | 检测当前 profile（CLI > `ALUN_PROFILE` > `"dev"`） |
| `parse_args()`             | 解析命令行（`gen-config`/`print-config`）           |
| `merge_env_overrides(cfg)` | `ALUN_` 前缀环境变量覆盖配置                           |

**配置加载流程**：

1. `config/config.toml` → 基础配置
2. `config/config-{profile}.toml` → profile 覆盖（按需）
3. `ALUN_*` 环境变量 → 最终覆盖

## 使用示例

```rust,ignore
use alun_config::ConfigManager;
use std::sync::Arc;

// ── 加载配置 ──
let cm = Arc::new(ConfigManager::load(Some("config".into())));

// ── 读取静态配置 ──
let port = &cm.get().server.listen;             // 监听端口
let db_enabled = cm.get().database.enabled;      // 数据库是否启用
let jwt_secret = &cm.get().middleware.auth.jwt_secret;

// ── 运行时动态配置（get_dynamic 返回 Option<serde_json::Value>） ──
cm.set_dynamic("rate_limit.requests_per_window", serde_json::json!(200));
let limit = cm.get_dynamic("rate_limit.requests_per_window").and_then(|v| v.as_u64());
cm.set_dynamic("feature.new_api", serde_json::json!(true));
let enabled = cm.get_dynamic("feature.new_api").and_then(|v| v.as_bool());

// ── 读取自定义配置 ──
let custom_val = cm.get_dynamic("custom.my_key");
let key = custom_val.as_ref().and_then(|v| v.as_str());

// ── 多环境 Profile ──
// ALUN_PROFILE=prod → 自动加载 config/config-prod.toml
// 加载顺序: config.toml → config-{profile}.toml → ALUN_* 环境变量

// ── 生成默认配置 ──
ConfigManager::generate_default("config")?;     // 生成 config/config.toml
```

***

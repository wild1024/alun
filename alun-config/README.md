# alun-config

Alun 配置系统：TOML 加载、多环境 Profile、环境变量覆盖、运行时动态配置。

> 业务侧一般只通过门面拿配置：`alun::cfg()`（静态）、`alun::config()`（可写动态层）、
> 以及 `alun::App` 自己读的 `config/config.toml`。本 crate 是它们的实现。

## 主要 API

- `AppConfig` —— 整个应用配置，分组为
  `app_name`、`profile`、`server`（`ServerConfig { listen }`）、`log`（`LogConfig { level, format, dir, file_prefix }`）、
  `database`、`redis`、`cache`、`middleware`、`router`、`plugins`、`upload`、`download`、`template`、
  `static_files`、以及 `custom`（给插件用的自由 `HashMap<String, serde_json::Value>`）。
  每一组都有 `Default`，所以**没有 `config.toml` 也能拿到一份能跑的配置**。
- `ConfigManager` —— 持有不可变 `static_config: AppConfig` 加一层 `dynamic` 覆盖：
  - `ConfigManager::load(config_dir: Option<&str>) -> Self`（`None` 即 `"config"`）
  - `get() -> &AppConfig`、`get_dynamic(key) -> Option<serde_json::Value>`、`set_dynamic(key, value)`、`remove_dynamic(key)`
  - `generate_default(dir) -> std::io::Result<()>` —— 写出一份带中文注释的 `config/config.toml`
- 自由函数：`detect_profile()`、`parse_args() -> (gen_config, print_config)`、`merge_env_overrides(&mut AppConfig)`
- 供中间件装配使用：`PermissionRule`、`PermissionConfig`、`NotificationConfig`、`SerialConfig`、
  `SerialRuleConfig`、`PluginsConfig`、`IdempotencyConfig`（`[middleware.idempotency]`）、
  `NonceConfig`（`[middleware.nonce]`）……

### 加载顺序

1. `config/config.toml`（基础；文件缺失则用 `AppConfig::default()`）
2. `config/config-{profile}.toml`（在其上做深合并）
3. `ALUN_*` 环境变量

### Profile 选择

命令行 `profile=prod` > `ALUN_PROFILE` > `ALUN_ENV` > `"dev"`

### 环境变量

`ALUN_<SECTION>_<FIELD>` 映射到配置路径，例如：

| 变量 | 字段 |
|------|------|
| `ALUN_SERVER_LISTEN` | `server.listen` |
| `ALUN_LOG_LEVEL` / `ALUN_LOG_FORMAT` / `ALUN_LOG_DIR` | `log.*` |
| `ALUN_DATABASE_HOST` / `_NAME` / `_USER` / `_PASSWORD` / `_SSL_MODE` | `database.*` |
| `ALUN_REDIS_URL` | `redis.url` |
| `ALUN_CACHE_TYPE` | `cache.type` |
| `ALUN_ROUTER_PREFIX` | `router.prefix` |
| `ALUN_UPLOAD_PATH` / `ALUN_DOWNLOAD_PATH` / `ALUN_TEMPLATE_PATH` | 对应路径字段 |
| `ALUN_MIDDLEWARE_AUTH_JWT_SECRET` | `middleware.auth.jwt_secret` |

空值会被忽略；JWT 密钥还接受别名 `FACAI_JWT_SECRET`（优先级更高）。

### CLI 参数

```text
cargo run -- gen-config       # 写出 config/config.toml
cargo run -- print-config     # 打印解析后的当前配置
cargo run -- profile=prod     # 指定 Profile
```

> 这三个参数需要在 `main` 里链 `.parse_cli()` 才生效；`profile=prod` 与 `ALUN_*` 环境变量
> 在启动加载配置时即生效，不依赖 `parse_cli()`。

## 示例

```rust,ignore
use alun_config::ConfigManager;

let cm = ConfigManager::load(Some("config"));

let listen = &cm.get().server.listen;
let level = &cm.get().log.level;

// 运行时覆盖（改完立即生效，不重启）
cm.set_dynamic("feature_x", serde_json::json!(true));
let enabled = cm.get_dynamic("feature_x").and_then(|v| v.as_bool()).unwrap_or(false);
cm.remove_dynamic("feature_x");
```

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)
- 速查参考：`skill-alun/references/config.md`

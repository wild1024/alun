# alun-log

Alun 日志：按配置初始化 tracing，支持控制台 + 按天滚动的文件输出。

> 框架启动流程里会自动初始化（读 `[log]` 段），业务一般不需要直接调用本 crate；
> 想在非 Web 程序（CLI / 工具）里单独初始化时用 `init`。

## 主要 API

- `init(config: &alun_config::LogConfig)` —— 安装全局 `tracing` subscriber。
  语义上幂等：已经装过全局 subscriber 时不会重复装（`try_init`，错误忽略）。

由 `LogConfig` 驱动的行为：

| 字段 | 作用 |
|------|------|
| `level` | 应用到 `alun*` 系列 crate 的级别（默认 `info`） |
| `format` | `"text"` 人类可读（默认）；`"json"` 结构化 JSON，直接喂 ELK / Loki |
| `dir` | 有值时额外按天滚动写文件到该目录 |
| `file_prefix` | 滚动文件名的前缀 |

日志过滤规则：

- 设了 `RUST_LOG` 就**完全覆盖**内置过滤器
- 否则：业务代码按 `debug`、其余 `alun*` crate 按 `LogConfig::level`、
   noisy 第三方（`tower_http`、`hyper`、`sqlx`、`reqwest` …）固定 `warn`

## 示例

```rust,ignore
use alun_config::ConfigManager;

#[tokio::main]
async fn main() -> alun_core::Result<()> {
    let cm = ConfigManager::load(None);
    alun_log::init(&cm.get().log);

    tracing::info!(app = %cm.get().app_name, "logging initialized");
    Ok(())
}
```

等价的 `config/config.toml` 段：

```toml
[log]
level = "info"
format = "text"
dir = "logs"          # 不写就只输出到 stdout
file_prefix = "alun"
```

## Feature

本 crate 不声明任何 cargo feature。

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)

# alun-core

Alun 框架内核：应用生命周期、插件系统、统一错误。

> 这里是**纯骨架层**：零 HTTP 概念、零数据库概念、零可选依赖，
> 可以被非 Web 项目单独复用（定时任务、CLI、业务库）。
> 大部分业务只需要依赖门面 [`alun`](https://crates.io/crates/alun)。

## 主要 API

### 错误

- `Error` —— 变体 `Config(String)`、`Plugin { name, source }`、`Server(String)`、
  `Io(std::io::Error)`、`Template(String)`、`Msg(String)`
- `Result<T> = std::result::Result<T, Error>`
- 内置转换：`From<std::io::Error>` / `From<String>` / `From<&str>`
- **本 crate 既不认识数据库也不认识 HTTP**：`DbError` 属于 `alun-db-core`，
  `ApiError` / `Res` 属于 `alun-web`，三者互不依赖（硬约束）。
  handler 的单一出口 `ResResult<T>` 由 `alun-web` 提供，它同时认识两边，
  因此 `?` 一个数据库错误可以直接抛进 handler（见 `alun-web/README.md`）

### 插件

- `trait Plugin` —— `name()`、`async start()`、`async stop()`、`depends_on()`（默认 `&[]`）
- `PluginManager` —— `new()`、`add(plugin)`、`add_discovered(plugins)`、`start_all()`、
  `stop_all()`、`check_duplicate_names()`；插件按依赖关系**正序启动、逆序停止**

> **响应形状类型不在这里**：`Res` / `ResResult` / `ApiError` / `PageQuery` /
> `PageData` / `codes` 住在 [`alun-web`](../alun-web/README.md)（`IntoResponse` 需要 axum）。
> 经 `alun` 门面的 `prelude` 原样导出，业务端路径不受影响。

## 示例

```rust,no_run
use alun_core::Result;

fn load_config() -> Result<String> {
    Err(alun_core::Error::Config("config.toml 格式错误".into()))
}

fn main() {
    let e = load_config().unwrap_err();
    assert!(e.to_string().contains("格式错误"));
    // io::Error / String / &str 也能直接 `?` 进来
    let _: alun_core::Error = std::io::Error::other("boom").into();
}
```

## 自定义插件

```rust,ignore
use alun_core::{Plugin, PluginManager, Result};

struct MyPlugin;

#[async_trait::async_trait]
impl Plugin for MyPlugin {
    fn name(&self) -> &str { "my-plugin" }
    async fn start(&self) -> Result<()> { Ok(()) }
    async fn stop(&self) -> Result<()> { Ok(()) }
}

let manager = PluginManager::new().add(MyPlugin);
manager.start_all().await?;
```

## Feature

| Feature | 启用内容 |
|---------|---------|
| （无） | 本 crate **刻意没有任何 feature** |

> 没有 `db` feature、也没有 `web` feature —— 这是刻意的：核心层一旦引入
> `alun-db-core` 或 axum，「非 Web / 非数据库项目可单独复用 alun-core」的定位就没了。
> HTTP 响应与数据库错误的收敛统一在 [`alun-web`](../alun-web/README.md)。

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)
- 速查参考：`skill-alun/references/plugins.md`

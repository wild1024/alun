---
id: reference.crates.core
title: `alun-core` — 核心抽象层
summary: `alun-core` — 核心抽象层——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [core, reference, error, plugin, api]
status: active
owner: alun-core-team
source_of_truth: [alun-core/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-core` — 核心抽象层

> 本文件是 [`crates.md`](../crates.md) 的 `core` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun-core/src/`

是整个框架的"语言"基础——定义错误类型与插件协议。**零 Web 依赖、零数据库依赖**，
非 Web 项目（定时任务 / CLI / 业务库）可单独复用。

> 响应形状类型（`Res` / `ApiError` / `PageQuery` / `PageData` / `codes`）**不在这里**——
> 它们满身 HTTP 概念，统一住在 [`alun-web`](web.md)（`alun-web/src/api.rs`）。

## 关键结构体

### `Error` （[error.rs](../../../alun-core/src/error.rs)）

框架统一错误类型，支持从 `std::io::Error`、`String`、`&str`、`ApiError` 自动转换。

| 变体                               | 说明                    |
| -------------------------------- | --------------------- |
| `Error::Config(String)`          | 配置加载或解析错误             |
| `Error::Plugin { name, source }` | 插件生命周期错误（携带插件名和底层错误源） |
| `Error::Server(String)`          | 服务器启动或运行失败            |
| `Error::Template(String)`        | 模板加载/渲染错误             |
| `Error::Io(std::io::Error)`      | IO 操作错误               |
| `Error::Msg(String)`             | 通用业务错误消息              |

```rust,ignore
pub type Result<T> = std::result::Result<T, Error>;
```

> **响应体与分页类型已迁出**：见 [`alun-web`](web.md) 的
> `Res<T>` / `ApiError` / `PageQuery` 与 `PageData<T>` 小节（源码 `alun-web/src/api.rs`）。

## 关键 Trait

### `Plugin` （[plugin.rs](../../../alun-core/src/plugin.rs)）

插件生命周期协议。

```rust,ignore
#[async_trait]
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    async fn start(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    fn depends_on(&self) -> &[&str] { &[] }
}
```

| 方法             | 说明                    |
| -------------- | --------------------- |
| `name()`       | 插件唯一名称（用于注册、日志、依赖解析）  |
| `start()`      | 启动插件，拓扑排序执行，失败则中止后续插件 |
| `stop()`       | 关闭插件，逆序执行，失败仅记录日志     |
| `depends_on()` | 依赖项声明，默认无依赖           |

### `PluginManager` （[plugin.rs](../../../alun-core/src/plugin.rs)）

插件管理器——负责注册、拓扑排序启动、逆序关闭。

| 方法                        | 说明           |
| ------------------------- | ------------ |
| `new()`                   | 创建空管理器       |
| `add(plugin)`             | 手动注册插件（链式调用） |
| `add_discovered(plugins)` | 批量注册编译期发现的插件 |
| `start_all()`             | 拓扑排序后依次启动    |
| `stop_all()`              | 逆序关闭         |
| `check_duplicate_names()` | 检查名称重复       |

## 使用示例

```rust,no_run
use alun::prelude::*;

// ── Res<T> 响应 ──
# #[derive(serde::Serialize)]
# struct UserModel { id: String, name: String }

async fn get_user(user: UserModel) -> Res<UserModel> {
    Res::ok(user)                                   // { code:0, msg:"ok", data:user }
}

async fn list_users(users: Vec<UserModel>, total: u64) -> Res<PageData<Vec<UserModel>>> {
    Res::page(users, total, 1, 20)                  // 分页响应
}

// ── 带错误返回 ──
// handler 的唯一出口就是 ResResult<T>；数据库调用直接用 `?`
async fn find_user(id: String) -> ResResult<UserModel> {
    if id.is_empty() {
        return Err(ApiError::bad_request("ID 不能为空"));
    }
    let row = db()
        .find_first_with_params(
            "SELECT id, name FROM users WHERE id = ?",
            vec![alun_db::prelude::Value::Text(id)],
        )
        .await?          // DbError → ApiError（From 内置在 alun-web）
        .ok_or_else(|| ApiError::not_found("用户不存在"))?;
    Ok(Res::ok(UserModel {
        id: row.get_typed("id").unwrap_or_default(),
        name: row.get_typed("name").unwrap_or_default(),
    }))
}

// ── 错误屏蔽（生产环境推荐） ──
# async fn do_something() -> std::result::Result<(), String> { Ok(()) }
async fn risky_operation() -> ResResult<()> {
    do_something().await.map_err(|e| {
        tracing::error!("内部错误: {:?}", e);
        ApiError::internal_masked("服务器内部错误", format!("{:?}", e))
    })?;
    Ok(Res::ok_empty())
}

// ── 分页参数：全框架唯一的 PageQuery 在 alun-web ──
async fn paging(pq: PageQuery) -> Result<()> {
    let pq = PageQuery::new(pq.page, pq.page_size);   // 第2页，每页20条
    assert_eq!(PageQuery::new(2, 20).offset(), 20);   // SQL OFFSET（越界值先规整，不会下溢）
    assert_eq!(pq.limit(), pq.page_size);             // SQL LIMIT
    assert!(pq.calc_pages(45) >= 1);                  // 总页数
    Ok(())
}

// ── Plugin Trait 实现 ──
struct MetricsPlugin;

#[async_trait::async_trait]
impl Plugin for MetricsPlugin {
    fn name(&self) -> &str { "metrics" }
    async fn start(&self) -> Result<()> { Ok(()) }
    async fn stop(&self) -> Result<()> { Ok(()) }
}

// ── PluginManager 使用（add 按值注册并返回 Self，可链式） ──
async fn run_plugins() -> Result<()> {
    let manager = PluginManager::new()
        .add(MetricsPlugin)
        .add_discovered(vec![]);
    manager.check_duplicate_names().map_err(Error::Msg)?;   // Result<(), String>
    manager.start_all().await?;
    // ... 应用运行 ...
    manager.stop_all().await;
    Ok(())
}
```

***

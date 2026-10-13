---
id: reference.crates.facade
title: `alun` — 门面 Crate（prelude / feature / re-export）
summary: `alun` — 门面 Crate（prelude / feature / re-export）——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [facade, reference, prelude, features]
status: active
owner: alun-core-team
source_of_truth: [alun/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun` — 门面 Crate（prelude / feature / re-export）

> 本文件是 [`crates.md`](../crates.md) 的 `facade` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun/src/lib.rs`

用户唯一直接依赖的 crate。通过 `prelude` 模块统一导出所有公共符号，proc macro 直接 re-export。

## 模块 `alun::prelude`

```rust,ignore
pub mod prelude {
    pub use alun_core::{
        Result, Error, Plugin, PluginManager, Res, ResResult, ApiError, PageData, PageQuery, codes,
        DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE,
    };
    // alun-db 的完整 prelude：Model 派生宏 / TypeMapper / CrudExt / PersistExt / ModelQuery /
    // ChangeSet / sql! / DbError / Value / Type / Expr / Hook / Db / Row / Page / Transaction /
    // Migration / DatabaseConfig / PoolConfig …（只依赖 alun 即可写 Model）
    #[cfg(feature = "db")]
    pub use alun_db::prelude::*;
    #[cfg(feature = "db")]
    pub use alun_web::PageResponseExt;
    pub use alun_web::{App, AlunRouter, TokenClaims, TokenType, UserId, AuthClaims, ValidatedJson, JWT};
    pub use alun_web::middleware::{NonceLayer, IdempotencyLayer};
    pub use alun_utils::{Crypto, Mask, Sid, Valid};
    #[cfg(feature = "xss")]
    pub use alun_utils::xss;
    pub use alun_config::AppConfig;
    pub use alun_web::resources::{cfg, config, try_config, set_config};
    #[cfg(feature = "db")]
    pub use alun_web::resources::{db, try_db, set_db};
    #[cfg(feature = "cache")]
    pub use alun_web::resources::{cache, try_cache, set_cache};
    #[cfg(feature = "template")]
    pub use alun_web::resources::{render_template, try_template, set_template};
    #[cfg(feature = "template")]
    pub use alun_template::TemplateEngine;
    #[cfg(feature = "cache")]
    pub use alun_cache::{Cache, CacheStats, LocalCache};
    #[cfg(feature = "plugin")]
    pub use alun_plugin;
    pub use serde_json::{json, Value as JsonValue};
    pub use axum::response::Json as AxumJson;
    pub use axum::extract::{Path, Query};
    pub use axum::Extension;
}
```

`prelude` 中数据库那一块是 `alun_db::prelude::*` 的**完整**转发（`Model` 派生宏、`TypeMapper`、
`CrudExt` / `PersistExt`、`ModelQuery` / `ChangeSet`、`sql!`、`DbError`、`Value` / `Type` / `Expr`、
`Hook` / `TimestampHook`、`Db` / `Row` / `Page` / `Transaction` / `Migration` 等），因此应用
**不需要**在自己的 `Cargo.toml` 里声明 `alun-db` 或 `alun-db-core`；`alun-db-macro` 会按调用方
实际声明的 crate 解析生成代码里的内核路径（只声明 `alun` 时用 `alun::__db_core`）。

> **注意**：`prelude` 中的 `Result` 是 `alun_core::Result<T>`（**只有一个泛型参数**）——它是显式导出，
> 优先于 `alun_db::prelude` 里同名的 `Result<T, E = DbError>`；因此 DB 辅助函数要写
> `DbResult<T>`（= `alun_db::prelude::Result<T>`，`E = DbError`）。
>
> handler 则**只有一种出口**：`ResResult<T>`（`Result<Res<T>, ApiError>`）。
> `ApiError` 住在 `alun-web`，是那里的本地类型，所以 `From<DbError>` /
> `From<alun_core::Error>` / `From<io::Error>` 全部内置——数据访问层的 `?` 可以直接抛进 handler。
> 非 HTTP 场景（启动 / 插件 / 定时任务）才是 `alun::Result<T>`，那里收不下 `DbError`。
> `prelude` **不**导出 `upload_path`/`download_path`（用 `alun_web::resources::upload_path` 或 `alun::web::...`）；
> 缓存在 `prelude` 里只有 `Cache`/`CacheStats`/`LocalCache`，`SharedCache`/`RedisCache`/`create_cache` 需从 `alun_cache` 直接引入。

## Feature 开关

`alun` 把子 crate 的 feature **逐个转发**，用户只需在门面上选：

| feature | 转发到 | 说明 |
| ------- | ------ | ---- |
| `db` | `alun-db`（+ `alun-web/db`） | 数据库门面，**只带 `async` 能力，不含任何驱动**（必须再选 `sqlite` / `postgres` / `mysql`） |
| `sqlite` / `postgres` / `mysql` | `alun-db/同名` | 驱动；`postgres`/`mysql` 隐含 `db` |
| `chrono` / `time` / `jiff` / `rust_decimal` / `bigdecimal` / `uuid` / `ulid` / `json` / `ipnetwork` | `alun-db/同名` | 模型生态类型映射，隐含 `db` |
| `xss` | `alun-utils/xss` | HTML 净化；模块经 `alun::xss` 与 prelude 暴露 |
| `template` / `cache` / `plugin` / `kafka` / `task` / `fs` / `config` | 对应 crate | 非数据库能力（原有） |
| `full` | 上述全部（含三驱动） | 默认 |

`db` 只带 SQLite 驱动：连 PG/MySQL 需再开 `postgres` / `mysql`，否则建池报
`DbError::UnsupportedFeature { feature: "driver", dialect: "postgres" }`。

## Proc Macro 导出

| 宏                                               | 说明                                         |
| ----------------------------------------------- | ------------------------------------------ |
| `#[alun::get("/path")]`                         | 标记异步函数为 GET 路由处理器                          |
| `#[alun::post("/path")]`                        | 标记异步函数为 POST 路由处理器                         |
| `#[alun::put("/path")]`                         | 标记异步函数为 PUT 路由处理器                          |
| `#[alun::delete("/path")]`                      | 标记异步函数为 DELETE 路由处理器                       |
| `#[alun::controller("/prefix")]`                | 标注在 **impl 块**上，块内方法用 `#[alun::get("/path")]` 等标注；宏摘除这些内部属性、在 impl 之外生成路由注册（handler 为 `类型::方法名`） |
| `#[alun::plugin]`                               | 标记结构体为插件                                   |
| `#[alun::permission(path = "/api/admin", method = "GET", permission = "admin:read")]` | 方法级权限校验（**必须用具名参数** `path=`/`method=`/`permission=`，位置参数会被解析为空串） |
| `#[alun::no_auth("/path")]`                     | 标记无需认证的路由（登录用户仍可获取用户信息）                    |
| `#[alun::task_handler(task_type = N, ...)]`     | 标记 TaskHandler 自动注册到任务框架（需 `task` feature） |

> `alun` 自身也有 `xss` feature（转发到 `alun-utils`，模块经 `alun::xss` 与 prelude 暴露）；
> 完整 feature 清单见本节「Feature 开关」表。

## 顶层 Re-export

除 `prelude` 外，`alun` crate 还在顶层（以及 `alun::web` 别名下）直接导出以下符号：

| 导出项                                                                                                              | 来源             | 说明                                    |
| ---------------------------------------------------------------------------------------------------------------- | -------------- | ------------------------------------- |
| `get`/`post`/`put`/`patch`/`delete`/`controller`/`routes`/`plugin`/`permission`/`no_auth`/`layer`/`task_handler`     | `alun-macros`  | 全部过程宏（含此前漏导出的 `no_auth`；`layer` 为路由级 layer，写在路由注解上方） |
| `Crypto`、`Mask`、`Sid`、`Valid`、`CyclePeriod`、`FormatEngine`、`IncrementStrategy`、`MemorySerialBackend`、`SerialError`、`SerialGenerator`、`SerialRecord`、`SerialRule` | `alun-utils`   | 工具类型与单号生成体系                           |
| `SerialPlugin`                                                                                                    | `alun-plugin`  | 需 `plugin` feature                     |
| `Result`、`Error`                                                                                                    | `alun-core`    | 骨架层错误（非 HTTP 场景）                     |
| `Res`、`ResResult`、`ApiError`、`PageQuery`、`PageData`、`codes`                                                    | `alun-web`     | 响应形状 + handler 唯一错误出口                |
| `Db`、`Page`、`Row`                                                                                                | `alun-db`      | 需 `db` feature                         |
| `App`、`AlunRouter`、`TokenClaims`、`TokenType`、`UserId`、`AuthClaims`、`ValidatedJson`、`JWT`、`ROUTES`、`PermissionDef`、`PERMISSION_ROUTES`、`NoAuthDef`、`NO_AUTH_ROUTES`、`PageResponseExt` | `alun-web`     | Web 层类型与分布式切片                          |
| `validate_uuid`、`validate_mobile`、`validate_password_strength`、`validate_id_card`、`validate_date`、`validate_datetime`、`validate_date_or_datetime`、`validate_email`、`validate_url`、`ValidateExt` | `alun-web`     | 9 个 validator 自定义校验函数 + 校验扩展 trait    |
| `cfg`/`config`/`try_config`/`set_config`、`db`/`try_db`/`set_db`、`cache`/`try_cache`/`set_cache`、`render_template`/`try_template`/`set_template` | `alun-web::resources` | 全局资源访问器（按 feature 门控）              |
| `AppConfig`                                                                                                       | `alun-config`  | 配置根结构体                                |
| `inventory`（`pub use inventory;`）                                                                                   | `inventory`    | 转发；宏生成的注册项写 `::alun::inventory::submit! { … }`，应用因此无需自己声明 `inventory` |
| `uuid`                                                                                                            | `uuid`         | 转发                                    |
| `web`                                                                                                             | `alun-web`     | 别名为 `alun_web`                         |
| `alun_kafka` / `alun_task` / `alun_fs` / `alun_plugin`                                                             | 同名 crate       | 整包转发（`kafka`/`task`/`fs`/`plugin` feature） |

> 顶层**不**导出 `upload_path`/`download_path`：用 `alun::web::resources::upload_path`（或直接依赖 `alun-web`）。
> `#[validate(custom(function = "validate_uuid"))]` 以函数名字符串引用，需保证该函数在当前模块可见，例如 `use alun::validate_uuid;`。

## 使用示例

```rust,ignore
// ── Proc Macro 路由 ──
use alun::{App, ApiError, Res, ResResult, ValidatedJson};

#[alun::get("/api/users")]
async fn list_users() -> Res<Vec<UserModel>> {
    Res::ok(users)
}

#[alun::post("/api/users")]
async fn create_user(ValidatedJson(req): ValidatedJson<CreateUserReq>) -> ResResult<UserModel> {
    let user = save_user(req).await?;      // prelude 的 Result 单参数，双参数用 ResResult
    Ok(Res::ok(user))
}

// ── #[controller] 标注在 impl 块上，内部方法用 #[get]/#[delete] 标注 ──
#[alun::controller("/api/admin")]
impl AdminController {
    #[alun::get("/dashboard")]
    async fn dashboard() -> Res<String> {
        Res::ok("admin dashboard".into())
    }

    #[alun::delete("/users/{id}")]
    async fn delete_user(Path(id): Path<String>) -> ResResult<()> {
        remove_user(&id).await?;
        Ok(Res::ok_empty())
    }
}

// ── 插件标记 ──
#[alun::plugin]
struct MyCustomPlugin;

// ── 权限校验注解（必须具名参数） ──
#[alun::permission(path = "/api/admin/*", method = "GET", permission = "admin:read")]
#[alun::get("/api/admin/users")]
async fn admin_list() -> Res<Vec<UserModel>> {
    Res::ok(users)
}

// ── 入口 ──
#[tokio::main]
async fn main() {
    App::new().expect("初始化失败").scan().start().await.unwrap();
}
```

***

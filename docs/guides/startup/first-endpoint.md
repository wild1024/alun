---
id: guides.startup.first-endpoint
title: 接数据库与写第一个接口
summary: 接入数据库的最小配置，以及第一个路由 + 统一响应 Res<T> + 错误直通的写法。
tags: [getting-started, database, routing, response]
status: active
owner: alun-core-team
source_of_truth: [code, examples, config/config.toml]
depends_on: [guides.getting-started]
last_reviewed: 2026-10-06
---

# 接数据库与写第一个接口

> 本文件是 [`getting-started.md`](../getting-started.md) 的 `first-endpoint` 分册；总目录与全流程顺序见 [从零到跑起来](../getting-started.md)。


## 5.1 先决条件：PG / MySQL 驱动要显式开 feature

`alun` 的 `db` feature 只带 **SQLite 驱动**。**只改 TOML、不开驱动 feature，PostgreSQL / MySQL 连不上**：
`App::start()` 建池时直接返回

```text
DbError::UnsupportedFeature { feature: "driver", dialect: "postgres" }
```

（每个驱动都挂在 `#[cfg(feature = "...")]` 上，见 `alun-db/src/factory.rs` 的 `create_driver`。）

所以换库第一步是改依赖：

```toml
[dependencies]
# 只要数据库、不要 Kafka / 任务 / 文件等重依赖时收窄 features（默认是最小集 core）
alun = { version = "0.3", features = [
    "postgres",          #  mysql 
    "chrono", "rust_decimal",  # 模型字段用到的生态类型
] }
```

驱动的 feature 都写在 `alun` 上、由它转发到 `alun-db`（`alun-web` 同名 feature 也可，两者等价）。

| 要用 | 在 `alun` 上开的 feature（底层转发到 `alun-db/同名`） | 不开会怎样 |
| ---- | ---- | ---- |
| PostgreSQL | `postgres` | 启动报 `UnsupportedFeature { feature: "driver", dialect: "postgres" }` |
| MySQL | `mysql` | 同上（`dialect: "mysql"`） |
| SQLite | `sqlite`（或只开 `db`） | —（`db` **默认已带**） |
| `Decimal` 金额列 | `rust_decimal`（或 `bigdecimal`） | 编译期：该字段类型不满足 `TypeMapper` |
| `DateTime<Utc>` / `NaiveDate` 等 | `chrono`（或 `time` / `jiff`） | 同上 |
| `#[json]` 字段 | `json` | 同上 |
| `Uuid` / `Ulid` 主键 | `uuid` / `ulid` | 同上 |

> 写 Model **不需要**额外声明 `alun-db`：`Model` derive、`CrudExt`、`sql!`、`Value` 都由
> `alun::prelude::*` 提供（门面完整转发 `alun-db` 的 prelude），派生宏与 `sql!` 还会按你
> 实际声明的 crate 解析生成代码里的内部路径。

> 生态类型 feature 缺失是**编译期**错误、不会拖到运行期——类型映射在编译期定死。
>
> 直接依赖 `alun-db`（feature 开在 `alun-db` 上）的老写法仍然可用——`alun-ledger` 就是这种；
> 只依赖 `alun` 则少一个依赖、少一处版本对齐，且 `Db` / `Model` / `CrudExt` / `sql!` 与门面
> 类型在同一个 prelude 里。
>
> 注意 `alun::prelude::Result` 是框架的 `alun_core::Result<T>`（错误 `Error`）；数据库调用返回的是
> `Result<T, DbError>`，写 DB 辅助函数时显式写 `Result<T, DbError>`（或 `std::result::Result`）。

## 5.2 三种数据库怎么填

```toml
# ---------- SQLite：零部署，name 就是文件路径 ----------
[database]
enabled = true
type = "sqlite"
name = "app.db"

# ---------- PostgreSQL ----------
[database]
enabled = true
type = "postgres"
host = "localhost"
name = "mydb"
user = "app_user"
password = "secret"
ssl_mode = "prefer"       # port 可省略，默认 5432

# ---------- MySQL ----------
[database]
enabled = true
type = "mysql"
host = "localhost"
name = "mydb"
user = "app_user"
password = "secret"       # port 可省略，默认 3306
```

`enabled = true` 之后，`App::start()` 会建连接池、跑一次 `SELECT 1`、注册全局 `db()`，
handler 里直接写 `db().find(...)` 就行，**不需要把连接当参数传来传去**。

**实战提问：从 SQLite 换成 PG / MySQL，一共要动哪些地方？**

| # | SQLite | PostgreSQL | MySQL |
| --- | --- | --- | --- |
| 1 | 开 `sqlite`（零部署，见 5.1） | `alun` 开 `postgres`（见 5.1） | `alun` 开 `mysql`（见 5.1） |
| 2 | 文件不存在会自建 | 先 `createdb mydb`（或 `CREATE DATABASE mydb`） | **必须先 `CREATE DATABASE mydb`**——MySQL 不会替你建库 |
| 3 | `type="sqlite"` + `name="./app.db"`（文件路径） | `type="postgres"` + `host` / `name` / `user` / `password`，`port` 省略即 5432 | `type="mysql"` + 同左，`port` 省略即 3306 |
| 4 | 无 SSL 选项 | `ssl_mode = "prefer"`，可改 `disable` / `require` / `verify-ca` / `verify-full` | **不读 `ssl_mode`**：只有 PG 会在 URL 上追加 `?sslmode=` |
| 5 | 密码随便 | 线上用 `ALUN_DATABASE_PASSWORD` 注入，别写进仓库（白名单见 3.4） | 同左 |
| 6 | 迁移单目录够用 | 三库共用一份迁移目录时，方言特有 DDL（`citext`、分区、FTS）无法共用；按库分目录，并用 profile 文件覆盖 `[database.migration] path` | 同左 |

两个**配置改不到、只能写代码**的点：

- **端口不能用环境变量覆盖**：白名单只有 `ALUN_DATABASE_HOST` / `_NAME` / `_USER` / `_PASSWORD` / `_SSL_MODE`
  （`alun-config/src/env.rs` 的 `merge_env_overrides` 逐项匹配，没有 `port`）。换端口请改 `config.toml` 或用 profile 文件。
- **连接池的 `idle_timeout` / `max_lifetime` 映射不到**：走 `[database]` 时它们固定为 `None`（连接不回收）。
  需要精细控制就改用代码初始化：`Db::connect_with(url, alun_db::DatabaseConfig { pool: PoolConfig { .. }, .. })`。

## 5.3 表结构怎么建：迁移

```toml
[database.migration]
enabled = true
path = "migrations"
auto_migrate = true       # 开发环境方便；生产建议 false，迁移作为发布流水线的独立步骤
```

```text
migrations/
  V1__create_users.sql           # 只有 up
  V2__add_users_age.up.sql       # 成对 up / down
  V2__add_users_age.down.sql
```

**实战提问：数据库相关的最常见四条报错是什么？**

| 报错 | 原因 | 处理 |
| ---- | ---- | ---- |
| 启动报 `UnsupportedFeature { feature: "driver", dialect: "postgres" }` | 只改了 TOML，没开驱动 feature | 按 5.1 在 `alun` 上加 `postgres` / `mysql`（或 `features = ["full"]`，它已含三驱动） |
| SQLite 报找不到表 / 文件为空 | 没建表，也没开迁移 | 开 `auto_migrate`，或手写一条 `V1__init.sql` |
| PG 报 `password authentication failed` | 密码/用户写错，或 `pg_hba.conf` 不允许该来源 | 用 `psql` 手工连一次验证账号；生产用环境变量注入密码 |
| MySQL 报 `Unknown database` | 库还没建（MySQL 不会自动建库） | 先 `CREATE DATABASE mydb`，再启动应用 |

**实战提问：一个进程要连第二个库（读写分离 / 多租户）怎么办？**
配置文件只能描述一个库。用代码初始化另一个库并自己持有句柄：
`let other = alun_db::Db::connect("postgres://...").await?;`，按业务把它传进函数。

更细的数据库用法（Model、CRUD、分页、事务、批量、迁移策略）见 [database.md](../database.md) 及其分册。

---

# 6. 第 6 步：写第一个接口

## 6.1 路由：四种写法，按项目阶段选

| 写法 | 长相 | 什么时候用 |
| ---- | ---- | ---- |
| **宏注解 + `scan()`** | `#[alun::get("/")]` + `.scan()` | **默认选择**：handler 分散在多个 `*.rs`，不想写注册代码 |
| 控制器分组 | `#[alun::controller("/api/admin")]` 标在 **impl 块**上 | 一组接口共享前缀/中间件 |
| Builder 链式 | `App::new()?.get("/", h)` | 接口 ≤ 10 个、示例代码、验证想法 |
| 模块显式注册 `#[alun::routes]` | `#[alun::routes]` 标在 `mod` 上 + `.module(mod::register)` | 可选通道：库形态分发、要完全掌控装配顺序，不依赖编译期收集 |

```rust,no_run
use alun::prelude::*;

struct AdminController;

#[alun::controller("/api/admin")]
impl AdminController {
    // 宏会摘除内部属性并在 impl 之外生成注册代码，handler 为 `类型::方法名`
    #[alun::get("/dashboard")]
    async fn dashboard() -> Res<String> {
        Res::ok("admin dashboard".into())
    }
}

# async fn demo() -> Result<()> {
App::new()?.scan().start().await
# }
```

子路由手动装配（分组、复用前缀）用 `AlunRouter`：

```rust,no_run
use alun::prelude::*;

# async fn list_users() -> Res<String> { Res::ok(String::new()) }
# async fn create_user() -> Res<String> { Res::ok(String::new()) }

fn user_routes() -> AlunRouter {
    let mut r = AlunRouter::new();
    r.add_get("/", list_users);
    r.add_post("/", create_user);
    r
}

# async fn demo() -> Result<()> {
App::new()?.merge("/api/users", user_routes()).serve("8080").await
# }
```

## 6.2 响应：统一用 `Res<T>`，出口只有一个 `ResResult<T>`

handler 的返回类型**只有一种**：`ResResult<T>`（= `Result<Res<T>, ApiError>`），
序列化成统一 JSON。（不需要返回错误时可以直接写 `Res<T>`，两者是同一套响应体。）

**业务端只需要三个名字**：`Res` / `ResResult` / `ApiError`。数据库（`DbError`）、
框架（`alun_core::Error`）、IO（`std::io::Error`）的错误都由内置 `From` 映射，
在 handler 里直接 `?` 就行，不需要写 `map_err`：

```rust,no_run
use alun::prelude::*;

#[derive(serde::Serialize)]
struct UserRes {
    id: String,
    name: String,
}

/// 成功。
async fn ok() -> Res<UserRes> {
    Res::ok(UserRes { id: "u1".into(), name: "张三".into() })
}

/// 成功 + 自定义消息。
async fn ok_msg() -> Res<String> {
    Res::ok_with_msg("u1".to_string(), "创建成功")
}

/// 成功但无数据。
async fn ok_empty() -> Res<()> {
    Res::ok_empty()
}

/// 分页：data = { list, total, page, page_size }
async fn page(Query(pq): Query<PageQuery>) -> Res<PageData<Vec<UserRes>>> {
    Res::page(Vec::<UserRes>::new(), 0u64, pq.page, pq.page_size)
}

/// 失败：返回 Err(ApiError)，前端拿到统一错误体。
async fn bad(Path(id): Path<String>) -> ResResult<UserRes> {
    if id.is_empty() {
        return Err(ApiError::bad_request("ID 不能为空"));
    }
    Err(ApiError::not_found("用户不存在"))
}
# async fn demo() -> Result<()> {
App::new()?.serve("8080").await
# }
```

`ApiError` 常用构造：`bad_request` / `unauthorized` / `forbidden` / `not_found` / `conflict` /
`unprocessable_entity` / `too_many_requests` / `service_unavailable` /
`internal(secret_detail)`（自动屏蔽成「服务器内部错误」）/ `internal_masked(public_msg, detail)`。

> **下层错误怎么进来**：`From<DbError>`（不存在 → 404、唯一冲突 → 409、其余 → 500 脱敏）、
> `From<alun_core::Error>`、`From<std::io::Error>` 都是内置的，所以
> `async fn h(Path(id): Path<i64>) -> ResResult<User> { Ok(Res::ok(load(id).await?)) }` 成立。
> 数据访问层函数签名写 `DbResult<T>`（`DbError` 只在这一层出现）；
> 非 HTTP 场景（启动 / 插件 / 定时任务）才是 `alun::Result<T>`，那里收不下 `DbError`。

## 6.3 取参：用 axum 的提取器

`Path` / `Query` / `Json` / `Extension`（拿当前登录用户）/ `ValidatedJson`（带校验）都可以直接用。

```rust,no_run
use alun::prelude::*;
use axum::Json;

#[derive(serde::Deserialize)]
struct CreateUserReq {
    name: String,
}

#[alun::post("/api/users")]
async fn create(Json(req): Json<CreateUserReq>) -> Res<String> {
    Res::ok(req.name)
}
```

## 6.4 中间件默认就开了一条：安全响应头

`[middleware.security_headers] enabled = true` 是默认值，启动时自动注入
`X-Content-Type-Options` / `X-Frame-Options` / HSTS / CSP / Referrer-Policy 等安全头，**零代码**。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [② 配置与启动](config-basics.md) | [④ 中间件](middleware.md) → |

<sub>本分册属于「从零到跑起来（8 步分册）」，共 8 篇。首篇 / 末篇可点「返回总目录」。</sub>

---
name: alun
description: >-
  Help users develop web applications and services using the alun Rust web framework
  (version 0.3). alun is a configuration-driven, plugin-architected Rust web framework
  built on axum + tokio, using "Just Config + Just Code".
  Use this skill whenever the user writes, debugs, or asks questions about alun-based
  Rust projects — including routing (`#[alun::get]` / `#[alun::routes]` / `#[alun::controller]`),
  the `Res<T>` unified response and `ApiError`, database CRUD (`Db` / `Row` /
  `#[derive(Model)]` / `sql!` / transactions / migrations / ModelQuery / ChangeSet / Hooks),
  JWT authentication, RBAC permissions, middleware stack, caching (`LocalCache` / Redis /
  `CacheNamespace`), templates (minijinja), async tasks, Kafka, file storage, utilities
  (`alun-utils`: IDs, masking, validation, crypto, serial numbers, CSV/JSON/XLSX export
  and import), SSE real-time push, metrics/Prometheus observability, plugin lifecycle,
  feature flags (`core` / `mini` / `lite` / `full`), and configuration management.
  A project depends on the single `alun` facade crate — routing, config and `#[derive(Model)]`
  all come from `alun::prelude::*`, without declaring `alun-db` / `alun-db-core`.
  Also use this skill when the user mentions alun internals (alun-core, alun-web, alun-db,
  alun-db-core, alun-db-sql, alun-db-macro, alun-db-build, the alun-db-driver-* crates,
  alun-task, alun-cache, alun-config, alun-kafka, alun-fs, alun-plugin, alun-template,
  alun-utils, alun-macros, alun-log), when building multi-tenant (SaaS) apps — tenant
  isolation, `#[tenant]` / `#[model(cross_tenant)]`, `TenantContext::{Scoped, Cross}`,
  `tenant_scope`, `raw_tenant_annotated`, `[tenant]` config,
  when doing observability (`[metrics]` config, `/metrics` endpoint, Prometheus),
  when exporting/importing Excel (`xlsx` feature, `Export::to_xlsx_*`, `Import::from_xlsx*`),
  when pushing events in real time (SSE, `Broadcaster`, `sse_json`),
  or any alun-specific types/annotations like `App`, `AlunRouter`, `Res`, `Db`, `Row`,
  `PageQuery`, `TenantContext`, `#[get]`, `#[post]`, `#[routes]`, `#[controller]`,
  `#[task_handler]`, `#[plugin]`, `#[permission]`, `#[hook_async]`, `#[tenant]`.
---

# Alun Framework Skill

Alun is a **configuration-driven, plugin-architected Rust web framework** built on axum + tower + tokio. Its philosophy is "Just Config + Just Code" — behavior is controlled by `config.toml`, and business logic is expressed as plain async functions with proc-macro annotations.

> **设计理念溯源**：Alun 承接 **[Aifei](https://github.com/jfinal/aifei)**（[官网 aifei.cn](https://aifei.cn)，
> 世界首个 AI Coding 框架）的 **Just Service** 范式——用「消除非业务分层」来降低模型上下文噪音。
> Alun 是该理念在 Rust 生态的表达（Just Config），并用编译期校验替代约定。解释某个 API 为何如此设计时，
> 参考 `docs/architecture/aifei-tribute.md`（含 aifei → alun 逐项映射与取舍）。

## 本 Skill 的内容地图（按需取用）

| 我想做 | 先看 |
|---|---|
| 起服务 / 选 feature / 写第一个接口 | 本文 `Quick Start` + `Routing Patterns` |
| 建表 / 写 Model / 增删改查 / 事务 / 迁移 | `references/database.md` |
| 联查 / 排序白名单 / 原生 SQL 租户安全 / 多数据源 | `references/advanced-db.md` |
| 登录鉴权 / 角色权限 / 中间件顺序 | `references/auth.md` |
| 路由三写法 / `Res<T>` / 分页响应 | `references/routing.md` |
| 配置项 / 多环境 / `ALUN_*` 环境变量 / 动态配置 | `references/config.md` |
| 自写插件 / 生命周期 / 内置插件 / 缓存 / 模板 | `references/plugins.md` |
| 启动钩子 / 自定义中间件 / `WebPlugin` / 显式路由注册 / 异步 Hook | `references/extension-points.md` |
| 导出导入（CSV / JSON / 真 XLSX） | `references/export-and-import.md` |
| SSE 实时推送 | `references/realtime-sse.md` |
| 埋点 / Prometheus / 安全指标 | `references/observability.md` |
| 定时任务 / 分布式异步任务 / Kafka | `references/task.md` |
| 工具函数（ID / 脱敏 / 校验 / 加密 / 单号） | `references/utils.md` |

## 什么时候该用这个 Skill

出现下面任意一种情况就应加载它：
- Writes new alun-based Rust web application code
- Debugs or asks questions about alun's API, types, macros, or patterns
- Needs help with alun's routing, database CRUD, JWT auth, middleware, plugins, or config
- Mentions any alun crate or type (e.g., `App`, `Res`, `Db`, `Row`, `#[get]`, `#[controller]`)

## Core Principles (Always Keep In Mind)

1. **Configuration-Driven**: All infrastructure behavior (DB, auth, CORS, rate limiting, compression) is declared in `config.toml` — never hardcoded in `main.rs`
2. **Global Resource Access**: Use global accessor functions (`db()`, `cache()`, `cfg()`, `config()`, `render_template()`) instead of axum `State` injection. No `with_state()` needed. Upload/download dirs 用 `alun::upload_path()` / `alun::download_path()`（这两个访问器只挂在 crate 根，不在 prelude）。
3. **Zero-Cost Abstraction**: Pure Rust traits + generics, compiled at build time — no reflection, no dynamic dispatch
4. **Compiler-Enforced Safety**: Transaction rollback is guaranteed by the driver handle's `Drop` + the `?` operator. Error handling is type-safe.
5. **Progressive Enhancement**: Start from `App::new()?.get("/", h).serve("8080").await` and layer on features as needed
6. **Security by Default**: Security headers auto-injected. 5xx errors masked from clients — details go only to logs.

## Project Architecture

The alun workspace consists of 22 library crates（`examples/*` 与 `docs-tests` 不计）:

| Layer | Crate | Purpose |
|-------|-------|---------|
| **Facade** | `alun` | 项目**唯一需要声明的依赖**。`alun::prelude::*` 转发 `alun-web` + `alun-core` + `alun-config` + `alun-utils`，并在 `db` feature 下**完整转发 `alun-db::prelude::*`**（`Model` / `sql!` / `CrudExt` / `ModelQuery` / 生态类型 …）——写 Model 不必声明 `alun-db` / `alun-db-core`。 |
| **Core** | `alun-core` | Zero web-framework dependency. `Error`/`Result`, `Plugin`/`PluginManager`, `Res<T>`, `ApiError`, `ResResult<T>`, `PageData<T>`, `PageQuery`, `codes`, `DEFAULT_PAGE_SIZE`/`MAX_PAGE_SIZE`. |
| **Features** | `alun-web` | `App` builder, `AlunRouter`, middleware stack, JWT, global singletons, `ValidatedJson`, `PageResponseExt`. |
| | `alun-db` | `Db` facade, dynamic `Row` CRUD, heterogeneous batch via `RowBatchExt`, RAII transactions, `CrudExt`/`PersistExt` for `Model`, `ModelQuery` chain + `ChangeSet`, `Hook` lifecycle, `sql!` fragments + `SqlQueryExt`, builders, migrations. |
| **DB internals** | `alun-db-core` | Core types: `Row`, `Value`/`Type`, `Expr`/`Statement`, `Dialect`/`Driver`, `Hook`/`Model`/`TypeMapper` traits, `Page<T>`, `DbError`. |
| | `alun-db-sql` | SQL rendering / AST helpers. |
| | `alun-db-macro` | `#[derive(Model)]` / `#[derive(Row)]` / `#[derive(TypeMapper)]` and the `sql!` macro. |
| | `alun-db-build` | `build.rs` schema snapshot (`build_snapshot`) used with `#[model(strict)]`. |
| | `alun-db-driver-sqlite` | SQLite driver. |
| | `alun-db-driver-postgres` | PostgreSQL driver. |
| | `alun-db-driver-mysql` | MySQL driver. |
| **Infrastructure** | `alun-config` | TOML config, multi-profile, `ALUN_*` env overrides, runtime dynamic config. |
| | `alun-log` | tracing init (text/json/file). |
| | `alun-macros` | Proc macros: `#[get]`, `#[post]`, `#[put]`, `#[patch]`, `#[delete]`, `#[controller]`, `#[routes]`, `#[plugin]`, `#[permission]`, `#[no_auth]`, `#[layer]`（route-level layer，写在路由注解上方）, `#[task_handler]`. |
| | `alun-utils` | 200+ utilities: strings, dates, masking, IDs, validation, crypto, export, XSS, serial number generation. |
| **Extensions** | `alun-cache` | `Cache` trait, `LocalCache`, `SharedCache` (Local/Redis). |
| | `alun-template` | minijinja (Jinja2) template engine. |
| | `alun-plugin` | Built-in plugins: cache, notification (SMTP), async-task, scheduler (cron), serial number generator, sid (短 ID)。 |
| | `alun-kafka` | Kafka producer/consumer (rdkafka). |
| | `alun-task` | Kafka-driven distributed async task engine with retry/DLQ. |
| | `alun-fs` | File storage abstraction (Local / MinIO). |
| **Reference app** | `alun-ledger` | `publish = false` 的多租户计费/账务参考实现（`alun` + `alun-db` 融合验证）；不发布、不属于门面 API，但可作为 `Model` / 事务 / Decimal 用法的范本。 |

## Feature Flags

最小依赖：**只声明 `alun`**。路由、配置、`Res`/`ApiError` 由 `alun::prelude::*` 提供；开启 `db` 后 `Model` 派生宏、`sql!`、`CrudExt`/`PersistExt`、`ModelQuery`/`ChangeSet` 等也从同一个 prelude 出——**不需要**在 `Cargo.toml` 里声明 `alun-db` 或 `alun-db-core`：

```toml
[dependencies]
# 门面：默认 = `core`（HTTP 能力，零外部重量级依赖）；按需叠加能力。
# 用数据库必须显式选驱动 —— `db` 只是"数据库能力"，**不含任何驱动**。
alun = { version = "0.3", features = ["postgres"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
validator = { version = "0.21", features = ["derive"] }
```

```toml
# ── 分层叠加（推荐直接用预设）──
alun = { version = "0.3" }                                    # core：App / 路由 / 中间件 / 配置 / 日志
alun = { version = "0.3", features = ["mini"] }                # 预设：+ SQLite + 本地缓存（零外部服务）
alun = { version = "0.3", features = ["lite"] }                # 预设：+ PostgreSQL + cache/template/plugin/fs（推荐）
alun = { version = "0.3", features = ["full"] }                # 预设：全部能力（含 MySQL / Kafka / MinIO）

# ── 单点叠加 ──
alun = { version = "0.3", features = ["postgres"] }                        # 驱动（隐含 db）
alun = { version = "0.3", features = ["postgres", "cache"] }               # + 缓存
alun = { version = "0.3", features = ["sqlite", "rust_decimal", "xss"] }   # SQLite + decimal + 净化
alun = { version = "0.3", features = ["postgres", "fs", "fs-minio"] }      # + 本地/对象存储
alun = { version = "0.3", features = ["task"] }                            # 任务模型（**无 Kafka**，零外部依赖）
alun = { version = "0.3", features = ["task-kafka"] }                      # 任务 + Kafka worker（需 CMake + C 工具链）
```

| Feature | Enables |
|---------|---------|
| `core` | **默认**。HTTP 服务最小集：路由 + 中间件 + 配置 + 日志，无外部重量级依赖 |
| `config` | 配置加载（转发 `alun-web/config`；`core` 已隐含） |
| `db` | `alun-db` 门面 —— **数据库能力，不含任何驱动** |
| `sqlite` / `postgres` / `mysql` | 对应驱动（各自隐含 `db`）。只开 `db` 不选驱动 → 建池报 `UnsupportedFeature { feature: "driver", … }` |
| `chrono` / `time` / `jiff` / `rust_decimal` / `bigdecimal` / `uuid` / `ulid` / `json` / `ipnetwork` | 模型生态类型映射（隐含 `db`）；缺失是编译期错误 |
| `xss` | HTML 净化：转发 `alun-utils/xss`，prelude 里 `xss::sanitize_html(..)` |
| `template` | `alun-template`（minijinja） |
| `cache` | `alun-cache`（Local / Redis） |
| `plugin` | `alun-plugin`（scheduler / notification / serial / sid） |
| `fs` | 文件存储能力，等价 `fs-local` |
| `fs-local` / `fs-minio` | 存储后端（本地 / 对象存储），正交于 `fs` |
| `kafka` | `alun-kafka` |
| `task` | `alun-task` 的**任务模型**（`#[task_handler]` 收集 / 存储抽象 / 类型 / 重试策略）——**零外部依赖** |
| `task-kafka` | 在 `task` 之上再加 Kafka worker 与 `TaskPlugin`——**需 CMake + C 工具链**（librdkafka） |
| `metrics` | **F10 埋点**：HTTP 计数器/直方图/在途 gauge + DB 查询 + 缓存命中 + 多租户安全指标。仅 `metrics` 门面，**未装 recorder 时是 no-op**，可安全常开 |
| `metrics-prometheus` | 在 `metrics` 之上安装 Prometheus recorder 并挂 `/metrics` 端点（默认**关闭**，见 `[metrics].enabled`） |
| `xlsx` | 真 XLSX（OOXML）：转发 `alun-utils/xlsx`，`Export::to_xlsx_*` / `Import::from_xlsx*` 才存在 |

**预设**（能力严格递增：**`mini` ⊂ `lite` ⊂ `full`**）：

| 预设 | 组成 | 场景 |
|------|------|------|
| `mini` | `core` + `sqlite` + `cache` | 单机自足，**零外部服务**（原型 / 小工具 / 测试）|
| `lite` | `core` + `postgres` + `cache` + `template` + `plugin` + `fs-local` | **单库生产（推荐）** |
| `full` | `lite` + `mysql` + `kafka` + `task-kafka` + `fs-minio` + `sse` + `metrics-prometheus` + `xlsx` | 全部能力，构建最重 |

> 分层纪律由 `ci/check_feature_discipline.sh`（15 条断言）守卫：默认集混入驱动 / Kafka / 对象存储即失败。

三点补充：

- `xss` **是** `alun` 的 feature（`xss = ["alun-utils/xss"]`）：开启后 `alun::prelude::xss::sanitize_html(..)` 与 `alun::xss` 都可用。
- 生态类型 feature（`chrono` / `rust_decimal` / `uuid` …）开启后经 prelude 引用即可：`alun::prelude::chrono::{DateTime, Utc}`、`alun::prelude::rust_decimal::Decimal`、`alun::prelude::uuid::Uuid`、`alun::prelude::ulid::Ulid`、`alun::prelude::bigdecimal::BigDecimal`、`alun::prelude::serde_json::{json, Value}`、`alun::prelude::{time, jiff, ipnetwork}` —— **不必**自己声明 `chrono` / `uuid` / `rust_decimal` crate。
- 路由 / 任务宏的注册项由门面转发 `inventory`（生成代码用 `::alun::inventory::submit! { … }`）——**不需要**自己声明 `inventory`。
- ⚠️ **历史坑（0.3 已修，现在不用绕开）**：早先的 `linkme` 分布式切片在部分工具链（实测 windows-gnu + rustc nightly）会**静默失效** —— 编译链接均无报错，表现为全站 404。0.3 已把收集机制换成 `inventory`（不依赖链接器自定义段，不受 `--gc-sections` 影响），**`App::scan()` 重新成为可靠的默认通道**，见 `references/extension-points.md` 第 5 节。
- 例外：`#[model(strict)]`（编译期 schema 校验）用构建期工具，门面不转发 → 需在 `[build-dependencies]` 声明 `alun-db-build`；`#[value_map(serde, …)]` 会生成 serde 实现，需应用自己声明 `serde`。另外 `alun_db::init` / `init_with` / `alun_db::db()` / `try_db()` 是 `alun-db` 自带全局实例的独立用法，与框架的 `alun::db()` 不是同一个实例，**不要**在框架项目里调用。

## Quick Start

```rust,no_run
use alun::prelude::*;

#[alun::get("/")]
async fn hello() -> Res<String> {
    Res::ok("Hello, alun!".into())
}

#[tokio::main]
async fn main() {
    App::new()
        .expect("初始化失败")
        .parse_cli()   // 识别 CLI：gen-config / print-config / profile=xxx（不调用则 `cargo run -- gen-config` 无效）
        .scan()        // 自动发现 #[alun::get] / #[alun::post] 处理器
        .start()       // 读取 config/config.toml、初始化资源、启动服务器（端口取自 [server].listen）
        .await
        .unwrap();
}
```

```bash
cargo run -- gen-config   # 在 config/ 生成默认 config.toml（需 main 里调用 .parse_cli()）
cargo run                 # 默认监听 0.0.0.0:8023，日志显示 http://127.0.0.1:8023
```

## Routing Patterns → see `references/routing.md`

按推荐度从上往下：

- **`#[alun::get("/path")]` + `App::scan()`** ⭐ **默认 / 绝大多数项目**：
  handler 分散在几个 `*.rs` 都不用管，只在 `main` 里加一行 `.scan()`。
  ```rust
  #[alun::get("/users")]
  async fn list_users() -> Res<String> { Res::ok("[]".into()) }

  App::new()?.parse_cli().scan().start().await?;
  ```
- **`#[alun::controller("/prefix")]` 分组**：一组接口共享前缀时用在 impl 块上。
- **Builder chaining**: `App::new()?.get("/api/users", h).post(...).patch(...).serve("8080").await`（注册方法 `.get/.post/.put/.patch/.delete/.route`；`start()` 读取配置端口，`serve(addr)` 用显式地址）。也可 `App::routes(fn(&mut AlunRouter))` 批量注册。
- **`#[alun::routes]` + `App::module()`** （**可选通道**，非默认）：`#[alun::routes] mod api` 由宏生成 `api::register`，`.module(api::register)` 挂上——它不依赖任何编译期收集，**只在「分发带接口的库 / 插件」或「必须自己掌控装配顺序」时才用**；普通应用不必上：
  ```rust
  #[alun::routes]
  mod api {
      use alun::prelude::*;
      #[alun::get("/users")]
      pub async fn list_users() -> Res<String> { Res::ok("[]".into()) }
  }
  App::new()?.scan().module(api::register).start().await?;   // 两条通道可混用，scan() 先、module() 后
  ```
- **Controller grouping**: `#[alun::controller("/api/admin")]` 标在 **`impl` 块**上（不是函数上），块内方法用 `#[alun::get("/path")]` 标注：

```rust,no_run
use alun::prelude::*;

struct AdminController;

#[alun::controller("/api/admin")]
impl AdminController {
    #[alun::get("/dashboard")]
    async fn dashboard() -> Res<String> {
        Res::ok("admin dashboard".into())
    }

    #[alun::delete("/users/{id}")]
    async fn delete_user(Path(id): Path<String>) -> ResResult<()> {
        let _ = id;
        Ok(Res::ok_empty())
    }
}
```

Use standard axum extractors (`Path`, `Query`, `Extension`, `ValidatedJson`).

## Unified Response (`Res<T>`) → see `references/routing.md`

All handlers return `ResResult<T>` (= `Result<Res<T>, ApiError>`) —— **这是 handler 唯一的签名**：
- `Res::ok(data)` → `{ code:0, msg:"ok", data }`；`Res::ok_with_msg(data, msg)`、`Res::ok_empty()`、`Res::ok_msg(msg)`
- `Res::page(list, total, page, page_size)` for pagination → `data = { list, total, page, page_size }`
- `Res::fail(code, msg)` for a standalone failure body（`codes::BAD_REQUEST` 等常量在 `alun::prelude` 中）
- `Err(ApiError::bad_request("msg"))` / `.unauthorized()` / `.forbidden()` / `.not_found()` / `.method_not_allowed()` / `.conflict()` / `.unprocessable_entity()` / `.too_many_requests()` / `.internal(...)` / `.internal_masked(public, detail)` / `.service_unavailable(...)`（全部构造器共 11 个；另有底层 `ApiError::new(status, code, msg)`）
- **下层错误直通（无需 `map_err`）**：`From<DbError> for ApiError`（404 / 409 / 500 语义）、`From<alun_core::Error>` 与 `From<io::Error>`（脱敏 500）全部内置，所以 `fn h() -> ResResult<T> { Ok(Res::ok(repo.load(id).await?)) }` 成立。只有**非 HTTP 场景**的 `alun::Result<T>` 收不下 `DbError`（那里用 `DbResult<T>`）——`ApiError` 住在 `alun-web` 是本地类型，孤儿规则天然满足（映射表见 `references/database.md` §错误）

## Database → see `references/database.md`（进阶：联查 / 排序白名单 / 原生 SQL 租户安全 / 命名数据源见 `references/advanced-db.md`）

Requires `features = ["db"]`. Key patterns（`Row` / `Value` / `Model` / `sql!` / `CrudExt` … 全部来自 `alun::prelude::*`，**无需**声明 `alun-db` / `alun-db-core`）:
- Dynamic `Row` CRUD: `let mut row = Row::new().with_entity("users", "id"); row.insert("id", Sid::uuid7());` → `db().insert_row(&row)`、`db().find_row_by_pk("users", "id", pk)`、`db().update_row(&row)`、`db().delete_row(&row)`
- Raw SQL: `db().find(sql)` / `db().find_first(sql)` / `db().scalar::<i64>(sql)` / `db().execute(sql)`（占位符参数用 `db().execute_with_params(sql, Vec<Value>)` / `db().find_with_params(...)` 或 `sql!` 宏）；`db().query()` / `db().page(page, size)` / `db().count()` 返回 `FindBuilder` / `PageBuilder` / `CountBuilder`
- Typed `Model`: `#[derive(Model)]`（表名 / 主键 / 时间戳 / 软删都有默认约定）+ `User::find(&db, 1)`、`user.insert_or_update(&db)`（需 `let mut user`）、`User::query().paginate(&db, page, size)`（`CrudExt` / `PersistExt` 由 facade 对 `T: Model` 的 blanket impl 提供）；链式强类型查询是 `User::query()` / `User::sql(sql!(...))` → `ModelQuery<T>`，部分更新用变更集 `User::of(1).set(User::name(), "李四").update(&db)`
- RAII Transactions: `db().transaction(|tx| async move { ... Ok(()) }).await?`；显式句柄用 `db().begin()` + `tx.commit()` / `tx.rollback()`（事务内的 SQL 必须走 `tx` 句柄）
- Hooks: `impl Hook`（**同步**方法）for CRUD lifecycle callbacks；含 `created_at` / `updated_at` 字段的 Model 会**自动挂载**内置 `TimestampHook`（固定写这两列），自定义 Hook 用 `#[hook(MyHook)]` 声明（显式写出 `TimestampHook` 只用于决定它在链上的位置）
- SQL fragments: `sql!("SELECT * FROM users WHERE email = ", { email }).find(db()).await?`（插值 `{ expr }` 自动参数化，`#where/#and/#or/#orderBy/#groupBy/#limit`、`#if/#else/#end`、`#para(name)` 等指令编译期校验；终态动词 `find` / `find_first` / `find_one` / `find_one_or_null` / `exists` / `paginate` / `for_each` / `for_each_page` / `scalar` / `execute`，`SqlQueryExt` 与 `ModelQuery` 同集）
- Pagination: 全框架唯一的 `PageQuery`（`alun-core`，反序列化即规整）+ `db().paginate(sql, pq.page, pq.page_size)` → `Page<Row>`（字段 `list` / `total` / `page` / `size` / `pages`）；响应侧用 `PageResponseExt::into_res()` / `into_page_data()`
- Migrations: `Migrator::from_dir(Path::new("migrations")).run(&db)`（单目录），或 `Migrator::new().source_dir(a).source_dir(b).run(&db)`（多来源），或 `Migrator::from_migrations(vec![Migration::new(...)]).run(&db)`（内存清单）；回退用 `.rollback(&db)`（`Migrator` / `Migration` / `MigratorOptions` 同来自 prelude；`alun-db` 自带全局实例的 `alun_db::init` / `alun_db::db()` 不要在框架项目里用）

## Authentication & Authorization → see `references/auth.md`

Configured in `[middleware.auth]`（`enabled` + 非空 `jwt_secret` 才挂载 `AuthLayer`）:
- `JWT::from_config()` for token management（`create_access_token` / `create_refresh_token` / `validate` / `refresh` / `logout`）
- `Extension(AuthClaims(claims))` to access current user in handlers（`AuthClaims` 是包装 `TokenClaims` 的元组结构体；`UserId` 亦可直接提取）
- `claims.has_role()` / `has_any_role()` / `has_all_roles()` / `has_permission()` / `has_any_permission()` / `is_super_admin()`（`*` 或 `*:*:*` 通配）for RBAC checks
- `App::with_permission(method, path, handler, permission)` / `App::with_role(...)` or `#[alun::permission(path = "...", method = "...", permission = "...")]` for guards（宏规则始终生效，不受 `[middleware.permission] enabled` 影响）
- `#[alun::no_auth("/path")]` to skip auth on specific endpoints（收集进 `NO_AUTH_ROUTES`，与 `[middleware.auth] ignore_paths` 的精确/前缀匹配合并）

> ⚠️ **路径前缀规则（`app.rs::with_prefix`，`no_auth` 与 `[tenant] ignore_paths` 通用）**：**不以 `!` 开头**的路径会被自动前置 `[router] prefix`（默认 `/api`）——所以写 `"/system/auth/login"`，**不要**写 `"/api/system/auth/login"`（会变成 `/api/api/...`）；**以 `!` 开头**表示绝对路径、不加前缀（如 `"!/health"`，因为 `/health` 注册在 `/api` 之外）。漏写 `!` 是最常见的「忽略规则永不命中」原因。

## Multi-Tenancy (多租户，框架已内置)

`alun-new` **已内置多租户隔离**（`alun-db/tenant.rs` + `alun-web/middleware/tenant.rs` + `[tenant]` 配置）。做 SaaS / 企业版时**不要手写 `AND tenant_id = ?`，直接用框架机制**，遵循「声明 → 注入 → 豁免」范式（复刻软删除 `soft_filter` 的设计）。

**总开关 `[tenant].enabled = true` 时 `App` 自动挂载 `TenantLayer`，业务代码零改动。** 开关拉起时在 `app.rs` 中同步进程级全局变量（`set_tenant_enabled` / `set_tenant_write_enabled` / `set_tenant_schema_enabled` / `set_tenant_schema_prefix`）。

### 1. 模型声明

```rust
// ① 参与隔离的模型：字段级 #[tenant] 标记租户列
#[derive(Model)]
#[table("order")]
pub struct OrderModel {
    #[key] id: i64,
    customer: String,
    #[tenant] tenant_id: i64,   // 至多一个；类型无关，值以 Value 承载
    /* … */
}

// ② 跨租户的平台域模型：结构体级 #[model(cross_tenant)]，此时**不能**再有 #[tenant] 字段
#[derive(Model)]
#[table("sys_dict")]
#[model(cross_tenant)]
pub struct SysDictModel { #[key] id: i64, code: String, /* … */ }
```

- **字段属性** `#[tenant]`：标记租户列（一张表至多一个，与 `#[model(cross_tenant)]` **互斥，同时写会编译报错**）。
- **结构体属性** `#[model(cross_tenant)]`：`CROSS_TENANT = true`，模型级豁免，恒不隔离（平台表、字典、配置等）。
- ⚠️ **不存在「结构体级 `#[tenant]`」**——只有「字段级 `#[tenant]`」+「结构体级 `#[model(cross_tenant)]`」两种，且二者互斥，写错编译不过。
- ⚠️ `cross_tenant` 表里**仍可以有 `tenant_id` 列**（如按租户统计的配额表），但框架**不会**自动加条件——此时租户条件必须**自己手写**。`cross_tenant` 的含义是「自动隔离关闭」，不是「无租户概念」。
- 读路径由 `scope_for::<T>()` 决定：`#[tenant]` + `Scoped(id)` → `ScopeMode::Scoped(ScopeFilter{table,column,value})`；`Cross` 或模型豁免 → `ScopeMode::Cross`。

### 2. 上下文与作用域

- `TenantContext::{Scoped(Value), Cross}`——**用枚举而非 `Option`**，区分「限定到某租户」与「显式跨租户」，避免 `None` 被误读为「不过滤」。
- `tenant_scope(ctx, fut).await`——在指定租户上下文中执行 future（定时任务 / CLI / MQ 消费 / 测试等无 HTTP 上下文的场景必用）。
- `current_tenant() -> Option<TenantContext>`——读取当前上下文（未设置返回 `None`）。
- 上下文存在 **`tokio::task_local!`** 而非 `thread_local!`：async 在 `await` 点会跨线程迁移，用 `thread_local!` 会导致 A 请求的租户被 B 请求读到（串租户）。这是框架已规避的关键缺陷。

### 3. 天然的三条防线

| 防线 | 行为 |
|---|---|
| **读路径** | 租户模型自动追加 `tenant_id = ?`；**fail-closed**：模型需要隔离但无租户上下文 → `DbError::InvalidValue`，**绝不静默放行全量数据** |
| **写路径** | INSERT 经 `resolve_tenant_value` / `fill_row_tenant` 补齐租户值；行内值占位→补、一致→保留、**不一致→报错（防越权写入）**；`ChangeSet`、整行 update、批量插入等手写 SQL 写路径用 `scoped_predicate` 追加谓词 |
| **原生 SQL** | `ModelQuery::from_sql` / `T::sql(..)` 绕过 AST 无法自动追加条件 → `ensure_raw_allowed::<T>()` 在「租户模型 + 非跨租户上下文」下**直接拒绝**，提示改用 `Model::query()` 或包 `tenant_scope(TenantContext::Cross, ..)` |

**灰度开关**：`[tenant].write_enabled = false` 时读路径照常隔离、写路径完全惰性（不补列/不校验/不追加 WHERE），便于与既有手写 `AND tenant_id = ?` 并存过渡；**勿长期关闭**（此时租户值由业务代码负责，框架不再兜底）。

### 4. 配置 `[tenant]`（`TenantConfig`，`alun-config/src/lib.rs`）

```toml
[tenant]
enabled = true          # 总开关（默认 false）
write_enabled = true    # 写路径补齐/校验（默认 true，仅 enabled 时生效）
claim = "tenant_id"     # JWT 中承载租户 ID 的字段名（默认 tenant_id）
column = "tenant_id"    # 默认租户列名（默认 tenant_id）＊仅脚手架用，不参与隔离判定
ignore_paths = ["/api/login", "/health"]   # 跳过上下文注入（登录/健康检查），不含路由前缀
schema_per_tenant = false  # PG schema 级隔离（默认 false，需预建各租户 schema）
schema_prefix = "tenant_"  # schema 名 = 前缀 + 租户 ID（默认 tenant_）
```

### 5. 中间件顺序（**关键约束**）

`TenantLayer` **必须先于 `AuthLayer` 添加**（成为认证层的**内层**）：请求先经认证层验签并注入 `AuthClaims`，租户层才据此信任载荷里的租户 claim；顺序颠倒会让**未验证的载荷被采信**，直接造成租户越权。`App` 已按此顺序处理（`app.rs` 中 tenant 层在 `if cfg.middleware.auth.enabled …` 之前插入），手写中间件时务必遵守。

- `TenantLayer { claim, ignore_paths }` 从 claim 解析租户并 `task_local` 包裹下游；`ignore_paths` 命中则跳过注入。引用路径 `alun::web::middleware::tenant::TenantLayer`（门面把 `alun_web` 重导出为 `alun::web`；**prelude 不含**，与 `AuthLayer` / `RequireRoleLayer` / `RequirePermissionLayer` 同理）。
- 业务代码通常**无需手动挂载**——`enabled = true` 时 `App` 自己挂；只有手写 router 时才需要显式 `layer(TenantLayer { .. })`。

### 6. ⚠️ 框架未提供、必须业务侧自己解决的三件事

1. **`TokenClaims` 里没有 `tenant_id`**（`alun-web/src/middleware/mod.rs` 现字段仅 `jti / sub / username / roles / permissions / token_type / exp / iat`），且 `JWT::create_token` 用字面量组装载荷 → **租户上下文传不进请求**。要让 token 携带租户，须：给 `TokenClaims` 加 `#[serde(default)] pub tenant_id: Option<String>`、新增 `create_access_token_with_tenant(.., tenant_id)`（或等价方法）、并让 `refresh` 透传 `claims.tenant_id`；**兜底**：业务侧用 `jsonwebtoken` 直接自签同结构 token（临时方案，需标注待框架补齐）。
2. **缓存 key 不会自动加租户前缀**——`cache().get/set` 仍是全局扁平命名空间，多租户下同类 key 会互相覆盖。需自行封装 `cache_key::tenant(suffix)`：无租户上下文时**返回 `Err`（禁止写出无前缀 key）**，定时任务的 key 必须用 `tenant_scope` 包裹后再拼前缀。
3. **文件/对象存储无租户语义**——`alun-fs` 只有 `StorageBackend` / `LocalFs` / `BackendRegistry` / `BackendConfig{backend_type,endpoint,region,access_key,secret_key,use_tls,root_path,is_default}`，不感知租户。隔离要自己做：单桶 + `t/{tenant_id}/yyyy/mm/` 路径前缀，或每租户独立桶，或走 PG `schema_per_tenant`。

> **`alun-fs` 的 `minio` 子 feature 不在门面里**：`alun-fs` 自身 `default = ["local"]`、`minio = ["dep:aws-sdk-s3", "dep:aws-config", "dep:http"]`，而门面 `alun` 只写了 `fs = ["dep:alun-fs"]`（**未开 `minio`**）。要 MinIO 支持，需给门面补 `fs-minio = ["alun-fs/minio"]` 之类的补丁，或业务侧自己实现 `alun::alun_fs::StorageBackend`。

### 7. 后台管理（跨租户）的正确姿势

平台端 / 运维后台查询必须显式跨租户：

```rust
tenant_scope(TenantContext::Cross, async {
    // 此处可读写所有 #[tenant] 模型；原生 SQL 也放行
    Ok::<_, DbError>(())
}).await?
```

对照记忆：`ensure_raw_allowed` 放行的三种情况 = 模型无 `#[tenant]` / 上下文为 `Cross` / 总开关关闭。

## Middleware Stack → see `references/auth.md`

Execution order (outermost first): SecurityHeaders → CORS → RequestLog → RequestId → Auth → PermissionCheck → RateLimit → Compression → Nonce → Idempotency → handler
（Auth 在 RateLimit 外层：未认证请求先得 401、不消耗限流额度；已按源码 layer 添加顺序实测确认）
（按配置开关挂载；Idempotency / Nonce 需 `cache` feature + 对应 `[middleware.*]` 段；Auth 与 PermissionCheck 分别是 `AuthLayer` 与 `PermissionCheckLayer`）
（请求体上限由 `[upload].max_size_mb` 自动推导（`+32 MiB` multipart 余量）并装配为 `axum::extract::DefaultBodyLimit`，**不要**在业务里再挂一层；需按 `Content-Length` 前置拒绝时复用 `UploadConfig::max_request_body_bytes()` 取同一口径）

## Configuration System → see `references/config.md`

1. `config/config.toml` → 2. `config-{profile}.toml` → 3. `ALUN_*` env vars
- `cfg().server.listen` for static config; `config().set_dynamic(key, value)` / `get_dynamic(key)` / `remove_dynamic(key)` for runtime（`set_dynamic` 收 `serde_json::Value`）
- CLI: `cargo run -- gen-config`、`cargo run -- print-config` 需 main 里链 `.parse_cli()`；`cargo run -- profile=prod` 与 `ALUN_SERVER_LISTEN=3000 cargo run` 不需要（启动加载配置时即生效）

## 扩展点：启动钩子 / 自定义中间件 / `WebPlugin` → see `references/extension-points.md`

启动钩子与「链外 tower 层」共用**同一条有序装配序列**，追加而非覆盖（多个模块各自注册互不影响）：

| 扩展点 | 签名 | 用途 / 能力边界 |
|---|---|---|
| 启动钩子 | `App::on_startup(|| async { .. })`（可调多次） | 预热缓存、注册定时任务、建连接 |
| 链外 tower 层 | `App::layer(L)` / `App::layer_from_fn(fn(Router) -> Router)`（可调多次） | 覆盖整站（含 401/403/429、404、`/metrics`），**读不到身份** |
| **链内**中间件 | `App::route_layer(L)` | 只作用于匹配路由，位于 Auth/权限**之内**，**能**读 `AuthClaims` / 租户上下文 |
| 子树层 | `AlunRouter::merge_with_layer(prefix, sub, wrap)` | 只给某前缀下的整组路由挂层 |
| 单路由层 | `#[alun::layer(L)]`（写在路由注解上方）/ `AlunRouter::add_get_with_layer(..)` | 只给一条路由挂层 |

> 选择规则：**要记 401/403/429 与真实耗时 → 外层 `layer`；要用当前身份做判断 → `route_layer` 及更细的档位。**
> 旧名 `App::with_middleware_hook` 已废弃（等价 `layer_from_fn`）。

```rust,no_run
use alun::prelude::*;

App::new()?
    .on_startup(|| async { tracing::info!("预热完成"); })
    .on_startup(|| async { /* 第二个钩子也会执行 */ })
    .layer(axum::middleware::from_fn(outer_guard))
    .parse_cli()
    .start()
    .await?;

# async fn outer_guard(
#     req: axum::http::Request<axum::body::Body>,
#     next: axum::middleware::Next,
# ) -> axum::response::Response {
#     next.run(req).await
# }
```

`WebPlugin` = **`Plugin` 的子 trait**，在生命周期之上再贡献 Web 装配（`routes` / `permission_rules` /
`no_auth_paths`，三者都有默认空实现）。已有 `impl Plugin for X` 只需改成 `impl WebPlugin for X`，
再 `.web_plugin(x)` 一行注册——**库形态分发的首选通道**（一个 crate 无法指望使用方的编译期收集
拿到自己的 handler）。

```rust,ignore
use alun::web::{AlunRouter, NoAuthDef, PermissionDef, WebPlugin};
use alun::Plugin as _;              // WebPlugin: Plugin，生命周期方法来自这里

struct AdminPlugin;

#[alun::async_trait]
impl alun::Plugin for AdminPlugin {
    fn name(&self) -> &str { "admin" }
    async fn start(&self) -> alun::Result<()> { Ok(()) }
    async fn stop(&self) -> alun::Result<()> { Ok(()) }
}

impl WebPlugin for AdminPlugin {
    fn routes(&self, router: &mut AlunRouter) {
        router.add_get("/admin/health", || async { "ok" });
    }
    fn permission_rules(&self) -> Vec<PermissionDef> { /* … */ vec![] }
    fn no_auth_paths(&self) -> Vec<NoAuthDef> { /* … */ vec![] }
}

App::new()?.web_plugin(AdminPlugin).parse_cli().start().await?;
```

> `App::module(register)` = `register(self)` 的糖：`App::module(|a| a.get("/x", h).post("/y", c))`；
> `App::routes(fn(&mut AlunRouter))` 用于只贡献路由、不启服务的场景（配合 `App::into_router()` 做测试）。

## Plugins → see `references/plugins.md`

`#[async_trait] impl Plugin for MyPlugin { fn name(), async fn start(), async fn stop(), fn depends_on() }`（`start` / `stop` 均 async、按依赖拓扑排序启动 / 逆序停止）
- Built-in（`alun-plugin`）：`CachePlugin`、`SchedulerPlugin`(cron)、`NotificationPlugin`(SMTP)、`AsyncTaskPlugin`(信号量池)、`SerialPlugin`(单号)、`SidPlugin`(短 ID)
- 配置名 ↔ 构造：`[plugins] enabled = ["cache", "scheduler", "notification", "async-task", "sid", "serial"]`。**`enabled` 只是名单，`App` 不会自动读取**——需自己 `.plugin(...)` 注册，例如 `.plugin(SchedulerPlugin::new())`、`.plugin(CachePlugin::new(&cfg().app_name, &cfg().cache, &cfg().redis))`、`.plugin(SidPlugin::new())`；`alun_plugin::create_plugins_from_config(&cfg())` 只返回 `PluginManager`，供脱离 `App` 的场景使用
- 只依赖 `alun` 时插件类型从 `alun::prelude::alun_plugin::{scheduler::SchedulerPlugin, cache_plugin::CachePlugin, sid_plugin::SidPlugin, …}` 取；单号插件另有门面别名 `alun::SerialPlugin`
- `SerialPlugin`：内置 `MemorySerialBackend`（`SerialPlugin::with_memory(cfg)`）；Redis / PG 等后端由业务实现 `SerialGenerator`，经 `SerialPlugin::new(config, Arc<dyn SerialGenerator>)` 注入（`[plugins.serial] backend` 仅作记录）
- Register: `App::new()?.plugin(MyPlugin).scan().start()`

## Caching & Templates → see `references/plugins.md`

Global: `cache().set("k", &v).await?`, `cache().set_ex("k", &"v", 300).await?`, `cache().get("k").await?`, `cache().del("k") / exists("k") / incr("k", 1) / keys("user:*") / delete_pattern("tmp:*")`（`cache` feature；`Cache` trait 由 `LocalCache` / `SharedCache` 实现）
Template: `render_template("page.html", &json!({...}))` — Jinja2 syntax, files in `templates/`（`template` feature；亦可用 prelude 的 `TemplateEngine`）

**缓存命名空间**（`cache().scoped(prefix)` 返回 `CacheNamespace`，同名方法全部自动加前缀 —— 天然适合按业务域 / 租户切 key，避免全局扁平命名互相覆盖）：

```rust,ignore
use alun::prelude::*;

let ns = cache().scoped("biz:order");     // 实际 key = "biz:order:hot"
ns.set("hot", &vec![1, 2, 3]).await?;
let v: Option<Vec<i32>> = ns.get("hot").await?;
ns.del("hot").await?;

// 租户视图：无租户上下文时报错（fail-closed），绝不退化成全局 key
let tns = cache_tenant()?;                // 前缀形如 "tenant:<id>"
let sub = tns.scoped("dict");             // → "tenant:<id>:dict:xxx"
```

> ⚠️ `cache()` 本身仍是**全局扁平视图**；多租户下优先用 `cache_tenant()`——它在无租户上下文时会
> **报错而非写出裸 key**。定时任务 / MQ 消费里的缓存 key，必须先 `tenant_scope(TenantContext::Scoped(id), ..)` 再取命名空间。

## Utilities → see `references/utils.md`

200+ functions: string conversion (`to_snake()`, `to_camel()` —— `StrExt` trait 方法), date formatting (`Date::now()` / `Date::fmt()`), data masking (`Mask::mobile()`), ID generation (`Sid::uuid7()`), validation (`Valid::is_email()`), crypto (`Crypto::hash_password()`), CSV/JSON export (`Export::to_csv(&["id","name"], &rows)` / `Export::to_json(&items)`), XSS sanitization (`xss::sanitize_html(..)`, 需 `xss` feature)。

## 导出与导入：CSV / JSON / 真 XLSX → see `references/export-and-import.md`

门面转发 `alun-utils` 的导出导入能力（`Export` / `Import`），`full` 预设已含 `xlsx`：

| 能力 | API | feature | 说明 |
|---|---|---|---|
| CSV | `Export::to_csv(cols, rows)` / `to_csv_bytes(..)` | 默认 | 带 BOM，可直接当响应体；响应头用 `text/csv`、扩展名 `.csv` |
| JSON | `Export::to_json(&items)` | 默认 | 序列化整个集合 |
| **XLSX** | `Export::to_xlsx_bytes(cols, rows)` / `to_xlsx_typed(cols, items)` / `to_xlsx_workbook(&[XlsxSheet])` | `xlsx` | 真 OOXML（`rust_xlsxwriter` 写）；多 sheet、列宽自适应、冻结首行 |
| 读 XLSX | `Import::xlsx_sheet_names(bytes)` / `from_xlsx(bytes)` / `from_xlsx_sheet(bytes, sheet)` / `from_xlsx_typed::<T>(bytes)` | `xlsx` | `calamine` 读 |

```rust,ignore
use alun::prelude::*;
use std::collections::HashMap;

// 写：一个 sheet 的明细
let rows: Vec<HashMap<&str, String>> = vec![HashMap::from([("code", "A-1"), ("qty", "3")])];
let bytes = Export::to_xlsx_bytes(&["code", "qty"], &rows)?;    // 合法 ZIP（PK\x03\x04）

// 写：多 sheet 工作簿
let sheets = vec![XlsxSheet::new("明细", &["code", "qty"], &rows)];
let wb = Export::to_xlsx_workbook(&sheets)?;

// 读
let names = Import::xlsx_sheet_names(&bytes)?;
let rows = Import::from_xlsx_sheet(&bytes, &names[0])?;
```

> ⚠️ **两条硬约束**：
> 1. **不开 `xlsx` feature 时这些 API 不存在**（不是退化成 CSV 返回错内容）——宁可编译失败也不给你错的产物；
> 2. **类型原样往返**：单元格类型由「字符串内容是不是纯数字」推断。想保证 `00123` 是文本而不是
>    数字，就在 `HashMap` 里存字符串；Excel 里「看起来是数字、实际上是编号」的列必须这样存。

## Async Task Framework → see `references/task.md`

Requires `features = ["task"]`. Kafka-driven distributed tasks with retry/DLQ:
- `#[alun::task_handler(task_type = 1, topic = "...", timeout_seconds = 60, max_retries = 3)]`（编译期收集到 `TASK_HANDLERS`；`task_type` 必填，`topic` 默认 `task_{task_type}`）
- Implement `TaskHandler`（`fn task_type(&self) -> i16` + `async fn execute(&self, payload: JsonValue /* = serde_json::Value */) -> std::result::Result<JsonValue, String>`）与 `TaskStorage`（`save_task_log` / `save_task_queue` / `update_task_status` / `get_retry_count` / `update_retry`，由业务决定表名与字段）
- Submit via `TaskProducer::new(brokers, Arc<dyn TaskStorage>, HandlerRegistry)?` → `.submit(SubmitTaskParams { task_type, payload, priority, user_id, resource_id, resource_type })`
- 生命周期：`PENDING → PROCESSING → COMPLETED / FAILED → (重试) → DEAD_LETTER`

## 实时推送（SSE）→ see `references/realtime-sse.md`

Requires `features = ["sse"]`。只需「服务端 → 客户端」单向推送时（站内信、待办红点、
导入导出进度、监控大屏），用 SSE 而不是 WebSocket：走普通 HTTP，浏览器自动带 Cookie /
可写 `Authorization` 头，因此**天然复用 `AuthLayer`**；断线重连由浏览器 `EventSource` 负责。

```rust,no_run
use std::sync::OnceLock;
use alun::prelude::*;

static NOTIFY: OnceLock<alun::sse::Broadcaster<String>> = OnceLock::new();
fn notify() -> &'static alun::sse::Broadcaster<String> {
    NOTIFY.get_or_init(|| alun::sse::Broadcaster::new(1024))
}

// 推送端：任何 handler / 异步 Hook / 定时任务里
notify().send(r#"{"type":"todo","id":7}"#.to_string());

// 订阅端
#[alun::get("/events")]
async fn events() -> impl axum::response::IntoResponse {
    alun::sse::sse_json(notify().subscribe(), "message")
}
```

- `Broadcaster::new(capacity)` / `.subscribe()` / `.send(msg) -> usize`（扇出个数）/ `.receiver_count()`；
  内部是 `tokio::sync::broadcast`，无锁有界——订阅端消费不及只会丢弃最旧消息，**不断开长连接**
  （对提醒类消息是正确取舍）。
- `sse_json(rx, event_name)` 已内置 15s 保活注释行，防止反向代理按空闲超时切断。
- 广播器是**进程级单例**，一般用 `OnceLock` + 启动钩子初始化；多实例部署时改用 Redis Pub/Sub
  或 `alun-kafka` 做跨进程扇出。

## Observability (F10) → 可观测性 → see `references/observability.md`

Requires `features = ["metrics"]`（埋点）/ `["metrics-prometheus"]`（+ `/metrics` 端点，门面 `full` 已含）。

```toml
# config/config.toml
[metrics]
enabled   = false          # 默认关闭：端点默认不存在
path      = "/metrics"     # 绝对路径，不参与 [router].prefix
allow_ips = []             # 空 = 仅本机回环；Prometheus 在外网抓取时必须显式列出其 IP
http      = true           # HTTP 自动埋点（关掉则只留 DB / 缓存 / 安全指标）
```

自动产出（无需业务代码）：

| 指标 | 标签 | 说明 |
|---|---|---|
| `alun_http_requests_total` / `alun_http_request_duration_seconds` / `alun_http_requests_in_flight` | `method` `route` `status` | `route` 为**归一化路径**（`/users/1` → `/users/:id`），`status` 为 `2xx` 类别——**刻意避免标签基数爆炸** |
| `alun_db_queries_total` / `alun_db_query_duration_seconds` | `op` `table` `status` | ORM 读路径按模型表名归类；原生 SQL 通道 `table="raw"` |
| `alun_tenant_scope_missing_total` / `alun_raw_sql_tenant_unbound_total` | `table` | ⚠️ **安全指标**：多租户 fail-closed 被触发的次数（持续增长 = 某处漏配 `tenant_scope` 或该标 `cross_tenant`） |
| `alun_cache_operations_total` / `alun_cache_operation_duration_seconds` | `op` `result` | `result` ∈ `hit` / `miss` / `ok` / `error` |

手动埋点：`use alun::metrics;` 后直接用 `metrics` 门面宏（`counter!` / `histogram!` / `gauge!`），
导出端点会一并带上；也可自行 `metrics::set_global_recorder(..)` 接 OpenTelemetry 等后端。
框架指标均带 **`# HELP` 说明行**（`describe_*` 在 `install_prometheus()` 内注册——
`describe_*` 在无 recorder 时会被**静默丢弃**，故不可提前调用）。

> 开启门面 `metrics` 会**自动下传**到已启用的 `alun-db` / `alun-cache`
> （弱 feature `alun-db?/metrics`），不会出现"HTTP 有指标、DB 没指标"的半套可观测性。

> 端点挂在**中间件链之外**（不穿鉴权/限流，抓取器无需 Token），因此访问控制由
> `[metrics].allow_ips` 独立完成；配置开了却没编 `metrics-prometheus` 会启动期 `warn`。

接入 Prometheus 抓取：

```yaml
# prometheus.yml
scrape_configs:
  - job_name: alun
    scrape_interval: 15s
    static_configs:
      - targets: ["127.0.0.1:8023"]      # 若 allow_ips 未放行该 IP，抓取会 403
```

手动埋点（可换其它后端，如 OpenTelemetry）：`use alun::metrics;` 后直接用 `metrics` 门面宏
`counter!` / `histogram!` / `gauge!`，导出端点会自动带上。

> ⚠️ 三个坑：
> 1. **全局 recorder 只能装一次** —— 并行测试里谁先装谁生效，务必统一走 `App` 的装配路径
>    （或 `install_prometheus()`），不要在多处 `set_global_recorder`；
> 2. **`describe_*` 在没有 recorder 时会被静默丢弃** —— 所以「注册 describe」必须排在
>    `install_prometheus()` 之后，否则产物里**没有 `# HELP` 说明行**；
> 3. **`/metrics` 自身要排除在 HTTP 计数之外**（`exclude_paths` 已默认含它），否则自抓取会污染 QPS。

## Kafka & File Storage → see `references/task.md`

- Kafka（`features = ["kafka"]`）：`alun::alun_kafka::{KafkaProducer, KafkaConsumer, KafkaError, KafkaPlugin}`（rdkafka）
- File storage（`features = ["fs"]`）：`alun::alun_fs::{StorageBackend, LocalFs, FsPlugin}`，后端注册用 `alun::alun_fs::registry::BackendRegistry` + `alun::alun_fs::types::BackendConfig`（本地 / MinIO）
  - **正确构造**：`FsPlugin::new(FsPluginConfig { .. }, registry)`；**没有** `FsPlugin::with_registry()`。便捷构造 `FsPlugin::new_local(root_dir)`（仅本地）。
  - `BackendRegistry::register(backend_实例, BackendConfig)`——**第一参是实例**（非 `Arc`），第二参才是 config；同名 `backend_type` 会覆盖，多桶共存需用不同 `backend_type` 标识。
  - `BackendConfig` 字段：`backend_type / endpoint / region / access_key / secret_key / use_tls / root_path / is_default`；`FsPluginConfig` 字段：`default_backend_type / local_root_dir / max_file_size_bytes / presign_url_ttl_secs`。
  - ⚠️ **MinIO 需 `alun-fs/minio` 子 feature**，而门面 `alun` 的 `fs = ["dep:alun-fs"]` **未开启它**——要 MinIO 需给门面补独立 feature（别把它并进 `fs`，会波及所有现有项目）。

## Recommended Project Structure

```text
src/
├── main.rs          # App::new()?.parse_cli().scan().start()
├── controllers/     # #[alun::controller] impl 块 / #[alun::get] 处理器
├── models/          # #[derive(Model)] 结构体与 DTO（*Model, *Req, *Res）
├── services/        # Business logic
└── plugins/         # Custom Plugin impls
config/config.toml
migrations/          # V{n}__Name.sql（或成对 .up.sql/.down.sql）
templates/           # .html Jinja2 files
uploads/ downloads/
```

## Quick Reference

| Category | Key types/macros |
|----------|-----------------|
| App | `App::new()`, `App::from_config()/from_config_dir(dir)/with_config(cfg)/with_config_manager(cm)`, `.parse_cli()`, `.get()/.post()/.put()/.patch()/.delete()`, `.route(method, path, h)`, `.group()`, `.merge(prefix, sub)`, **`.module(f)` / `.routes(f)` / `.permission_defs(v)` / `.no_auth_defs(v)` / `.into_router()`**, `.scan()`（宏注解自动收集，默认主推）, `.plugin()`, `.with_permission()/.with_role()`, `.on_startup()`（可多个）, `.layer(L)` / `.layer_from_fn(f)` / `.route_layer(L)`（可多个）, `.start()`, `.serve(addr)` |
| Routing | **`#[alun::routes]`（模块级，推荐——不依赖链接器）**, `#[get/post/put/patch/delete]`, `#[controller]`, `AlunRouter::add_get/add_post/add_put/add_patch/add_delete/add_route`（+ `add_*_with_layer`）, `AlunRouter::merge` / `merge_with_layer(prefix, sub, wrap)`, `AlunRouter::into_axum`, `#[alun::layer(L)]`（路由级 layer，写在路由注解上方） |
| Features | 默认 `core`；预设 **`mini` / `lite` / `full`**；驱动 `sqlite`/`postgres`/`mysql`（**`db` 不含驱动**）；`cache`/`template`/`plugin`/`fs`；后端 `fs-local`/`fs-minio`；重型 `kafka`/`task`（模型）/`task-kafka`（含 Kafka）；可观测性 **`metrics`（埋点，no-op 安全）/`metrics-prometheus`（导出端点）**；`sse`；`xlsx`（真 XLSX，转发 `alun-utils/xlsx`） |
| Response | `Res::ok()`, `Res::page(list, total, page, page_size)`, `ResResult<T>`, `PageData<T>`, `ApiError::bad_request()` etc., `codes::*` |
| Extractors | `Path`, `Query`, `Extension`, `ValidatedJson<T>`, `ValidateExt::validate_or_reject()`, `validate_{uuid,mobile,password_strength,id_card,date,datetime,date_or_datetime,email,url}` |
| DB | `db()`, `Row`, `db().insert_row/find_row_by_pk/update_row/delete_row`, `db().find/find_first/find_with_params/scalar/execute/execute_with_params/paginate`, `db().query()/page()/count()`, `sql!`, `SqlQueryExt` 终态, `Model`, `TypeMapper`（枚举字段）, `CrudExt`, `PersistExt`, `ModelQuery`, `ChangeSet`, `RowBatchExt`, `Transaction`, `migrate`, `Migration`, `PageQuery`（core 唯一）, `Page`, `PageResponseExt` |
| Auth | `JWT`, `AuthClaims`（**提取器**，双通道：请求扩展 → 回退验签 + 黑名单；`#[no_auth]` 路由上同样能拿到）, `Principal`（身份视图 `user_id`: `UserId` 即**原样 `sub`**（非 `Uuid`，强类型用 `p.user_id.parse::<T>()`）/`username`/`roles`/`claims`，prelude 已含）, `Option<AuthClaims>`（可选身份：无 token 得 `None`、不报错）, `TokenClaims`, `UserId`, `#[permission]`, `#[no_auth]`（`AuthLayer` / `RequireRoleLayer` / `RequirePermissionLayer` 在 `alun::web`，prelude 不含） |
| Multi-Tenancy | `[tenant]` config（`enabled` / `write_enabled` / `claim` / `column` / `ignore_paths` / `schema_per_tenant` / `schema_prefix`）, `TenantContext::{Scoped, Cross}`, `tenant_scope(ctx, fut)`, `current_tenant()`, `scope_for::<T>()`, `ensure_raw_allowed::<T>()`, `TenantLayer { claim, ignore_paths }`, 模型 `#[tenant]` 字段 + `#[model(cross_tenant)]` |
| Config & Paths | `cfg()`, `config()`, `ALUN_*` env vars, `alun::upload_path()`, `alun::download_path()` |
| Cache | `cache()`, `Cache` trait, `LocalCache`, `CacheStats`（`SharedCache` 是 `cache()` 的返回类型，门面未再导出，需直接依赖 `alun-cache` 才能点名） |
| Plugins | `Plugin` trait, `#[plugin]`, `SchedulerPlugin`, `CachePlugin`, `NotificationPlugin`, `AsyncTaskPlugin`, `SidPlugin` |
| Utils | prelude: `Mask`, `Sid`, `Valid`, `Crypto`；`StrExt` / `Date` / `Export` 等需直接依赖 `alun-utils` |
| Task | `#[task_handler]`, `TaskHandler`, `TaskStorage`, `TaskProducer`, `TaskPlugin` |
| Extensions | `App::on_startup` / `.layer(L)` / `.layer_from_fn(f)` / `.route_layer(L)` / `.web_plugin(P)` / `.module(f)` / `.routes(f)` / `.into_router()`；`AlunRouter::merge_with_layer(prefix, sub, wrap)`（子树层，在 `.routes(f)` 里用）；`#[alun::layer(L)]` 路由级 layer；`WebPlugin`（`routes` / `permission_rules` / `no_auth_paths`）；`#[hook_async(H)]` 异步行级 Hook（提交后触发） |
| Cache NS | `cache().scoped(prefix)` → `CacheNamespace`（`get` / `set` / `set_ex` / `del` / `exists` / `keys` / `delete_pattern`），`cache_tenant()?` 租户视图 |
| Multi-DB | `db()` 主库；`set_db_named(name, db)` + `db_named(name)` 命名数据源（事务必须走主库、只读副本不自 routing） |
| Realtime | `alun::sse::{Broadcaster, sse_json}`（`sse` feature）：`Broadcaster::new(cap)` / `.send(msg)` / `.subscribe()`；`sse_json(rx, "message")` |
| Export / Import | `Export::{to_csv, to_csv_bytes, to_json, to_xlsx_bytes, to_xlsx_typed, to_xlsx_workbook}`、`XlsxSheet`、`Import::{from_csv, from_json, xlsx_sheet_names, from_xlsx, from_xlsx_sheet, from_xlsx_typed}`（`xlsx` feature） |
| Observability | `metrics` / `metrics-prometheus`；自动指标 `alun_http_*` / `alun_db_*` / `alun_cache_*` / `alun_tenant_scope_missing_total`；`[metrics].{enabled,path,allow_ips,http}`；手动 `counter!` / `histogram!` / `gauge!` |
| Sorting | `SortQuery`（请求侧 `?sort_by=&sort_desc=`）+ `ModelQuery::order_by_whitelisted(field, desc, &[&'static str])` —— 白名单外直接报错 |
| Serial | `SerialPlugin`, `SerialRule`, `SerialGenerator`, `MemorySerialBackend` |

Error codes: `OK=0`, `BAD_REQUEST=400`, `UNAUTHORIZED=401`, `FORBIDDEN=403`, `NOT_FOUND=404`, `METHOD_NOT_ALLOWED=405`, `CONFLICT=409`, `UNPROCESSABLE_ENTITY=422`, `TOO_MANY_REQUESTS=429`, `INTERNAL=500`, `SERVICE_UNAVAILABLE=503`

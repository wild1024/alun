---
id: reference.dependencies
title: 依赖与版本基线（Dependencies）
summary: 编译基线（edition 2024 / MSRV 1.94.1）、关键第三方依赖与版本约束。
tags: [reference, dependencies, msrv]
status: active
owner: alun-core-team
source_of_truth: code
depends_on: []
last_reviewed: 2026-10-06
---

# 依赖关系

> 版本基线：`edition = "2024"`、`resolver = "3"`、`rust-version = "1.94.1"`（根 `Cargo.toml`）。
> 工作区共 21 个成员 crate + 5 个示例工程。

## 1. 内部 Crate 依赖关系图

依赖是**严格分层**的：第 0 层零内部依赖，上层只向下依赖，不存在环。

```text
第 0 层 ─ 零内部依赖（叶子）
┌───────────┐ ┌─────────────┐ ┌──────────────┐ ┌──────────────┐ ┌──────────────┐ ┌─────────────┐
│ alun-core │ │ alun-config │ │ alun-db-core │ │ alun-db-macro│ │ alun-db-build│ │ alun-macros │
└───────────┘ └─────────────┘ └──────────────┘ └──────────────┘ └──────────────┘ └─────────────┘
     ▲              ▲                ▲                 ▲               ▲                ▲
     │              │                │                 │               │                │
第 1 层 ─ 基础设施（仅依赖第 0 层）
┌──────────┐ ┌──────────┐ ┌────────────┐ ┌──────────────┐ ┌──────────┐ ┌──────────┐ ┌────────┐
│ alun-log │ │alun-utils│ │alun-db-sql │ │alun-db-driver│ │alun-     │ │alun-     │ │alun-   │
│←config   │ │←core     │ │←db-core    │ │-sqlite/      │ │template  │ │kafka     │ │task    │
└──────────┘ └──────────┘ └────────────┘ │ postgres/    │ │←core     │ │←core+cfg │ │←core   │
┌──────────────┐                         │ mysql        │ └──────────┘ └──────────┘ └────────┘
│ alun-cache   │                         │←db-core+sql  │ ┌──────────┐
│←core+config  │                         └──────────────┘ │ alun-fs  │
└──────────────┘                                          │←core+cfg │
                                                          └──────────┘
第 2 层 ─ 组合层
┌───────────────────────────────────────┐   ┌──────────────────────────────────────────┐
│ alun-db                               │   │ alun-web                                 │
│ ← db-core / db-sql / db-macro         │   │ ← core / macros / config / log / utils   │
│ ← [可选] 三个 driver                  │   │ ← [可选] alun-db / alun-cache / template │
└───────────────────────────────────────┘   └──────────────────────────────────────────┘
┌───────────────────────────────────────┐
│ alun-plugin ← core + config + cache + utils │
└───────────────────────────────────────┘

第 3 层 ─ 门面
┌──────────────────────────────────────────────────────────────────────────────┐
│ alun（prelude + re-export + proc macro 再导出；其余 crate 按 feature 可选出） │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 依赖详解

| Crate | 直接依赖（内部） | 说明 |
|-------|-----------------|------|
| **alun-core** | 无 | 最底层（纯骨架）：`Error`/`Result`、`Plugin`/`PluginManager`。**零内部依赖、零 feature**，只依赖 serde/tokio/thiserror/async-trait/anyhow/parking_lot/serde_json。响应形状类型（`Res`/`ApiError`/`PageData`/`PageQuery`）已下沉 `alun-web` |
| **alun-config** | 无 | TOML 加载、多环境 profile、动态配置；只依赖 serde/serde_json/toml/tracing/parking_lot/dashmap |
| **alun-macros** | 无 | Proc-macro crate（`syn 3`/`quote`/`proc-macro2`）。生成的注册代码以 `::alun::…` 绝对路径引用门面，因此**编译期不需要**声明 `alun-web` |
| **alun-db-core** | 无 | 数据库核心类型与 trait：`Row`、`Model`、`Hook`、`Dialect`、`Driver`、`Expr`/`Statement`、`Page`、`Value`/`Type`，以及 `hook` 模块（`TimestampHook`、`now_value` 与 Hook 洋葱执行器；本次重构从 `alun-db` 迁入）。**零异步、零驱动、零生态**（默认只依赖 `thiserror`；chrono/uuid/ulid/… 全是可选 feature） |
| **alun-db-macro** | 无 | `#[derive(Model)]`/`#[derive(Row)]`/`#[derive(TypeMapper)]` 与 `sql!`；只依赖 syn/quote/proc-macro2 |
| **alun-db-build** | 无 | `build.rs` 助手（`build_snapshot()` 扫描 SQL 迁移生成 schema 快照），零依赖 |
| **alun-db-sql** | `alun-db-core` | SQL 渲染器与 SQLite/PostgreSQL/MySQL 方言实现 |
| **alun-db-driver-sqlite** | `alun-db-core`, `alun-db-sql` | rusqlite 驱动 |
| **alun-db-driver-postgres** | `alun-db-core`, `alun-db-sql` | tokio-postgres 驱动 |
| **alun-db-driver-mysql** | `alun-db-core`, `alun-db-sql` | mysql_async 驱动 |
| **alun-db** | `alun-db-core`, `alun-db-sql`, `alun-db-macro`；驱动可选 | `Db`/`Transaction`/rowops/`CrudExt`/`PersistExt`/`ModelQuery`/`ChangeSet`/builder/迁移。`sqlite`（默认）、`postgres`、`mysql` 各自门控一个驱动 |
| **alun-log** | `alun-config` | 按 `LogConfig` 初始化 tracing（控制台 + 文件滚动） |
| **alun-utils** | `alun-core` | 工具集（crypto/mask/sid/valid/date/str/serial/export/ua）。单号生成模块需 `tokio`（sync+macros）与 `async-trait` |
| **alun-cache** | `alun-core`, `alun-config` | `Cache` trait + `LocalCache`/`RedisCache`/`SharedCache` |
| **alun-template** | `alun-core` | `TemplateEngine`（minijinja，`askama` 为可选特性） |
| **alun-web** | `alun-core`, `alun-macros`, `alun-config`, `alun-log`, `alun-utils`（features `validator-integration`）；`alun-db`/`alun-cache`/`alun-template` **可选** | Web 层：`App`/`AlunRouter`/中间件/extract/resources/JWT，以及响应形状类型 `Res`/`ApiError`/`PageQuery`（`src/api.rs`，含 `From<DbError>` 等跨层错误映射）。数据库、缓存、模板均**经 feature 可选**（`db`/`cache`/`template`），默认不引入 |
| **alun-plugin** | `alun-core`, `alun-config`, `alun-cache`, `alun-utils` | 内置插件：通知（SMTP）、缓存、异步任务、调度器、单号（`serial_plugin`） |
| **alun-kafka** | `alun-core`, `alun-config` | `KafkaProducer`/`KafkaConsumer`/`KafkaPlugin` |
| **alun-task** | `alun-core` | 异步任务框架，零 SQL 依赖，持久化通过 `TaskStorage` trait 委托 |
| **alun-fs** | `alun-core`, `alun-config` | 多后端文件存储（`local` 默认 / `minio` 特性引入 S3 SDK） |
| **alun** | `alun-core`, `alun-utils`, `alun-macros`, `alun-web`（features `config`）, `alun-config`；其余全部可选 | 门面层统一导出 + proc macro 再导出。默认 `core`（**不含任何数据库驱动**，能力靠 feature 叠加）。**feature 转发**：`db`/`sqlite`/`postgres`/`mysql` 与九个生态类型逐个透传到 `alun-db`，`xss` 透传到 `alun-utils`；`alun-web` 提供同名转发 |

### alun-web 的可选依赖与门控点

`alun-web` 只对 `alun-core`/`alun-macros`/`alun-config`/`alun-log`/`alun-utils` 是**硬依赖**，
其余三个基础设施全部通过 feature 控制：

- `alun-web/db` → 引入 `alun-db`（**只带 `async`，不含任何驱动**），启用 `resources::{db, try_db, set_db}`，`App::init_resources()` 中条件初始化；`sqlite` / `postgres` / `mysql` / 生态类型 / `xss` 为同名转发 feature
- `alun-web/cache` → 引入 `alun-cache`，启用 `resources::{cache, try_cache, set_cache}` 与 JWT 黑名单等能力
- `alun-web/template` → 引入 `alun-template`，启用 `render_template()` / `try_template()` / `set_template()`
- `alun-utils` 是硬依赖，且恒定带 `validator-integration`：`ValidatedJson::validate()` / `ValidateExt` 需要它

> `alun-db-sql`、`alun-db-macro`、`alun-db-build`、三个驱动都**不在** `alun-web` 的依赖里：
> 它们只经 `alun-db` 传递——打开 `db` 才进入构建图，而三个驱动各自还要 `sqlite`（`db` 自带）/
> `postgres` / `mysql`，生态类型再各自一个 feature。

### crate 版本与升级依据

版本号按「已发布版本 → 当前源码」的**公共 API 差异**定级（`cargo` 的 caret 语义下 `0.1.x` 是兼容区间，
所以破坏性变更必须升 minor）：

| crate | 版本 | 本轮级别 | 依据 |
|---|---|---|---|
| `alun` | **0.2.0** | 破坏性 | prelude 组成变化（`db` 下整块转发 `alun-db` prelude；`alun::prelude::Result` 遮蔽 db 的 `Result`、两个 prelude 同时 glob 会歧义）+ 默认 `full` 下依托 `alun-db` 0.2.0 |
| `alun-db` | **0.2.0** | 破坏性 | 相对 0.1.4 重写：公共 API 移除 80 项 / 新增 145 项（`batch_*`、`create_db`、`find_by_pk` 等已改名或移除） |
| `alun-web` | 0.1.7 | 兼容 | 新增 `add_patch` / `add_patch_with_layer` / `build_config` 与 `#[alun::patch]` 路由 |
| `alun-core` | 0.1.2 | 兼容 | 新增 `DEFAULT_PAGE_SIZE` / `MAX_PAGE_SIZE` / `PageQuery::normalized` / `calc_pages` |
| `alun-macros` | 0.1.1 | 兼容 | 新增 `#[patch]`；注册项生成改为 `::alun::inventory::submit! { … }`（应用无需声明 `inventory`） |
| `alun-config` | 0.1.3 | 兼容 | 新增 `NonceConfig` / `IdempotencyConfig` 配置段 |
| `alun-plugin` | 0.1.2 | 兼容 | 新增 cron 调度 API（`register_scheduled` / `attach` / `trigger` / `list` / `is_running` …） |
| `alun-utils` | 0.1.4 | 修复 | 实现修复，公共 API 无变化 |
| `alun-cache` | 0.1.2 | 修复 | 实现修复，公共 API 无变化 |
| `alun-kafka` | 0.1.2 | 修复 | 实现修复，公共 API 无变化 |
| `alun-task` | 0.1.2 | 修复 | 实现修复，公共 API 无变化 |
| `alun-log` | 0.1.1 | 无变化 | 与已发布版本源码一致，本轮不发布 |
| `alun-template` | 0.1.0 | 无变化 | 同上 |
| `alun-fs` | 0.1.2 | 无变化 | 同上 |
| `alun-db-core` / `-sql` / `-macro` / `-build` / `alun-db-driver-{sqlite,postgres,mysql}` | 0.1.0 | 新 crate | 尚未发布，首版即 0.1.0 |

> 内部依赖一律用 `{ workspace = true }`，因此版本只需改「各 crate 的 `[package] version`」与根
> `[workspace.dependencies]` 两处；`alun-macros` 原先继承工作区版本，本轮起改为显式 `0.1.1`。
> 校验：每个升级后的版本都严格大于该 crate 在 crates.io 上的已发布最大值（`alun-db` 已发布 0.1.4，
> 故新实现必须 ≥ 0.1.5，取 0.2.0 以标示破坏性重写）。

---

## 2. 外部核心依赖

> 版本号均取自根 `Cargo.toml` 的 `[workspace.dependencies]` 或成员 crate 的 `Cargo.toml`。

### 2.1 HTTP / Web 框架

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **axum** | 0.8 | HTTP Web 框架 | `alun-web`（App、Router、Middleware、Extract） |
| **tower** | 0.5 | 中间件 Layer/Service trait 体系 | 所有中间件模块 |
| **tower-http** | 0.7 | CORS、Trace、Compression、Limit、静态文件服务 | `alun-web`（features `cors`/`trace`/`compression-gzip`/`limit`/`fs`） |
| **http** | 1.5 | HTTP 类型（HeaderValue、Method、StatusCode） | 多个中间件、`alun-utils::web`、`alun-fs`（minio 特性） |
| **bytes** | 1.12 | 字节缓冲区 | 中间件、`alun-db-driver-postgres`、`alun-task`、`alun-fs` |
| **hyper** | 间接 | 底层 HTTP 实现 | 由 axum/tower-http 传递引入，工作区**无直接声明**，文档中不应作为直接依赖使用 |

### 2.2 数据库

`sqlx` 已从项目中移除（本仓库不再使用）；数据库访问改为自研分层实现：

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **rusqlite** | 0.40.2 | SQLite 驱动（features `bundled`/`chrono`/`serde_json`/`uuid`） | `alun-db-driver-sqlite` |
| **tokio-postgres** | 0.7.18 | PostgreSQL 异步驱动 | `alun-db-driver-postgres` |
| **mysql_async** | 0.37.1 | MySQL 异步驱动（`default-features = false`, `minimal`） | `alun-db-driver-mysql` |
| **redis** | 1.7 | Redis 异步客户端（features `tokio-comp`/`connection-manager`） | `alun-cache`（`RedisCache`） |
| **time** / **jiff** / **rust_decimal** / **bigdecimal** / **ulid** / **ipnetwork** | 0.3.55 / 0.2.37 / 1.43 / 0.4.10 / 3.0 / 0.21.1 | `alun-db-core` 可选生态类型（逐一同名 feature 门控，经 `alun-db` 透传） | `alun-db-core`、`alun-db` |

### 2.3 序列化与数据处理

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **serde** | 1.0.229 | 序列化框架（features `derive`） | 全项目（所有 struct 的 Serialize/Deserialize） |
| **serde_json** | 1.0.151 | JSON 处理 | `alun-core`(Res/ApiError)、`alun-config`、`alun-cache`、`alun-db`(json feature) |
| **regex** | 1.13 | 字符串清洗/文件名清理 | `alun-utils::str`、示例工程 |
| **csv** | 1.4 | CSV 导出 | `alun-utils::export` |
| **url** | 2.5 | URL 解析 | `alun-utils::web` |

### 2.4 认证与安全

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **jsonwebtoken** | 11.1（`features = ["rust_crypto"]`） | JWT 编解码 | `alun-web`（`JWT`、`AuthLayer`、`TokenClaims`） |
| **bcrypt** | 0.19 | Bcrypt 密码哈希 | `alun-utils::crypto`（`verify_password` 自动识别 `$2` 前缀） |
| **argon2** | 0.6 | Argon2 密码哈希（默认算法） | `alun-utils::crypto`（`hash_password`/`verify_password`） |
| **aes-gcm** | 0.11 | AES-256-GCM 加解密 | `alun-utils::crypto`（`aes_encrypt`/`aes_decrypt`） |
| **sha2** | 0.11 | SHA-256 哈希 | `alun-utils::crypto`、`alun-web`（请求指纹/幂等） |
| **hmac** | 0.13 | HMAC-SHA256 | `alun-utils::crypto`、`alun-web` |
| **base64** | 0.23 | Base64 编解码 | `alun-utils::crypto`、`alun-web` |
| **rand** | 0.10 | 随机数/随机密钥 | `alun-utils::crypto`（注意 v0.10 API：`rand::rng()` / `random_range` / `RngExt`） |
| **hex** | 0.4.3 | 十六进制编解码 | `alun-utils::crypto`、`alun-web` |
| **uuid** | 1.26（`v4`/`v7`） | UUID 生成 | `alun-utils::sid`、`alun-web`（RequestId/TokenClaims）、`alun-db-core`（uuid feature） |

> **`jsonwebtoken 11` 必须显式选择 crypto provider**：要么 `rust_crypto`，要么 `aws_lc_rs`，二者之一，
> 否则编译不通过（v11 移除了默认 provider）。本项目工作区已统一选择 `features = ["rust_crypto"]`。

### 2.5 模板

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **minijinja** | 2.24 | Jinja2 模板引擎（默认实现） | `alun-template` |
| **askama** | 0.16（可选） | 编译期模板 | `alun-template`（可选依赖） |

### 2.6 日志

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **tracing** | 0.1.44 | 结构化日志框架 | 全项目 |
| **tracing-subscriber** | 0.3.23 | 日志订阅器（features `env-filter`/`json`/`fmt`） | `alun-log`、`alun-web` |
| **tracing-appender** | 0.2.5 | 日志文件滚动 | `alun-log`、`alun-web` |

### 2.7 异步运行时

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **tokio** | 1.53 | 异步运行时（工作区 `features = ["full"]`；各 crate 按需收窄） | 全项目 |

### 2.8 其他

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **thiserror** | 2.0.20 | 派生 `Error` trait | `alun-core`、`alun-db-core`（`DbError`）、`alun-db`、`alun-kafka`、`alun-task`、`alun-fs` |
| **anyhow** | 1.0.104 | 灵活错误处理 | `alun-core`、`alun-web`、示例项目 |
| **async-trait** | 0.1.92 | async trait 支持 | `alun-core`(Plugin)、`alun-cache`(Cache)、`alun-web`、`alun-utils`(serial)、`alun-task`、`alun-fs` |
| **inventory** | 0.3.24 | 编译期注册收集（不依赖链接器自定义段） | `alun-web`（路由/权限/免鉴权）、`alun-task`（TASK_HANDLERS）、`alun-fs`（STORAGE_BACKENDS）、`alun-macros` |
| **inventory** | 0.3.24 | 编译期类型注册（替代旧方案 `linkme`，不依赖链接器自定义段） | 路由 / 权限 / 免鉴权的 `scan()` 通道、任务处理器、`alun-fs` 后端自动发现；显式注册通道（`#[alun::routes]`）不依赖它 |
| **parking_lot** | 0.12.5 | 高性能同步原语 | 全项目（RwLock、Mutex） |
| **dashmap** | 6.2 | 并发 HashMap | 在 `alun-config`/`alun-task` 中声明 |
| **chrono** | 0.4.45 | 日期时间（features `serde`） | `alun-utils::date`、`alun-core`/`alun-db`(chrono feature)、`alun-plugin`、`alun-task`、`alun-fs` |
| **validator** | 0.21 | 结构体字段校验（features `derive`） | `alun-web::extract`(`ValidatedJson`)、`alun-utils` 的 `validator-integration` 特性、`alun` 的 dev-dependency（仅 doctest） |
| **ammonia** | 4.2（可选） | HTML/XSS 净化 | `alun-utils::xss`（feature `xss`） |
| **rdkafka** | 0.39 | Kafka 客户端 | `alun-kafka`、`alun-task` |
| **toml** | 1.1 | TOML 解析/序列化 | `alun-config`、`alun-web` |
| **lettre** | 0.11.23 | SMTP 邮件发送（features `builder`/`rustls-tls`/`smtp-transport`） | `alun-plugin::notification` |
| **aws-sdk-s3** / **aws-config** | 1.149 / 1.12（可选） | S3/MinIO 对象存储 | `alun-fs`（feature `minio`） |
| **syn** / **quote** / **proc-macro2** | 3.0.6 / 1.0.47 / 1.0.107 | 过程宏解析/生成 | `alun-macros`、`alun-db-macro` |

---

## 3. Feature 开关传递链

```text
alun（default = ["full"]）
  ├── "full" = ["db", "postgres", "mysql", "template", "cache", "plugin", "kafka", "task", "fs"]
  │
  ├── "config" = ["alun-web/config"]            （空 feature，仅作开关；alun-web 依赖处已恒定开启）
  ├── "db" → ["config", "dep:alun-db", "alun-web/db", "alun-db/async"]
  │     └── alun-web/db = ["dep:alun-db", "config", "alun-db/async"]
  │           └── 启用 resources::{db, try_db, set_db}；**db 不含任何驱动**
  ├── "sqlite" = ["db", "alun-db/sqlite"]       （驱动必须显式选）
  ├── "postgres" / "mysql" = ["db", "alun-db/postgres" | "alun-db/mysql"]      ← 驱动按需
  ├── "chrono" / "time" / "jiff" / "rust_decimal" / "bigdecimal" / "uuid" / "ulid" / "json" / "ipnetwork"
  │     = ["db", "alun-db/同名"]                                              ← 模型生态类型按需
  ├── "xss" = ["alun-utils/xss"]                ← HTML 净化；prelude 里 `xss::sanitize_html(..)`
  ├── "template" → ["dep:alun-template", "alun-web/template"]
  │     └── alun-web/template → render_template() / try_template() / set_template()
  ├── "cache" → ["dep:alun-cache", "alun-web/cache"]
  │     └── alun-web/cache → cache() / try_cache() / set_cache()
  ├── "plugin" → ["dep:alun-plugin"]
  ├── "kafka" → ["dep:alun-kafka"]
  ├── "task" → ["dep:alun-task"]
  └── "fs" → ["dep:alun-fs"]

alun-web（无 default feature）
  ├── "config" = []                              （空 feature）
  ├── "db" = ["dep:alun-db", "config", "alun-db/sqlite", "alun-db/async"]
  ├── "sqlite" / "postgres" / "mysql" / 九个生态类型 / "xss"
  │     └── 与 alun 同名的转发 feature（直接依赖 alun-web 的项目也只需在它上面选）
  ├── "cache" = ["dep:alun-cache"]
  └── "template" = ["dep:alun-template"]

alun-core（default = []，且**无任何 feature** —— 骨架层不认识 HTTP / 数据库）

alun-db（default = ["sqlite", "async"]）
  ├── "sqlite" / "postgres" / "mysql" → 对应 alun-db-driver-*
  ├── "async" = ["dep:tokio"] ；"sync" = []
  └── "chrono"/"time"/"jiff"/"rust_decimal"/"bigdecimal"/"uuid"/"ulid"/"json"/"ipnetwork"
        → 同时打开 alun-db-core 的同名 feature（生态类型）

alun-db-core（default = []）
  └── chrono / time / jiff / rust_decimal / bigdecimal / uuid / ulid / json / ipnetwork

alun-utils（default = []）
  ├── "validator-integration" = ["validator"]   // alun-web 恒定开启
  └── "xss" = ["ammonia"]                       // 经 alun / alun-web 的 "xss" feature 转发；xss 模块在 prelude 中暴露

alun-db-sql（default = ["sqlite", "postgres", "mysql"]）；alun-db-driver-sqlite（default = ["async"]）
alun-fs（default = ["local"]）
  └── "minio" = ["dep:aws-sdk-s3", "dep:aws-config", "dep:http"]
```

> `xss` 现在**是 `alun` 的 feature**（转发到 `alun-utils/xss`），底层实现仍在 `alun-utils`；
> 历史文档曾写"alun 门面上没有 xss feature"，已作废。
> 另：`alun-core` 的 `web` feature 已随响应类型下沉 `alun-web` 一并删除——
> 现在的 `alun-core` 没有任何 feature。

---

## 4. 编译期依赖最小化

对于一个仅需 Web 服务的项目：

```toml
[dependencies]
alun = { version = "0.3" }   # 默认 = core，已是最小 Web 构建
```

> 不需要再写 `default-features = false`：0.3 起默认就是最小可用的 `core`，
> 而 `core` → `config` → `alun-web/config` 这条链是空标记（全仓库无 `#[cfg(feature = "config")]`），
> 关与不关的依赖树完全一致。

编译图仅包含：`alun` + `alun-web`（恒定依赖 `alun-core`、`alun-macros`、`alun-config`、
`alun-log`、`alun-utils`〔features `validator-integration`〕），即 **7 个内部 crate**，
加上 axum/tower/tower-http/tokio/jsonwebtoken/validator 等外部依赖。

**不会**编译任何数据库代码：`alun-db`、`alun-db-core`、`alun-db-sql`、`alun-db-macro`、
`alun-db-build` 与各驱动（rusqlite / tokio-postgres / mysql_async）只经 `db` 与对应驱动 feature 进入
（`db` 不含任何驱动）；
模板、缓存、Kafka、任务、FS、插件同理。`alun-core` 依然是零内部依赖的叶子，
所以“最小 Web 构建无需 DB 代码”这一结论仍然成立。

对于需要数据库的项目：

```toml
[dependencies]
alun = { version = "0.3", features = ["db"] }
```

额外进入构建图的内部 crate：

```text
alun-db → alun-db-core, alun-db-sql, alun-db-macro
        → alun-db-driver-sqlite            （db 自带）
        → alun-db-driver-postgres / -mysql （仅当开 postgres / mysql）
        →（构建期）alun-db-build
```

对应外部依赖随驱动而定：sqlite 为 rusqlite 0.40.2（bundled）；再加 `postgres` 才拉入
tokio-postgres 0.7.18；再加 `mysql` 才拉入 mysql_async 0.37.1。
其余能力（template/cache/plugin/kafka/task/fs）仍不会编译。

连 PostgreSQL / MySQL 时写 `features = ["db", "postgres"]`（或 `"mysql"`）；
只写 `db` 会在建池时报 `UnsupportedFeature { feature: "driver", dialect: "postgres" }`。

> 提示：`db` feature 会连带打开 `config`（即 `alun-web/config`），因为它同时要把
> `alun-web` 的 `db` 门控打开；这是 `feature` 声明里写死的传递关系，不是多余的依赖。

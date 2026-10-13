# alun

Alun：一个**配置驱动、插件可扩展、开箱即用**的 Rust Web 框架门面（facade）。

`alun` 是整个框架的出口：它把全部 crate 再导出一遍，**业务项目通常只需要声明这一个依赖** ——
路由、配置、`#[derive(Model)]`、`Res<T>` 全部来自 `alun::prelude::*`，
你的 `Cargo.toml` 里不需要写 `alun-db` / `alun-db-core`。

## 五分钟上手

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
        .parse_cli()               // 识别 cargo run -- gen-config / print-config / profile=xxx
        .module(api::register)     // 注册路由（register 由 #[alun::routes] 自动生成）
        .start()                   // 读 config/config.toml，端口取自 [server].listen
        .await
        .unwrap();
}

#[alun::routes]
mod api {
    use alun::prelude::*;

    #[alun::get("/users")]
    pub async fn list_users() -> Res<String> {
        Res::ok("[]".into())
    }
}
```

`App::scan()` 是另一条等价的注册通道（收集 `#[alun::get]` 等宏标注的处理器），
但 `#[alun::routes]` + `App::module()` 更直观、可裁剪、IDE 能跳转，新代码建议优先用它。

## Crate 地图

| Crate | 主要 API |
|-------|---------|
| [`alun-core`](https://crates.io/crates/alun-core) | `Error` / `Result`、`Plugin` / `PluginManager`、`Res` / `ResResult` / `ApiError` / `PageData` / `PageQuery` / `codes` |
| [`alun-web`](https://crates.io/crates/alun-web) | `App`、`AlunRouter`、`JWT`、`TokenClaims` / `AuthClaims`、`ValidatedJson` / `ValidateExt`、`PageResponseExt`、全局资源访问器、SSE、可观测 |
| [`alun-utils`](https://crates.io/crates/alun-utils) | `Sid`、`Mask`、`Crypto`、`Valid`、`Date`、`Export` / `Import`、`StrExt`、单号引擎 |
| [`alun-config`](https://crates.io/crates/alun-config) | `AppConfig`、`ConfigManager` |
| [`alun-log`](https://crates.io/crates/alun-log) | `init(&LogConfig)` —— tracing 引导 |
| [`alun-db`](https://crates.io/crates/alun-db) | `Db`、`Row`、`Page<T>`、`Model` 派生、`CrudExt` / `PersistExt`、`ModelQuery`、`ChangeSet` |
| [`alun-cache`](https://crates.io/crates/alun-cache) | `Cache`、`LocalCache`、`RedisCache`、`SharedCache`、`CacheNamespace`、`create_cache` |
| [`alun-template`](https://crates.io/crates/alun-template) | `TemplateEngine` |
| [`alun-plugin`](https://crates.io/crates/alun-plugin) | `serial_plugin`、`notification`、`scheduler`、`cache_plugin`、`async_task`、`sid_plugin` |
| [`alun-kafka`](https://crates.io/crates/alun-kafka) | `KafkaProducer`、`KafkaConsumer`、`KafkaPlugin` |
| [`alun-task`](https://crates.io/crates/alun-task) | `TaskHandler`、`TaskStorage`、`TaskProducer`、`TaskWorker`、`RetryScanner`、`TaskPlugin` |
| [`alun-fs`](https://crates.io/crates/alun-fs) | `StorageBackend`、`LocalFs`、`FsPlugin`、`FileMeta` |
| [`alun-macros`](https://crates.io/crates/alun-macros) | 下面这些过程宏 |

`alun::prelude::*` 除了框架类型，还会带上**完整的 `alun-db` prelude**
（`Model` 派生、`TypeMapper`、`CrudExt` / `PersistExt`、`ModelQuery` / `ChangeSet`、`sql!`、`DbError` …）
以及生态类型模块（`alun::prelude::chrono`、`alun::prelude::rust_decimal`、`alun::prelude::uuid` …）；
派生宏会从调用方实际声明的 crate 路径解析内核，所以你**不需要**自己声明 `chrono` / `uuid` / `rust_decimal`。

## 宏

再导出自 `alun-macros`：

| 宏 | 用途 |
|-------|---------|
| `#[alun::get("/path")]` / `#[alun::post]` / `#[alun::put]` / `#[alun::patch]` / `#[alun::delete]` | 注册 HTTP 处理器 |
| `#[alun::routes]` | 标在 `mod` 上，整块显式注册并生成 `register` 函数（推荐） |
| `#[alun::controller("/prefix")]` | 标在 `impl` 块上，块内方法统一加前缀 |
| `#[alun::permission(path = …, method = …, permission = …)]` | 方法级权限校验 |
| `#[alun::no_auth("/path")]` | 标记免认证路径 |
| `#[alun::plugin]` | 标记一个插件类型 |
| `#[alun::task_handler(task_type = …, topic = …)]` | 自动注册异步任务处理器 |

## Feature

| Feature | 启用内容 |
|---------|---------|
| `default` | `core` —— HTTP 最小集：路由 + 中间件 + 配置 + 日志，无外部重量级依赖 |
| `core` | 同上（`config` 恒定开启） |
| `mini` 预设 | `core` + `sqlite` + `cache` —— 零外部服务，本机自足 |
| `lite` 预设 | `core` + `postgres` + `cache` + `plugin` + `fs-local` —— 单库生产推荐；模板不在预设内，用得到就自己加 `template` |
| `full` 预设 | 以上 + `mysql` + `kafka` + `task-kafka` + `fs-minio` + `sse` + `metrics-prometheus` + `xlsx` + `template` |
| `db` | `alun-db` 门面 + `alun-web/db` —— **数据库能力，不含驱动** |
| `sqlite` / `postgres` / `mysql` | 对应驱动（各自隐含 `db`） |
| `chrono` / `time` / `jiff` / `rust_decimal` / `bigdecimal` / `uuid` / `ulid` / `json` / `ipnetwork` | 模型生态类型映射（隐含 `db`），缺失是编译期错误 |
| `xlsx` | 真 XLSX（转发 `alun-utils/xlsx`）：`Export::to_xlsx_*` / `Import::from_xlsx*` 才存在 |
| `xss` | HTML 净化（转发 `alun-utils/xss`），prelude 里 `xss::sanitize_html(..)` |
| `template` / `cache` / `plugin` / `fs` | 模板 / 缓存 / 插件 / 文件存储 |
| `sse` | 实时推送：`Broadcaster` / `sse_json` |
| `metrics` | 埋点（HTTP / DB / 缓存 / 安全指标），**未装 recorder 时是 no-op**，可安全常开 |
| `metrics-prometheus` | 在 `metrics` 之上装 Prometheus recorder 并挂 `/metrics` 端点 |
| `kafka` / `task` / `task-kafka` | Kafka 集成 / 异步任务模型 / 含 Kafka worker（`task-kafka` 需 CMake + C 工具链） |

```toml
# 精简构建：不要 Kafka / task / fs / cache / plugin
alun = { version = "0.3", default-features = false, features = ["db", "template"] }

# 常见生产组合
alun = { version = "0.3", features = ["postgres", "cache", "sse", "metrics-prometheus", "xlsx"] }
```

> ⚠️ **只写 `db` 不选驱动没用**（建池报 `UnsupportedFeature { feature: "driver" }`）；
> 生态类型 feature 缺失则是编译期错误 —— 框架刻意区分这两类「缺依赖」。

## 数据库与分页

`alun-db` 与 `alun-core` 互不依赖，所以「数据库分页容器」`Page<T>`
（`list` / `total` / `page` / `size` / `pages`）与「响应分页容器」`PageData<T>`
（`list` / `total` / `page` / `page_size`）之间的桥是 `PageResponseExt`。
`Row` **不实现 `Serialize`**，请先映射成自己的 DTO。

```rust,ignore
use alun::prelude::*;
use serde::Serialize;

#[derive(Serialize)]
struct UserRes { id: String, name: String }

#[alun::get("/users")]
async fn list_users(Query(pq): Query<PageQuery>) -> ResResult<PageData<Vec<UserRes>>> {
    let page = db()
        .paginate("SELECT * FROM users ORDER BY id", pq.page, pq.page_size)
        .await?
        .map(|row| UserRes {
            id: row.get_typed("id").unwrap_or_default(),
            name: row.get_typed("name").unwrap_or_default(),
        });
    Ok(page.into_res())     // Res<PageData<Vec<UserRes>>>
}
```

写一行动态数据：

```rust,ignore
use alun::prelude::*;

let mut row = Row::new().with_entity("users", "id");
row.insert("name", "alice");
let saved = db().insert_row(&row).await?;                  // 主键会自动回填
let found = db().find_row_by_pk("users", "id", "1").await?; // Option<Row>
db().delete_row(&saved).await?;
```

## 安全默认

- 安全响应头、请求 ID、请求日志中间件默认由 `config.toml` 驱动装配
- 防重放 `NonceLayer`、幂等键 `IdempotencyLayer`（需 `cache` feature + 对应 `[middleware.*]` 段）
- HTML 净化是**可选**的、不默认开：开 `alun` 的 `xss` feature，然后显式调用
  `xss::sanitize_html` / `sanitize_html_strict` / `has_potential_xss`
- 5xx 对前端只返回模糊信息，详细信息只写日志

## 文档

- 主框架说明与上手：[项目根 `README.md`](../README.md)
- 入门与决策：[docs/guides/getting-started.md](../docs/guides/getting-started.md)
- 数据库实操：[docs/guides/database.md](../docs/guides/database.md)
- 编码速查（人和 AI 都适用）：[skill-alun/SKILL.md](../skill-alun/SKILL.md)，另有 12 份 `references/`（按能力拆的模块文档）
- crate 级 API 参考：[docs/reference/crates.md](../docs/reference/crates.md)

# 02 — Crate 详细参考

本文档按层级对所有 Crate 中的关键结构体、Trait、函数和枚举进行详细说明。

***

## 2.1 `alun-core` — 核心抽象层

**路径**: `alun-core/src/`

是整个框架的"语言"基础——定义所有子 crate 共享的错误类型、插件协议、API 响应体结构和分页参数。零 Web 框架依赖。

### 关键结构体

#### `Error` （[error.rs](file:///Volumes/zdh/projects/alun/alun/alun-core/src/error.rs)）

框架统一错误类型，支持从 `std::io::Error`、`String`、`&str`、`ApiError` 自动转换。

| 变体                               | 说明                    |
| -------------------------------- | --------------------- |
| `Error::Config(String)`          | 配置加载或解析错误             |
| `Error::Plugin { name, source }` | 插件生命周期错误（携带插件名和底层错误源） |
| `Error::Server(String)`          | 服务器启动或运行失败            |
| `Error::Io(std::io::Error)`      | IO 操作错误               |
| `Error::Msg(String)`             | 通用业务错误消息              |

```rust
pub type Result<T> = std::result::Result<T, Error>;
```

#### `Res<T>` （[api.rs](file:///Volumes/zdh/projects/alun/alun/alun-core/src/api.rs)）

统一 API 响应体，实现 `axum::IntoResponse`（需 `axum` feature）。

| 字段     | 类型          | 说明         |
| ------ | ----------- | ---------- |
| `code` | `i32`       | 业务码，0 表示成功 |
| `msg`  | `String`    | 提示信息       |
| `data` | `Option<T>` | 数据载荷       |

**关键方法**：

| 方法                                        | 说明                     |
| ----------------------------------------- | ---------------------- |
| `Res::ok(data)`                           | 成功响应（code=0, msg="ok"） |
| `Res::ok_with_msg(data, msg)`             | 成功响应 + 自定义消息           |
| `Res::ok_empty()`                         | 成功响应（无 data）           |
| `Res::ok_msg(msg)`                        | 成功响应仅消息                |
| `Res::fail(code, msg)`                    | 失败响应                   |
| `Res::page(list, total, page, page_size)` | 分页响应（T 为 PageData）     |

#### `ApiError` （[api.rs](file:///Volumes/zdh/projects/alun/alun/alun-core/src/api.rs)）

对外暴露的统一 API 错误类型。HTTP 状态码使用 `u16` 存储，与 Web 框架解耦。实现了 `IntoResponse`（需 `axum` feature），5xx 错误自动日志记录内部详情。

**工厂方法一览**：

| 方法                                          | HTTP 状态码 | 业务码 |
| ------------------------------------------- | -------- | --- |
| `ApiError::bad_request(msg)`                | 400      | 400 |
| `ApiError::unauthorized(msg)`               | 401      | 401 |
| `ApiError::forbidden(msg)`                  | 403      | 403 |
| `ApiError::not_found(msg)`                  | 404      | 404 |
| `ApiError::method_not_allowed(msg)`         | 405      | 405 |
| `ApiError::conflict(msg)`                   | 409      | 409 |
| `ApiError::unprocessable_entity(msg)`       | 422      | 422 |
| `ApiError::too_many_requests(msg)`          | 429      | 429 |
| `ApiError::internal(msg)`                   | 500      | 500 |
| `ApiError::internal_masked(public, detail)` | 500      | 500 |
| `ApiError::service_unavailable(msg)`        | 503      | 503 |

#### `PageQuery` / `PageData<T>` （[api.rs](file:///Volumes/zdh/projects/alun/alun/alun-core/src/api.rs)）

分页查询公共类型。

**`PageQuery`** — 分页参数：

- `new(page, page_size)` — 创建，自动规整到合法范围（page>=1, 1<=page\_size<=1000）
- `offset()` — 计算 SQL OFFSET
- `limit()` — 获取 LIMIT 值

**`PageData<T>`** — 分页数据结构：

- `list: T` — 数据列表
- `total: u64` — 总条数
- `page: u64` — 当前页码
- `page_size: u64` — 每页条数

### 关键 Trait

#### `Plugin` （[plugin.rs](file:///Volumes/zdh/projects/alun/alun/alun-core/src/plugin.rs)）

插件生命周期协议。

```rust
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

#### `PluginManager` （[plugin.rs](file:///Volumes/zdh/projects/alun/alun/alun-core/src/plugin.rs)）

插件管理器——负责注册、拓扑排序启动、逆序关闭。

| 方法                        | 说明           |
| ------------------------- | ------------ |
| `new()`                   | 创建空管理器       |
| `add(plugin)`             | 手动注册插件（链式调用） |
| `add_discovered(plugins)` | 批量注册编译期发现的插件 |
| `start_all()`             | 拓扑排序后依次启动    |
| `stop_all()`              | 逆序关闭         |
| `check_duplicate_names()` | 检查名称重复       |

### 错误码常量

定义在 `alun_core::api::codes` 模块中：

| 常量                            | 值   | 含义    |
| ----------------------------- | --- | ----- |
| `codes::OK`                   | 0   | 成功    |
| `codes::BAD_REQUEST`          | 400 | 参数错误  |
| `codes::UNAUTHORIZED`         | 401 | 未登录   |
| `codes::FORBIDDEN`            | 403 | 无权限   |
| `codes::NOT_FOUND`            | 404 | 资源不存在 |
| `codes::METHOD_NOT_ALLOWED`   | 405 | 方法不允许 |
| `codes::CONFLICT`             | 409 | 数据冲突  |
| `codes::UNPROCESSABLE_ENTITY` | 422 | 验证失败  |
| `codes::TOO_MANY_REQUESTS`    | 429 | 限流    |
| `codes::INTERNAL`             | 500 | 服务器错误 |
| `codes::SERVICE_UNAVAILABLE`  | 503 | 服务不可用 |

### 使用示例

```rust
use alun::prelude::*;

// ── Res<T> 响应 ──
async fn get_user() -> Res<UserModel> {
    Res::ok(user)                                   // { code:0, msg:"ok", data:user }
}

async fn create_user() -> Res<String> {
    Res::ok_with_msg("u1", "创建成功")              // 自定义消息
}

async fn list_users() -> Res<PageData<Vec<UserModel>>> {
    Res::page(users, total, 1, 20)                 // 分页响应
}

// ── Result<Res<T>, ApiError> 带错误 ──
async fn find_user(Path(id): Path<String>) -> Result<Res<UserModel>, ApiError> {
    if id.is_empty() {
        return Err(ApiError::bad_request("ID 不能为空"));
    }
    let user = db.find_user(&id).await
        .ok_or_else(|| ApiError::not_found("用户不存在"))?;
    Ok(Res::ok(user))
}

// ── 错误屏蔽（生产环境推荐） ──
async fn risky_operation() -> Result<Res<()>, ApiError> {
    do_something().await.map_err(|e| {
        tracing::error!("内部错误: {:?}", e);
        ApiError::internal_masked("服务器内部错误", format!("{:?}", e))
    })
}

// ── 分页查询 ──
use alun_core::PageQuery;
let pq = PageQuery::new(2, 20);     // 第2页，每页20条
assert_eq!(pq.offset(), 20);        // SQL OFFSET
assert_eq!(pq.limit(), 20);         // SQL LIMIT

// ── Plugin Trait 实现 ──
use async_trait::async_trait;
struct MetricsPlugin;

#[async_trait]
impl Plugin for MetricsPlugin {
    fn name(&self) -> &str { "metrics" }
    async fn start(&self) -> Result<()> { Ok(()) }
    async fn stop(&self) -> Result<()> { Ok(()) }
}

// ── PluginManager 使用 ──
let mut manager = PluginManager::new();
manager.add(Box::new(MetricsPlugin));
manager.check_duplicate_names()?;
manager.start_all().await?;
// ... 应用运行 ...
manager.stop_all().await;
```

***

## 2.2 `alun` — 门面 Crate

**路径**: `alun/src/lib.rs`

用户唯一直接依赖的 crate。通过 `prelude` 模块统一导出所有公共符号，proc macro 直接 re-export。

### 模块 `alun::prelude`

```rust
pub mod prelude {
    pub use alun_core::{Result, Error, Plugin, PluginManager, Res, ResResult, ApiError, PageData, PageQuery, codes};
    pub use alun_web::{App, AlunRouter, TokenClaims, TokenType, UserId, AuthClaims, ValidatedJson, JWT};
    pub use alun_web::middleware::{NonceLayer, IdempotencyLayer};
    pub use alun_config::AppConfig;
    #[cfg(feature = "db")]
    pub use alun_db::{Db, Row, ActiveTx, Isolation, Hook, NullHook, HookChain, factory};
    #[cfg(feature = "db")]
    pub use alun_web::resources::{db, try_db, set_db};
    #[cfg(feature = "cache")]
    pub use alun_web::resources::{cache, try_cache, set_cache};
    pub use alun_web::resources::{cfg, config, try_config, set_config};
    pub use alun_web::resources::{upload_path, try_upload_path, set_upload_path};
    pub use alun_web::resources::{download_path, try_download_path, set_download_path};
    #[cfg(feature = "template")]
    pub use alun_web::resources::{render_template, try_template, set_template};
    pub use serde_json::{json, Value as JsonValue};
    #[cfg(feature = "template")]
    pub use alun_template::TemplateEngine;
    #[cfg(feature = "cache")]
    pub use alun_cache::{Cache, CacheStats, LocalCache};
    pub use axum::response::Json as AxumJson;
    pub use axum::extract::{Path, Query};
    pub use axum::Extension;
}
```

### Proc Macro 导出

| 宏                                               | 说明                                         |
| ----------------------------------------------- | ------------------------------------------ |
| `#[alun::get("/path")]`                         | 标记异步函数为 GET 路由处理器                          |
| `#[alun::post("/path")]`                        | 标记异步函数为 POST 路由处理器                         |
| `#[alun::put("/path")]`                         | 标记异步函数为 PUT 路由处理器                          |
| `#[alun::delete("/path")]`                      | 标记异步函数为 DELETE 路由处理器                       |
| `#[alun::controller("/prefix")]`                | 标记 impl 块为路由控制器组                           |
| `#[alun::plugin]`                               | 标记结构体为插件                                   |
| `#[alun::permission(path, method, permission)]` | 方法级权限校验                                    |
| `#[alun::no_auth("/path")]`                     | 标记无需认证的路由（登录用户仍可获取用户信息）                    |
| `#[alun::task_handler(task_type = N, ...)]`     | 标记 TaskHandler 自动注册到任务框架（需 `task` feature） |

### Features

| Feature            | 效果                                       |
| ------------------ | ---------------------------------------- |
| `default` / `full` | 启用全部功能 crate                             |
| `db`               | 引入 `alun-db`                             |
| `template`         | 引入 `alun-template` + `alun-web/template` |
| `cache`            | 引入 `alun-cache`                          |
| `plugin`           | 引入 `alun-plugin`                         |
| `kafka`            | 引入 `alun-kafka`                          |
| `task`             | 引入 `alun-task`（异步任务框架）                   |
| `fs`               | 引入 `alun-fs`                             |
| `xss`              | 启用 HTML/XSS 净化工具（`alun-utils::xss`）      |

### 顶层 Re-export

除 `prelude` 外，`alun` crate 还在顶层直接导出以下符号，可通过 `use alun::*` 或直接引用：

| 导出项                                                                                               | 来源         | 说明                                 |
| ------------------------------------------------------------------------------------------------- | ---------- | ---------------------------------- |
| `validate_uuid`、`validate_mobile`、`validate_password_strength`、`validate_id_card`、`validate_date` | `alun-web` | 自定义 validator 校验函数                 |
| `ValidateExt`                                                                                     | `alun-web` | 为 DTO 提供 `validate_or_reject()` 方法 |
| `web`                                                                                             | `alun-web` | 别名为 `alun_web`                     |

> `#[validate(custom(function = "validate_uuid"))]` 宏属性使用函数名字符串引用，需确保该函数在当前模块可访问。建议在文件顶部添加 `use alun::validate_uuid;` 便于宏正确解析。

### 使用示例

```rust
// ── Proc Macro 路由 ──
use alun::{App, Res};

#[alun::get("/api/users")]
async fn list_users() -> Res<Vec<UserModel>> {
    Res::ok(users)
}

#[alun::post("/api/users")]
async fn create_user(ValidatedJson(req): ValidatedJson<CreateUserReq>) -> Result<Res<UserModel>, ApiError> {
    let user = save_user(req).await?;
    Ok(Res::ok(user))
}

#[alun::controller("/api/admin")]
impl AdminController {
    #[alun::get("/dashboard")]
    async fn dashboard() -> Res<String> {
        Res::ok("admin dashboard".into())
    }

    #[alun::delete("/users/{id}")]
    async fn delete_user(Path(id): Path<String>) -> Result<Res<()>, ApiError> {
        remove_user(&id).await?;
        Ok(Res::ok_empty())
    }
}

// ── 插件标记 ──
#[alun::plugin]
struct MyCustomPlugin;

// ── 权限校验注解 ──
#[alun::permission("/api/admin/*", "GET", "admin:read")]
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

## 2.3 `alun-web` — Web 核心层

**路径**: `alun-web/src/`

提供 App 构建器、路由注册、中间件体系和共享状态。基于 axum + tower 构建。

### 关键结构体

#### `App` （[app.rs](file:///Volumes/zdh/projects/alun/alun/alun-web/src/app.rs)）

框架唯一入口，Builder 模式，所有路由注册方法返回 `Self`。无泛型状态参数——运行时资源（Db/Cache/Config/Template）通过全局函数访问。

| 方法                                              | 说明                                  |
| ----------------------------------------------- | ----------------------------------- |
| `App::new()`                                    | 从默认 `config/` 目录加载配置                |
| `App::from_config()`                            | 同上，语义明确                             |
| `App::from_config_dir(dir)`                     | 从指定目录加载配置                           |
| `App::with_config(cfg)`                         | 直接使用 `AppConfig`（跳过文件加载）            |
| `.get(path, handler)`                           | 注册 GET 路由                           |
| `.post(path, handler)`                          | 注册 POST 路由                          |
| `.put(path, handler)`                           | 注册 PUT 路由                           |
| `.delete(path, handler)`                        | 注册 DELETE 路由                        |
| `.route(method, path, handler)`                 | 通用路由注册                              |
| `.group(prefix, closure)`                       | 路由分组（嵌套前缀）                          |
| `.merge(prefix, sub_router)`                    | 合并已构建的 AlunRouter                   |
| `.scan()`                                       | 扫描 Proc Macro 注解自动注册路由              |
| `.with_permission(method, path, handler, perm)` | 注册带方法级权限校验的路由                       |
| `.with_role(method, path, handler, role)`       | 注册带方法级角色校验的路由                       |
| `.with_startup_hook(hook)`                      | 注册启动回调（全局资源就绪后、插件启动前执行）             |
| `.with_middleware_hook(hook)`                   | 注册中间件注入钩子（框架中间件链构建完毕后回调）            |
| `.plugin(plugin)`                               | 注册插件                                |
| `.parse_cli()`                                  | 解析 CLI（`gen-config`/`print-config`） |
| `.start()`                                      | 启动（端口从配置文件读取）                       |
| `.serve(addr)`                                  | 指定地址启动                              |

#### `AlunRouter` （[router.rs](file:///Volumes/zdh/projects/alun/alun/alun-web/src/router.rs)）

路由注册器——延迟构建 axum Router。所有路由以闭包暂存，`into_axum()` 一次性构建。

| 方法                                        | 说明                  |
| ----------------------------------------- | ------------------- |
| `new()`                                   | 创建空注册器              |
| `add_get/path, handler)`                  | 注册 GET              |
| `add_post(path, handler)`                 | 注册 POST             |
| `add_put(path, handler)`                  | 注册 PUT              |
| `add_delete(path, handler)`               | 注册 DELETE           |
| `add_route(method, path, handler)`        | 通用方法注册              |
| `add_get_with_layer(path, handler, wrap)` | 带 tower Layer 的 GET |
| `merge(prefix, sub)`                      | 合并子路由               |
| `into_axum()`                             | 构建最终 axum Router    |

#### `resources` 模块 （[resources.rs](file:///Volumes/zdh/projects/alun/alun/alun-web/src/resources.rs)）

全局资源管理模块，基于标准库 `OnceLock` 实现，在框架启动时初始化，所有 Handler 可直接调用全局函数访问。

**全局函数**：

| 函数                           | 说明                            |
| ---------------------------- | ----------------------------- |
| `db()`                       | 获取全局数据库实例（需 feature = "db"）   |
| `cache()`                    | 获取全局缓存实例（需 feature = "cache"） |
| `config()`                   | 获取全局配置管理器 Arc<ConfigManager>  |
| `cfg()`                      | 快捷获取配置引用 AppConfig            |
| `render_template(name, ctx)` | 渲染模板（需 feature = "template"）  |
| `set_db(db)`                 | 初始化数据库（框架启动时调用）               |
| `set_cache(cache)`           | 初始化缓存（框架启动时调用）                |
| `set_config(config)`         | 初始化配置（框架启动时调用）                |
| `set_template(engine)`       | 初始化模板引擎（框架启动时调用）              |
| `try_db()`                   | 安全获取数据库（返回 Option）            |
| `try_cache()`                | 安全获取缓存（返回 Option）             |
| `try_config()`               | 安全获取配置（返回 Option）             |
| `try_template()`             | 安全获取模板引擎（返回 Option）           |
| `upload_path()`              | 获取上传文件存储目录路径                  |
| `try_upload_path()`          | 安全获取上传路径（返回 Option）           |
| `download_path()`            | 获取下载文件存储目录路径                  |
| `try_download_path()`        | 安全获取下载路径（返回 Option）           |
| `set_upload_path(path)`      | 初始化上传路径（框架启动时自动调用）            |
| `set_download_path(path)`    | 初始化下载路径（框架启动时自动调用）            |

#### `TokenClaims` （[middleware/mod.rs](file:///Volumes/zdh/projects/alun/alun/alun-web/src/middleware/mod.rs)）

JWT Token 声明结构。

| 字段            | 类型                  | 说明                       |
| ------------- | ------------------- | ------------------------ |
| `jti`         | `Option<String>`    | JWT 唯一标识，用于黑名单机制         |
| `sub`         | `String`            | 用户 ID（subject）           |
| `username`    | `Option<String>`    | 用户名                      |
| `roles`       | `Vec<String>`       | 角色列表                     |
| `permissions` | `Vec<String>`       | 权限列表                     |
| `token_type`  | `Option<TokenType>` | Token 类型（access/refresh） |
| `exp`         | `usize`             | 过期时间（Unix 时间戳，秒）         |
| `iat`         | `usize`             | 签发时间（Unix 时间戳，秒）         |

**关键方法**：`has_role()`, `has_any_role()`, `has_all_roles()`, `has_permission()`, `has_any_permission()`, `is_super_admin()`

#### `ValidatedJson<T>` （[extract.rs](file:///Volumes/zdh/projects/alun/alun/alun-web/src/extract.rs)）

带自动 JSON 解析校验的请求提取器（实现 `FromRequest`）。实现了 `Deref` / `DerefMut`，可像 `&T` 一样访问内部值。

若目标类型实现 `validator::Validate`，可通过 `.validate()` 方法执行字段级校验。

#### `ValidateExt` Trait （[extract.rs](file:///Volumes/zdh/projects/alun/alun/alun-web/src/extract.rs)）

为所有实现 `validator::Validate` 的类型提供便捷的 `validate_or_reject()` 方法，一行完成字段级校验，校验失败自动返回 422 `ApiError`。

#### 自定义验证函数

`alun-web` 内置了以下 validator 自定义校验函数，可直接用于 `#[validate(custom(function = "..."))]` 属性：

| 函数                                  | 说明                       |
| ----------------------------------- | ------------------------ |
| `validate_uuid(value)`              | 验证 UUID 格式（v1\~v7 均支持）   |
| `validate_mobile(value)`            | 验证手机号（中国大陆）              |
| `validate_password_strength(value)` | 验证密码强度（≥8位，含大小写+数字+特殊字符） |
| `validate_id_card(value)`           | 验证中国居民身份证号（含校验位）         |
| `validate_date(value)`              | 验证日期格式（YYYY-MM-DD）       |

### 中间件体系

| 中间件               | Layer 类型                        | 类型  | 说明                                                                                         |
| ----------------- | ------------------------------- | --- | ------------------------------------------------------------------------------------------ |
| SecurityHeaders   | `SecurityHeadersLayer`          | 全局  | 注入 6 个安全响应头（nosniff/frame/HSTS/CSP/Referrer/Permissions）                                   |
| RequestId         | `RequestIdLayer`                | 全局  | 为每个请求生成 UUID 请求 ID                                                                         |
| RequestLog        | `RequestLogLayer`               | 全局  | 记录请求路径/状态码/耗时/客户端IP（tracing 输出）；操作审计日志（含请求参数、响应体、脱敏、写库）请通过 `with_middleware_hook` 注入项目级中间件 |
| Auth              | `AuthLayer`                     | 全局  | JWT Bearer Token 验证 + 黑名单检查；支持 `ignore_paths` 配置和 `#[no_auth]` 宏注解                         |
| CORS              | `CorsLayer` (tower-http)        | 全局  | 跨域配置                                                                                       |
| Compression       | `CompressionLayer` (tower-http) | 全局  | Gzip 响应压缩                                                                                  |
| RateLimit         | `RateLimitLayer`                | 全局  | IP 滑动窗口限流                                                                                  |
| PermissionCheck   | `PermissionCheckLayer`          | 全局  | 路径权限规则校验（配置文件 + 宏注解）                                                                       |
| RequirePermission | `RequirePermissionLayer`        | 方法级 | 权限标识校验（any 匹配）                                                                             |
| RequireRole       | `RequireRoleLayer`              | 方法级 | 角色校验（any 匹配）                                                                               |
| Nonce             | `NonceLayer`                    | 方法级 | Nonce 防重放（按需，建议写操作路由）                                                                      |
| Idempotency       | `IdempotencyLayer`              | 方法级 | 幂等键（按需，建议订单/支付路由）                                                                          |

#### Auth 中间件认证行为

`AuthLayer` 支持两种方式定义无需认证的路径：

1. **配置文件**：`[middleware.auth.ignore_paths]` 列表
2. **宏注解**：`#[alun::no_auth("/path")]` 标记

两种方式会合并生效。

**认证逻辑**：

| 场景                               | 行为                                                     |
| -------------------------------- | ------------------------------------------------------ |
| `ignore_path` + 有效 Token         | 解析 Token，注入 `AuthClaims` 到 `extensions`，**跳过黑名单检查**，放行 |
| `ignore_path` + 无效 Token         | 忽略错误，**不注入用户信息**，放行                                    |
| `ignore_path` + 无 Token          | 直接放行                                                   |
| 非 `ignore_path` + 有效 Token（非黑名单） | 注入 `AuthClaims`，放行                                     |
| 非 `ignore_path` + 有效 Token（黑名单）  | **不注入用户信息**，放行                                         |
| 非 `ignore_path` + 无效/无 Token     | 返回 401                                                 |

#### PermissionCheck 中间件权限校验行为

`PermissionCheckLayer` 支持两种方式定义权限规则：

1. **配置文件**：`[middleware.permission.rules]` 列表
2. **宏注解**：`#[alun::permission(path, method, permission)]` 标记

两种方式会合并生效。**白名单模式**：未匹配任何规则的路径直接放行。

**配置示例**：

```toml
[ middleware.permission ]
enabled = true

[[ middleware.permission.rules ]]
path = "/api/admin"
methods = ["GET", "POST"]
permission = "admin:access"
```

**宏注解示例**：

```rust
#[alun::permission(path = "/api/admin/users", method = "GET", permission = "admin:read")]
#[alun::get("/api/admin/users")]
async fn list_users() -> Res<Vec<User>> {
    Res::ok(users)
}
```

### 分布式切片

| 静态变量                                 | 说明                                        |
| ------------------------------------ | ----------------------------------------- |
| `ROUTES: [fn(&mut AlunRouter)]`      | linkme 路由注册切片——`#[get]`/`#[post]` 宏注解在此汇集 |
| `PERMISSION_ROUTES: [PermissionDef]` | linkme 权限规则切片——`#[permission]` 宏注解汇集      |
| `NO_AUTH_ROUTES: [NoAuthDef]`        | linkme 无需认证路径切片——`#[no_auth]` 宏注解汇集       |

### 使用示例

```rust
// ── 最简启动（配置驱动） ──
#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        .get("/", || async { "Hello" })
        .serve("8080")
        .await
}

// ── Proc Macro + scan ──
#[alun::get("/api/health")]
async fn health() -> Res<&'static str> {
    Res::ok("OK")
}

#[tokio::main]
async fn main() {
    App::new().expect("初始化失败").scan().start().await.unwrap();
}

// ── 路由分组 ──
fn user_routes() -> AlunRouter {
    let mut r = AlunRouter::new();
    r.add_get("/", list_users);
    r.add_post("/", create_user);
    r
}

#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        .merge("/api/users", user_routes())
        .serve("8080")
        .await
}

// ── 带权限的路由 ──
#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        .with_permission(Method::GET, "/api/admin", admin_handler, "admin:read")
        .with_role(Method::DELETE, "/api/users/{id}", delete_user, "admin")
        .serve("8080")
        .await
}

// ── 自定义中间件注入（操作审计日志等） ──
#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        .with_middleware_hook(|router| {
            router.layer(axum::middleware::from_fn_with_state(
                my_audit_log_middleware,
            ))
        })
        .serve("8080")
        .await
}

// ── 无状态测试模式 ──
#[tokio::main]
async fn main() -> alun::Result<()> {
    App::empty()
        .get("/", || async { "OK" })
        .serve("8080")
        .await
}

// ── 配置管理器 ──
let cm = Arc::new(ConfigManager::load(Some("config".into())));
let port = &cm.get().server.listen;
cm.set_dynamic("feature.x", "enabled");
let val: Option<String> = cm.get_dynamic("feature.x");

// ──  JWT Token 管理 ──
use alun::prelude::JWT;

#[alun::post("/api/auth/login")]
async fn login(Json(req): Json<LoginReq>) -> Result<Res<LoginRes>, ApiError> {
    let jwt = JWT::from_config();
    let access = jwt.create_access_token("u1", Some("admin"), &["admin".into()], &["*:*".into()])
        .map_err(|e| ApiError::internal(e))?;
    let refresh = jwt.create_refresh_token("u1")
        .map_err(|e| ApiError::internal(e))?;
    Ok(Res::ok(LoginRes { access, refresh }))
}

#[alun::post("/api/auth/refresh")]
async fn refresh(Json(req): Json<RefreshReq>) -> Result<Res<LoginRes>, ApiError> {
    let jwt = JWT::from_config();
    let (access, refresh) = jwt.refresh(&req.refresh_token).await
        .map_err(|e| ApiError::unauthorized(e))?;
    Ok(Res::ok(LoginRes { access, refresh }))
}

#[alun::post("/api/auth/logout")]
async fn logout(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<()> {
    let jwt = JWT::from_config();
    jwt.logout(&claims).await;
    Res::ok(())
}

// ── 获取当前用户 ──
#[alun::get("/api/auth/me")]
async fn me(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<serde_json::Value> {
    Res::ok(json!({ "user_id": claims.sub, "roles": claims.roles, "permissions": claims.permissions }))
}

// ── Nonce 防重放（按需，写操作路由） ──
use alun_web::middleware::NonceLayer;
#[alun::post("/api/transfer")]
async fn transfer() -> Result<Res<()>, ApiError> {
    // 请求需携带 x-nonce 头，相同 nonce 返回 409 Conflict
    Ok(Res::ok_empty())
}
// 在 App 中通过 route_layer 包裹：
// router.route("/api/transfer", post(transfer)).route_layer(
//     NonceLayer::new(cache, Duration::from_secs(300))
// );

// ── Idempotency-Key 幂等键（按需，订单/支付路由） ──
use alun_web::middleware:{id}empotencyLayer;
#[alun::post("/api/order/create")]
async fn create_order(ValidatedJson(req): ValidatedJson<CreateOrderReq>) -> Result<Res<OrderModel>, ApiError> {
    // 请求需携带 x-idempotency-key 头，相同 key 返回首次缓存的响应
    Ok(Res::ok(order))
}
// 在 App 中通过 route_layer 包裹：
// router.route("/api/order/create", post(create_order)).route_layer(
//     IdempotencyLayer::new(cache, Duration::from_secs(86400))
// );

// ── TokenClaims 权限判断 ──
async fn check_access(claims: &TokenClaims) -> bool {
    claims.has_role("admin") || claims.has_permission("user:write")
}

// ── ValidatedJson 校验 ──
#[derive(Debug, Deserialize, Validate)]
struct CreateUserReq {
    #[validate(length(min = 1, max = 50))]
    name: String,
    #[validate(email)]
    email: String,
}

async fn create(ValidatedJson(req): ValidatedJson<CreateUserReq>) -> Result<Res<UserModel>, ApiError> {
    req.validate()
        .map_err(|e| ApiError::unprocessable_entity(e.to_string()))?;
    let user = save_user(req).await?;
    Ok(Res::ok(user))
}

// ── ValidateExt::validate_or_reject() 一行校验 ──
#[derive(Debug, Deserialize, Validate)]
struct RegisterReq {
    #[validate(email)]
    email: String,
    #[validate(custom(function = "validate_password_strength"))]
    password: String,
    #[validate(custom(function = "validate_uuid"))]
    invite_id: String,
}

async fn register(ValidatedJson(req): ValidatedJson<RegisterReq>) -> Result<Res<String>, ApiError> {
    req.validate_or_reject()?; // 一行完成所有字段校验，失败返回 422
    Ok(Res::ok("OK".into()))
}

// ── 获取上传目录路径 ──
use alun::upload_path;

#[alun::post("/api/file/upload")]
async fn upload_file() -> Res<String> {
    let path = upload_path();  // 返回 "uploads"（或 config.toml 中配置的值）
    let full = format!("{}/{}", path, "report_2024.pdf");
    // 执行文件保存 ...
    Res::ok(full)
}

// ── 获取下载目录路径 ──
use alun::download_path;

#[alun::get("/api/file/download/:name")]
async fn download_file(Path(name): Path<String>) -> Res<String> {
    let path = download_path();  // 返回 "downloads"（或 config.toml 中配置的值）
    let full = format!("{}/{}", path, name);
    Res::ok(full)
}

// ── 启用静态文件服务（config.toml） ──
// [static_files]
// enabled = true
// path = "static"         # 将 static/ 目录作为 Web 根目录对外提供
//
// 启动后 App 自动创建 static/ 目录，通过 ServeDir 挂载为 fallback，
// 所有未匹配 API 路由的请求返回 static/ 下对应文件。
```

***

## 2.4 `alun-db` — 数据库层

**路径**: `alun-db/src/`

提供 Row 模式数据访问、RAII 事务、Hook 生命周期、SQL 模板、迁移和多数据库适配。

### 关键结构体

#### `Db` （[db.rs](file:///Volumes/zdh/projects/alun/alun/alun-db/src/db.rs)）

数据库门面——统一 PostgreSQL/MySQL/SQLite 的 CRUD 接口。

| 方法                                                   | 说明                                      |
| ---------------------------------------------------- | --------------------------------------- |
| `find_by_id(table, id)`                              | 按主键（默认为 `id`）查询单条                       |
| `query_one(sql, params)`                             | 执行原始 SQL 查询（单条）                         |
| `query(sql, params)`                                 | 执行原始 SQL 查询（多条）                         |
| `query_page(sql, params, page)`                      | 分页查询，自动包裹 COUNT + LIMIT/OFFSET          |
| `count(sql, params)`                                 | COUNT 查询                                |
| `insert(row)`                                        | 插入单条（PG 用 RETURNING \*，MySQL/SQLite 回查） |
| `batch_insert(rows)`                                 | 批量插入                                    |
| `update(row)`                                        | 按 Row changes 和主键更新                     |
| `batch_update(table, sets, where_sql, where_params)` | 批量条件更新                                  |
| `delete_by_id(table, id)`                            | 按主键删除                                   |
| `batch_delete_by_ids(table, ids)`                    | 批量按 ID 删除                               |
| `execute(sql, params)`                               | 执行写操作                                   |
| `transaction(closure)`                               | 事务闭包，Ok→Commit，Err→Rollback             |

#### `Row` （[row.rs](file:///Volumes/zdh/projects/alun/alun/alun-db/src/row.rs)）

数据载体——像操作 Map 一样操作数据库行。

| 字段             | 类型                       | 说明                       |
| -------------- | ------------------------ | ------------------------ |
| `table`        | `Option<String>`         | 表名                       |
| `primary_keys` | `Vec<String>`            | 主键字段名（默认 `["id"]`）       |
| `data`         | `HashMap<String, Value>` | 字段数据                     |
| `changes`      | `HashSet<String>`        | 变更追踪——用于 UPDATE SET 精确字段 |

| 方法                                    | 说明                  |
| ------------------------------------- | ------------------- |
| `Row::table(name)`                    | 创建指定表的 Row          |
| `.primary_key(key)`                   | 设置主键                |
| `.primary_keys(keys)`                 | 设置复合主键              |
| `.id(value)`                          | 快捷设置主键值             |
| `.set(key, value)`                    | 设置字段值（自动加入 changes） |
| `.get(key)`                           | 获取字段值（Value）        |
| `.get_as::<T>(key)`                   | 反序列化获取字段值           |
| `.get_id()`                           | 获取主键值               |
| `.mark_all_changed()`                 | 标记所有字段为已修改          |
| `.has(key)`                           | 判断字段是否存在            |
| `.to_json()` / `Row::from_json(json)` | JSON 序列化            |

#### `ActiveTx` （[tx.rs](file:///Volumes/zdh/projects/alun/alun/alun-db/src/tx.rs)）

活跃事务句柄——通过 `Db::transaction()` 创建。

| 方法                       | 说明                 |
| ------------------------ | ------------------ |
| `execute(sql, params)`   | 在事务中执行写操作          |
| `query_one(sql, params)` | 在事务中执行查询           |
| `set_rollback_only()`    | 标记事务需回滚（即使闭包返回 Ok） |

**安全保证**：`Drop` 检测未提交/未回滚的事务并输出警告日志，`?` 操作符自然传播错误触发 Rollback。

#### `Isolation` （[tx.rs](file:///Volumes/zdh/projects/alun/alun/alun-db/src/tx.rs)）

事务隔离级别枚举：

| 变体                | 说明          |
| ----------------- | ----------- |
| `ReadUncommitted` | 读未提交（可能脏读）  |
| `ReadCommitted`   | 读已提交（默认）    |
| `RepeatableRead`  | 可重复读        |
| `Serializable`    | 串行化（最高隔离级别） |

#### `Migrator` （[migrate.rs](file:///Volumes/zdh/projects/alun/alun/alun-db/src/migrate.rs)）

数据库迁移管理器——扫描 `migrations/` 目录中 `*.up.sql`/`*.down.sql` 文件。

| 方法           | 说明                         |
| ------------ | -------------------------- |
| `run()`      | 执行所有未执行的 `.up.sql` 迁移      |
| `rollback()` | 回滚最近一个迁移（需对应的 `.down.sql`） |

#### `SqlTemplate` （[sql.rs](file:///Volumes/zdh/projects/alun/alun/alun-db/src/sql.rs)）

Jinja2 风格 SQL 模板引擎。

| 方法                     | 说明                 |
| ---------------------- | ------------------ |
| `add(name, sql)`       | 添加模板               |
| `get_raw(name)`        | 获取原始模板             |
| `render(name, params)` | 渲染（替换 `{{ key }}`） |

#### `Dialect` （[dialect.rs](file:///Volumes/zdh/projects/alun/alun/alun-db/src/dialect.rs)）

数据库方言枚举——封装不同数据库的 SQL 语法差异。

| 变体         | 占位符  | 引号      |
| ---------- | ---- | ------- |
| `Postgres` | `$1` | `""`    |
| `Mysql`    | `?`  | `` ` `` |
| `Sqlite`   | `?`  | `""`    |

方法：`placeholder(index)`, `quote(ident)`

#### `IdKind` （[idkind.rs](file:///Volumes/zdh/projects/alun/alun/alun-db/src/idkind.rs)）

主键 ID 类型自动识别。

| 变体       | 检测依据                    | PG SQL 转换            |
| -------- | ----------------------- | -------------------- |
| `Uuid`   | 36 字符 + 4 个 `-`         | `::uuid`             |
| `I64`    | `Value::Number` 且可转 i64 | `::bigint`           |
| `F64`    | `Value::Number` 且为 f64  | `::double precision` |
| `String` | 普通字符串                   | 无                    |
| `Bool`   | `Value::Bool`           | `::boolean`          |

### 关键 Trait

#### `Hook` （[hook.rs](file:///Volumes/zdh/projects/alun/alun/alun-db/src/hook.rs)）

CRUD 生命周期 Hook（所有方法默认空实现）。

```rust
#[async_trait]
pub trait Hook: Send + Sync {
    async fn before_insert(&self, row: &mut Row) -> DbResult<()> { ... }
    async fn after_insert(&self, row: &Row) -> DbResult<()> { ... }
    async fn before_update(&self, row: &mut Row) -> DbResult<()> { ... }
    async fn after_update(&self, row: &Row) -> DbResult<()> { ... }
    async fn before_delete(&self, table: &str, id: &str) -> DbResult<()> { ... }
    async fn after_delete(&self, table: &str, id: &str) -> DbResult<()> { ... }
}
```

**内置实现**：

- `NullHook` — 空 Hook
- `HookChain` — 多 Hook 链式聚合
- `TimestampHook` — 自动填充 `created_at`/`updated_at`

### 工厂函数

| 函数                                      | 说明                              |
| --------------------------------------- | ------------------------------- |
| `factory::create_db(config)`            | 从 `DatabaseConfig` 创建 Db + 连接测试 |
| `factory::create_db_if_enabled(config)` | 仅在 `enabled=true` 时创建           |

支持加密密码存储：配置 `password_encrypted = true`，密码字段填写 AES-GCM Base64 密文。

### 使用示例

```rust
// ── 创建数据库连接 ──
use alun_db::factory;
let db = factory::create_db(&config.database).await?;
factory::test_connection(&db).await?;            // 连接测试

// 启动时自动创建，Handler 中通过全局函数获取
async fn handler() -> Result<Res<Vec<Row>>, ApiError> {
    let rows = db().query("SELECT * FROM users LIMIT 50", &[]).await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(rows))
}

// ── Row 模式 CRUD ──
// 插入
let row = Row::table("users")
    .id(Sid::uuid())
    .set("name", "张三")
    .set("email", "zhangsan@example.com")
    .set("age", 28);
let inserted = db.insert(&row).await?;

// 按主键查询
let user: Option<Row> = db.find_by_id("users", "u1").await?;

// 更新（仅 changes 中的字段进入 SET）
let mut row = db.find_by_id("users", "u1").await?.unwrap();
row.set("age", 29);
let updated = db.update(&row).await?;

// 删除
let deleted: bool = db.delete_by_id("users", "u1").await?;
let count: u64 = db.batch_delete_by_ids("users", &["u1", "u2"]).await?;

// ── 原生 SQL ──
let user = db.query_one("SELECT * FROM users WHERE id = $1", &["1"]).await?;
let users: Vec<Row> = db.query("SELECT * FROM users WHERE active = $1 ORDER BY id", &["true"]).await?;
let total: i64 = db.count("SELECT COUNT(*) FROM users WHERE active = $1", &["true"]).await?;

// ── 分页查询 ──
use alun_core::PageQuery;
let (rows, total) = db.query_page(
    "SELECT * FROM users ORDER BY created_at DESC",
    &[],
    &PageQuery::new(1, 20),
).await?;

// ── 批量插入 ──
let rows: Vec<Row> = users.iter().map(|u| {
    Row::table("users")
        .id(Sid::uuid())
        .set("name", &u.name)
        .set("email", &u.email)
}).collect();
let inserted = db.batch_insert(&rows).await?;

// ── 批量条件更新 ──
use std::collections::HashMap;
let mut sets = HashMap::new();
sets.insert("status", "inactive");
db.batch_update("users", sets, "created_at < $1", &["2024-01-01"]).await?;

// ── RAII 事务 ──
db.transaction(|tx| async move {
    tx.execute("UPDATE accounts SET balance = balance - 100 WHERE id = $1", &["a1"]).await?;
    tx.execute("UPDATE accounts SET balance = balance + 100 WHERE id = $1", &["a2"]).await?;
    let row = tx.query_one("SELECT balance FROM accounts WHERE id = $1", &["a1"]).await?;
    if row.get("balance").as_i64().unwrap_or(0) < 0 {
        tx.set_rollback_only();                  // 标记回滚
    }
    // Ok → Commit, Err/Drop → Rollback
    Ok(())
}).await?;

// ── TimestampHook ──
use alun_db::{TimestampHook, Hook, HookChain, NullHook};
let hook = HookChain::new()
    .add(TimestampHook::new("created_at", "updated_at"));
// 在 Db 创建时注入 Hook

// ── SQL 模板 ──
use alun_db::SqlTemplate;
let mut tpl = SqlTemplate::new();
tpl.add("search", r#"
    SELECT * FROM users WHERE 1=1
    {% if name %} AND name LIKE '%{{ name }}%' {% endif %}
    {% if status %} AND status = {{ status }} {% endif %}
    ORDER BY created_at DESC
"#);
let mut params = HashMap::new();
params.insert("name".into(), "张三".into());
let sql = tpl.render("search", &params)?;
let rows = db.query(&sql, &[]).await?;

// ── 数据库迁移 ──
use alun_db::Migrator;
let migrator = Migrator::new(&db.pool, "migrations");
migrator.run().await?;               // 执行所有未执行的 .up.sql
migrator.rollback().await?;          // 回滚最近一次迁移

// ── 自定义 Hook ──
struct AuditHook;
#[async_trait]
impl Hook for AuditHook {
    async fn after_insert(&self, row: &Row) -> DbResult<()> {
        tracing::info!("审计 - 新增 {}: {:?}", row.table.as_deref().unwrap_or("?"), row.get_id());
        Ok(())
    }
}
```

***

## 2.5 `alun-config` — 配置系统

**路径**: `alun-config/src/`

### 关键结构体

#### `AppConfig` （[lib.rs](file:///Volumes/zdh/projects/alun/alun/alun-config/src/lib.rs)）

完整应用配置的根结构体，包含 `server`、`log`、`database`、`redis`、`cache`、`middleware`、`router`、`plugins`、`upload`、`download`、`template`、`static_files`、`custom` 等子配置。

#### `ConfigManager` （[lib.rs](file:///Volumes/zdh/projects/alun/alun/alun-config/src/lib.rs)）

配置管理器。

| 方法                                     | 说明                   |
| -------------------------------------- | -------------------- |
| `ConfigManager::load(config_dir)`      | 从目录加载配置，自动检测 profile |
| `get()`                                | 获取静态配置引用             |
| `get_dynamic(key)`                     | 获取动态配置值              |
| `set_dynamic(key, value)`              | 设置运行时动态配置            |
| `remove_dynamic(key)`                  | 删除动态配置               |
| `ConfigManager::generate_default(dir)` | 生成默认配置文件             |

### 配置子结构体

| 结构体                               | 说明                                                                           |
| --------------------------------- | ---------------------------------------------------------------------------- |
| `ServerConfig`                    | 监听地址配置                                                                       |
| `LogConfig`                       | 日志级别/格式/目录                                                                   |
| `DatabaseConfig`                  | 数据库类型/连接/连接池/迁移                                                              |
| `RedisConfig`                     | Redis 连接                                                                     |
| `CacheConfig`                     | 缓存类型/容量/TTL                                                                  |
| `MiddlewareConfig`                | 中间件总开关（含 security\_headers/auth/cors/compression/rate\_limit/permission 子配置） |
| `RouterConfig`                    | 全局路由前缀、404 自定义处理                                                             |
| `PluginsConfig`                   | 插件启用列表                                                                       |
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

### 环境配置模块 `env`

| 函数                         | 说明                                           |
| -------------------------- | -------------------------------------------- |
| `detect_profile()`         | 检测当前 profile（CLI > `ALUN_PROFILE` > `"dev"`） |
| `parse_args()`             | 解析命令行（`gen-config`/`print-config`）           |
| `merge_env_overrides(cfg)` | `ALUN_` 前缀环境变量覆盖配置                           |

**配置加载流程**：

1. `config/config.toml` → 基础配置
2. `config/config-{profile}.toml` → profile 覆盖（按需）
3. `ALUN_*` 环境变量 → 最终覆盖

### 使用示例

```rust
use alun_config::ConfigManager;
use std::sync::Arc;

// ── 加载配置 ──
let cm = Arc::new(ConfigManager::load(Some("config".into())));

// ── 读取静态配置 ──
let port = &cm.get().server.listen;             // 监听端口
let db_enabled = cm.get().database.enabled;      // 数据库是否启用
let jwt_secret = &cm.get().middleware.auth.jwt_secret;

// ── 运行时动态配置 ──
cm.set_dynamic("rate_limit.requests_per_window", 200);
let limit: Option<i32> = cm.get_dynamic("rate_limit.requests_per_window");
cm.set_dynamic("feature.new_api", true);
let enabled: Option<bool> = cm.get_dynamic("feature.new_api");

// ── 读取自定义配置 ──
let custom_val = cm.get_dynamic::<String>("custom.my_key");

// ── 多环境 Profile ──
// ALUN_PROFILE=prod → 自动加载 config/config-prod.toml
// 加载顺序: config.toml → config-{profile}.toml → ALUN_* 环境变量

// ── 生成默认配置 ──
ConfigManager::generate_default("config")?;     // 生成 config/config.toml
```

***

## 2.6 `alun-cache` — 缓存系统

**路径**: `alun-cache/src/lib.rs`

### 关键 Trait

#### `Cache` （[lib.rs](file:///Volumes/zdh/projects/alun/alun/alun-cache/src/lib.rs)）

统一缓存接口（async trait）。

| 方法                             | 说明            |
| ------------------------------ | ------------- |
| `get<T>(key)`                  | 读取并反序列化       |
| `set(key, value)`              | 设置（永不过期）      |
| `set_ex(key, value, ttl_secs)` | 设置（指定过期秒数）    |
| `del(key)`                     | 删除            |
| `exists(key)`                  | 检查是否存在且未过期    |
| `incr(key, delta)`             | 计数器递增         |
| `keys(pattern)`                | Glob 模式匹配 key |
| `delete_pattern(pattern)`      | Glob 模式批量删除   |
| `stats()`                      | 统计信息          |

### 关键结构体

#### `LocalCache` — 本地内存缓存

- `HashMap<String, CacheEntry>` + `RwLock`（高并发读写）
- 支持 TTL 过期 + 后台清理任务，统计命中率/淘汰/过期清理

#### `RedisCache` — Redis 缓存

基于 `redis::aio::ConnectionManager` 的远程缓存实现。

#### `SharedCache` — 共享缓存枚举

枚举包装 `Local` / `Redis`，消除 `dyn Cache` 的对象安全问题。

#### `CacheStats` — 缓存统计

| 字段                 | 说明     |
| ------------------ | ------ |
| `hits`             | 命中次数   |
| `misses`           | 未命中次数  |
| `sets`             | 设置次数   |
| `deletes`          | 删除次数   |
| `evictions`        | 淘汰次数   |
| `expired_cleanups` | 过期清理次数 |

### 工厂函数

```rust
pub async fn create_cache(
    cache_config: &CacheConfig,
    redis_config: &RedisConfig,
) -> Result<SharedCache>
```

### 使用示例

```rust
use alun_cache::{create_cache, Cache, SharedCache};

// ── 创建缓存 ──
let cache: SharedCache = create_cache(&config.cache, &config.redis).await?;

// ── Handler 中通过全局函数获取缓存 ──
async fn handler() -> Res<String> {
    let c = cache();
    c.set_ex("key", "value", 3600).await.unwrap();
    let val: Option<String> = c.get("key").await.unwrap();
    Res::ok(val.unwrap_or_default())
}

// ── 基本操作 ──
cache.set("greeting", "hello").await?;
let val: Option<String> = cache.get("greeting").await?;
assert_eq!(val, Some("hello".into()));

// 带过期时间
cache.set_ex("session:abc", &session_data, 1800).await?;  // 30分钟
let data: Option<SessionData> = cache.get("session:abc").await?;

// 删除
cache.del("temp_key").await?;

// 检查存在
if cache.exists("greeting").await? {
    // ...
}

// ── 计数器 ──
let count: i64 = cache.incr("api_calls", 1).await?;       // 递增
let count: i64 = cache.incr("api_calls", -5).await?;      // 递减

// ── 模式匹配 ──
let keys: Vec<String> = cache.keys("user:*").await?;       // 模糊查找
let deleted: u64 = cache.delete_pattern("temp:*").await?;   // 批量删除

// ── 统计信息 ──
let stats = cache.stats().await?;
tracing::info!("缓存命中率: {:.2}%", stats.hit_rate() * 100.0);

// ── LocalCache 直接使用 ──
use alun_cache::LocalCache;
let local = LocalCache::new(10000, 3600);                   // 容量10000, 默认TTL 3600s
local.set("key", "value").await?;
```

***

## 2.7 `alun-template` — 模板引擎

**路径**: `alun-template/src/lib.rs`

封装 `minijinja`（Jinja2 语法），启动时一次性加载模板目录。

### `TemplateEngine`

| 方法                        | 说明                      |
| ------------------------- | ----------------------- |
| `new()`                   | 创建空引擎（仅支持 `render_str`） |
| `from_dir(dir)`           | 从目录加载模板                 |
| `render(name, ctx)`       | 渲染指定模板                  |
| `render_str(source, ctx)` | 从字符串渲染                  |

### 使用示例

```rust
use alun_template::TemplateEngine;

// ── 从目录加载 ──
let engine = TemplateEngine::from_dir("templates")?;

// ── 渲染模板文件 ──
use serde_json::json;
let html = engine.render("index.html", &json!({
    "title": "Alun",
    "users": [{"name": "张三"}, {"name": "李四"}],
}))?;

// ── 从字符串渲染 ──
let result = engine.render_str("Hello, {{ name }}!", &json!({"name": "World"}))?;

// ── 在 Handler 中使用（需 template feature） ──
async fn home() -> Result<Res<String>, ApiError> {
    let html = render_template("index.html", &json!({"title": "Home"}))
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(html))
}
```

**模板示例** (`templates/index.html`):

```html
<!DOCTYPE html>
<html>
<head><title>{{ title }}</title></head>
<body>
    <h1>{{ title }}</h1>
    <ul>
        {% for user in users %}
        <li>{{ user.name }}</li>
        {% endfor %}
    </ul>
</body>
</html>
```

***

## 2.8 `alun-utils` — 工具集

**路径**: `alun-utils/src/`

### 子模块

#### `str` — `StrExt` Trait

| 方法              | 说明                           |
| --------------- | ---------------------------- |
| `to_snake()`    | `helloWorld` → `hello_world` |
| `to_camel()`    | `hello_world` → `helloWorld` |
| `is_blank()`    | 是否为空字符串                      |
| `truncate(max)` | 按字符数截断，超出末尾补 `...`           |
| `random(len)`   | 生成指定长度的随机字母数字串               |
| `has_text()`    | 非空白字符（`is_blank` 取反）         |

#### `str` — 自由函数

| 函数                            | 说明                               | 示例                                           |
| ----------------------------- | -------------------------------- | -------------------------------------------- |
| `sanitize_filename(s)`        | 将非字母/数字/点/横线/下划线的字符替换为 `_`       | `file<name>.txt` → `file_name.txt`           |
| `parse_json_value(s)`         | 解析 JSON 字符串为 `serde_json::Value` | `"{\"k\":1}"` → `Ok(Value)`                  |
| `format_file_size(bytes)`     | 字节数 → 人类可读（B/KB/MB/GB/TB/PB）     | `1500000` → `"1.43 MB"`                      |
| `clean_string_param(s)`       | 去除前后空格                           | `"  hello  "` → `"hello"`                    |
| `clean_email(email)`          | 去除前后空格 + 转小写                     | `" A@B.COM "` → `"a@b.com"`                  |
| `clean_password(pwd)`         | 只去除前后空格，保留中间空格                   | `" pass 123 "` → `"pass 123"`                |
| `generate_invite_code()`      | 生成 12 位随机邀请码                     | `"a1B2c3D4e5F6"`                             |
| `generate_random_digits(n)`   | 生成指定位数的随机数字串（不含 `0`）             | `generate_random_digits(6)` → `"573192"`     |
| `generate_random_alphanum(n)` | 生成无易混淆字符（`0`/`O`/`I`/`l`）的随机串    | `generate_random_alphanum(8)` → `"aB3kM9xQ"` |

#### `str` — `InputCleaner` 输入清理器

| 方法                                           | 说明                                                          |
| -------------------------------------------- | ----------------------------------------------------------- |
| `clean_register_input(email, pwd, nickname)` | 清理注册三项：邮箱小写去空格、密码去空格、昵称去空格，返回 `(email, password, nickname)` |
| `clean_login_input(email, pwd)`              | 清理登录两项：邮箱小写去空格、密码去空格，返回 `(email, password)`                 |

#### `date` — `Date` 工具

| 方法                   | 说明             |
| -------------------- | -------------- |
| `now()`              | 当前 UTC 时间      |
| `now_local()`        | 当前本地时间         |
| `fmt(dt, fmt)`       | 格式化日期          |
| `from_timestamp(ts)` | 时间戳 → DateTime |
| `relative(ts)`       | 相对时间（"3分钟前"）   |
| `begin_of_day(dt)`   | 当天 00:00:00    |
| `end_of_day(dt)`     | 当天 23:59:59    |

#### `mask` — `Mask` 脱敏

| 方法                          | 示例输入                   | 输出                   | 说明               |
| --------------------------- | ---------------------- | -------------------- | ---------------- |
| `mobile(s)`                 | `13812345678`          | `138****5678`        | 手机号：保留前3后4位       |
| `email(s)`                  | `a@b.com`              | `a***@b.com`         | 邮箱：保留首字符和域名       |
| `id_card(s)`                | `320112199001011234`   | `3201****1234`       | 身份证：保留前4后4位       |
| `name(s)`                   | `张三丰`                  | `张**`                | 姓名：保留首字符，其余用 `*`  |
| `bank_card(s)`              | `6222021234567890`     | `6222 **** 7890`     | 银行卡：保留前4后4位       |
| `user_id(s)`                | `user_abc123`          | `us****23`           | 用户ID：保留前2后2字符     |
| `password(s)`               | `secret`               | `******`             | 密码：固定返回 `******`  |
| `address(s)`                | `北京市海淀区中关村`           | `北京市海淀****`          | 地址：保留前6字符         |
| `license_plate(s)`          | `京A12345`              | `京****5`             | 车牌：保留首字符和末位       |
| `mask_by_type(type, value)` | `("mobile", "1381...")` | `"138****5678"`      | 按数据类型自动选择脱敏方式     |

**`mask_json_value`** **函数**：递归对 JSON 对象脱敏，自动检测手机号/身份证/邮箱格式。

| 参数                          | 说明                           |
| --------------------------- | ---------------------------- |
| `value: Value`              | 待脱敏的 JSON                    |
| `sensitive_fields: &[&str]` | 敏感字段名列表（匹配到的字段值替换为 `"****"`） |

```rust
use alun_utils::mask::mask_json_value;
use serde_json::json;

let input = json!({"password": "secret123", "mobile": "13812345678", "name": "张三"});
let masked = mask_json_value(input, &["password", "mobile"]);

// masked → {"password": "****", "mobile": "****", "name": "张三"}
// 字段名匹配 → 值替换为 "****"；其他字段值若匹配手机号/身份证/邮箱格式也会自动脱敏
```

#### `sid` — `Sid` 短 ID

| 方法        | 说明            |
| --------- | ------------- |
| `short()` | 16 位 hex      |
| `tiny()`  | 8 位 hex       |
| `tsid()`  | 时间戳 + 随机数     |
| `uuid()`  | UUID v4       |
| `uuid7()` | UUID v7（时间有序） |

#### `ua` — User-Agent 解析

从 User-Agent 字符串中提取设备类型、浏览器类型和操作系统信息。

**`UaInfo`** — 解析结果结构体：

| 字段             | 类型       | 说明                               |
| -------------- | -------- | -------------------------------- |
| `device_type`  | `String` | 设备类型：`"PC"` / `"MOBILE"` / `"TABLET"` / `"UNKNOWN"` |
| `browser_type` | `String` | 浏览器：`"Chrome"` / `"Firefox"` / `"Safari"` / `"Edge"` / `"Unknown"` |
| `os_type`      | `String` | 操作系统：`"Windows"` / `"macOS"` / `"Linux"` / `"iOS"` / `"Android"` / `"Unknown"` |

**`parse_user_agent(ua)`** — 解析 User-Agent 字符串，返回 `UaInfo`。

```rust
use alun_utils::parse_user_agent;

let info = parse_user_agent(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0 Safari/537.36"
);
assert_eq!(info.device_type, "PC");
assert_eq!(info.browser_type, "Chrome");
assert_eq!(info.os_type, "Windows");
```

#### `valid` — `Valid` 验证

| 方法                             | 说明                          |
| ------------------------------ | --------------------------- |
| `is_email(s)`                  | 邮箱格式                        |
| `is_mobile(s)`                 | 手机号（中国大陆）                   |
| `is_phone(s)`                  | 电话号码（E.164 格式）              |
| `is_url(s)`                    | URL 格式                      |
| `is_ipv4(s)`                   | IPv4 格式                     |
| `is_strong_password(s)`        | 强密码（≥8位，含大小写+数字+特殊字符）       |
| `is_username(s)`               | 用户名（3\~50位字母/数字/下划线/点/横线）   |
| `is_color(s)`                  | 十六进制颜色（#RRGGBB）             |
| `is_uuid(s)`                   | UUID（v1\~v7 均支持）            |
| `is_id_card(s)`                | 中国居民身份证号（含校验位）              |
| `is_date(s)`                   | 日期格式（YYYY-MM-DD）            |
| `is_datetime(s)`               | 日期时间（ISO 8601 / RFC 3339）   |
| `is_json(s)`                   | 合法 JSON 字符串                 |
| `is_base64(s)`                 | Base64 编码                   |
| `is_digits(s)`                 | 纯数字                         |
| `is_alphanumeric(s)`           | 字母+数字组合                     |
| `len_between(s, min, max)`     | 字符串长度范围                     |
| `has_html(s)`                  | 检测是否包含 HTML 标签              |
| `is_html_free(s)`              | 检测是否不包含 HTML 标签             |
| `is_file_extension(name, ext)` | 验证文件扩展名是否在允许列表中             |
| `format_validation_errors(e)`  | 将 `ValidationErrors` 转为可读消息 |

> **`format_validation_errors`** 需要启用 `validator-integration` feature，用于将 `validator::ValidationErrors` 转换为 `"字段名: 错误描述"` 格式的可读字符串。

#### `crypto` — `Crypto` 加密

| 方法                                | 说明               |
| --------------------------------- | ---------------- |
| `sha256(data)`                    | SHA-256 哈希       |
| `hmac_sha256(key, data)`          | HMAC-SHA256      |
| `aes_encrypt(key, plaintext)`     | AES-256-GCM 加密   |
| `aes_decrypt(key, cipher, nonce)` | AES-256-GCM 解密   |
| `hash_password(pwd)`              | Argon2 密码哈希          |
| `verify_password(pwd, hash)`      | 验证密码（自动检测 Argon2/BCrypt 算法） |
| `base64_url_encode(data)`         | URL 安全 Base64    |
| `base64_url_decode(s)`            | URL 安全 Base64 解码 |
| `random_key()`                    | 32 字节随机密钥        |
| `random_token(len)`               | 随机 hex Token     |

#### `web` — `WebExt` Web 工具

- `domain(url)` — 提取域名
- `query_to_map(query)` — Query String 解析

#### `export` — `Export` / `Import` 导出

- `to_csv(columns, records)` — 导出 CSV
- `to_json(records)` — 导出 JSON

#### `xss` — HTML/XSS 净化（需启用 `xss` feature）

基于 `ammonia` crate 的 HTML 净化工具。**按需启用**，仅当后端接收/返回用户富文本时需要。

| 函数                           | 说明                             |
| ---------------------------- | ------------------------------ |
| `sanitize_html(html)`        | 使用默认规则净化 HTML（保留安全标签如 p/a/img） |
| `sanitize_html_strict(html)` | 严格净化（strip 所有标签，仅保留纯文本）        |
| `has_potential_xss(html)`    | 检测 HTML 是否包含潜在 XSS 载荷          |

```toml
# Cargo.toml
alun = { version = "0.1", features = ["xss"] }
```

### 使用示例

```rust
use alun_utils::*;

// ── 字符串转换 ──
"helloWorld".to_snake();           // → "hello_world"
"hello_world".to_camel();           // → "helloWorld"
"".is_blank();                      // → true
"  ".is_blank();                    // → true

// ── 字符串清理 ──
sanitize_filename("file<name>.txt");       // → "file_name.txt"
clean_email("  User@Mail.COM  ");          // → "user@mail.com"
clean_string_param("  hello  ");           // → "hello"
clean_password("  pass 123  ");            // → "pass 123"

// ── 输入清理器 ──
let (email, pwd, nick) = InputCleaner::clean_register_input(" A@B.com ", " 123 ", " Tom ");
// → ("a@b.com", "123", "Tom")
let (email, pwd) = InputCleaner::clean_login_input(" A@B.com ", " 123 ");

// ── 格式化 ──
format_file_size(0);                       // → "0 B"
format_file_size(1_500_000);               // → "1.43 MB"
parse_json_value(r#"{"key": 1}"#);         // → Ok(Value({"key": Number(1)}))

// ── 随机生成 ──
generate_invite_code();                    // → 12位随机邀请码（字母数字）
generate_random_digits(6);                 // → 6位数字（如 "573192"，不含0）
generate_random_alphanum(8);               // → 8位无混淆字符（如 "aB3kM9xQ"）

// ── 日期操作 ──
let now = Date::now();                                           // UTC
let local = Date::now_local();                                   // 本地时间
Date::fmt(&now, "%Y-%m-%d %H:%M:%S");                           // → "2026-05-06 08:16:22"
Date::relative(now.timestamp());                                 // → "3分钟前"
Date::begin_of_day(&now);                                        // → 当天 00:00:00
Date::from_timestamp(1700000000);                                // 时间戳 → DateTime

// ── 脱敏 ──
Mask::mobile("13812345678");                                     // → "138****5678"
Mask::email("alice@company.com");                                // → "a***@company.com"
Mask::id_card("320112199001011234");                             // → "3201****1234"
Mask::name("张三丰");                                              // → "张**"
Mask::bank_card("6222021234567890");                             // → "6222 **** 7890"
Mask::user_id("user_abc123");                                    // → "us****23"
Mask::password("secret");                                        // → "******"
Mask::address("北京市海淀区中关村");                                   // → "北京市海淀****"
Mask::license_plate("京A12345");                                  // → "京****5"
Mask::mask_by_type("mobile", "13812345678");                    // → "138****5678"（按类型自动选择）

// ── JSON 脱敏 ──
use alun_utils::mask::mask_json_value;
use serde_json::json;
let input = json!({"password": "secret", "mobile": "13812345678"});
let masked = mask_json_value(input, &["password"]);

// ── ID 生成 ──
Sid::short();                     // → "a1b2c3d4e5f6a7b8"（16位hex）
Sid::tiny();                      // → "a1b2c3d4"（8位hex）
Sid::tsid();                      // 时间戳+随机数混合
Sid::uuid();                      // → UUID v4
Sid::uuid7();                     // → UUID v7（时间有序，适合主键）

// ── User-Agent 解析 ──
let info = parse_user_agent(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36"
);
info.device_type;                             // → "PC"
info.browser_type;                            // → "Chrome"
info.os_type;                                 // → "Windows"

// ── 验证 ──
Valid::is_email("a@b.com");                          // → true
Valid::is_mobile("13812345678");                     // → true
Valid::is_phone("+8613812345678");                   // → true
Valid::is_url("https://example.com");                // → true
Valid::is_ipv4("192.168.1.1");                       // → true
Valid::is_strong_password("Abc@12345");              // → true
Valid::is_username("john_doe");                      // → true
Valid::is_color("#FF00AA");                          // → true
Valid::is_uuid("550e8400-e29b-41d4-a716-446655440000"); // → true
Valid::is_id_card("110101199003077758");             // → true（含校验位）
Valid::is_date("2024-01-01");                        // → true
Valid::is_datetime("2024-01-01T00:00:00Z");          // → true
Valid::is_json(r#"{"key": 1}"#);                     // → true
Valid::is_base64("SGVsbG8=");                        // → true
Valid::is_digits("123456");                          // → true
Valid::is_alphanumeric("abc123");                    // → true
Valid::len_between("hello", 2, 10);                  // → true
Valid::has_html("<div>hello</div>");                 // → true
Valid::is_html_free("plain text");                   // → true
Valid::is_file_extension("photo.jpg", &["jpg", "png"]); // → true

// ── 加密 ──
Crypto::sha256("alun");                              // → SHA256 hex
Crypto::random_key();                                // → 32字节随机密钥
Crypto::random_token(32);                            // → 随机hex token
Crypto::hash_password("pass123");                    // → Argon2 哈希
Crypto::verify_password("pass123", &hash)?;          // → 验证密码（自动检测 Argon2/BCrypt）

let key = Crypto::random_key();
let encrypted = Crypto::aes_encrypt("secret data", &hex::encode(&key))?;
let decrypted = Crypto::aes_decrypt(&encrypted, &hex::encode(&key))?;

// ── Web 工具 ──
WebExt::domain("https://a.com/path")?;               // → "a.com"
WebExt::query_to_map("?a=1&b=2");                    // → {"a":"1","b":"2"}

// ── CSV/JSON 导出 ──
use std::collections::HashMap;
let records = vec![
    HashMap::from([("name","张三"), ("age","28")]),
    HashMap::from([("name","李四"), ("age","32")]),
];
let csv = Export::to_csv(&["name", "age"], &records)?;
let json = Export::to_json(&records)?;

// ── XSS 净化（需 xss feature） ──
use alun_utils::xss;
let safe = xss::sanitize_html("<script>alert(1)</script><p>Hello</p>");
assert_eq!(safe, "<p>Hello</p>");

let strict = xss::sanitize_html_strict("<p>Hello</p>");
assert_eq!(strict, "Hello");                   // 仅保留纯文本

let malicious = xss::has_potential_xss("<script>alert(1)</script>");
assert!(malicious);

// ── 在 Handler 中使用 ──
#[alun::get("/api/demo/utils")]
async fn utils_demo(Query(params): Query<HashMap<String, String>>) -> Res<serde_json::Value> {
    let input = params.get("input").cloned().unwrap_or_default();
    Res::ok(json!({
        "to_snake": input.to_snake(),
        "to_camel": input.to_camel(),
        "is_email": Valid::is_email(&input),
        "masked": Mask::mobile(&input),
        "timestamp": Sid::tsid(),
        "size_formatted": format_file_size(1_500_000),
        "invite_code": generate_invite_code(),
    }))
}
```

***

## 2.9 `alun-macros` — Proc Macro

**路径**: `alun-macros/src/`

### `route.rs` — 路由宏

| 宏                                               | 行为                                                                             |
| ----------------------------------------------- | ------------------------------------------------------------------------------ |
| `#[alun::get("/path")]`                         | 为 async fn 生成 `linkme::distributed_slice(ROUTES)` 注册项，调用 `AlunRouter::add_get` |
| `#[alun::post("/path")]`                        | 同上，`add_post`                                                                  |
| `#[alun::put("/path")]`                         | 同上，`add_put`                                                                   |
| `#[alun::delete("/path")]`                      | 同上，`add_delete`                                                                |
| `#[alun::controller("/base")]`                  | 扫描 impl 块中标注了 `#[get]`/`#[post]` 的方法，自动生成路由注册                                  |
| `#[alun::permission(path, method, permission)]` | 生成 `PERMISSION_ROUTES` 切片注册项                                                   |
| `#[alun::no_auth("/path")]`                     | 生成 `NO_AUTH_ROUTES` 切片注册项，标记无需认证的路径                                            |

### `plugin.rs` — 插件标记宏

| 宏                 | 行为                                      |
| ----------------- | --------------------------------------- |
| `#[alun::plugin]` | 为结构体添加 `alun_plugin_name()` 静态方法，返回结构体名 |

### `task_handler.rs` — 任务处理器宏（需 `task` feature）

| 宏                                           | 行为                                                                   |
| ------------------------------------------- | -------------------------------------------------------------------- |
| `#[alun::task_handler(task_type = N, ...)]` | 生成 `linkme::distributed_slice(TASK_HANDLERS)` 注册项，编译期自动发现并注册 handler |

***

## 2.10 `alun-plugin` — 内置插件

**路径**: `alun-plugin/src/`

### `CachePlugin` （[cache\_plugin.rs](file:///Volumes/zdh/projects/alun/alun/alun-plugin/src/cache_plugin.rs)）

缓存生命周期管理器。`start()` 创建 LocalCache/RedisCache，`stop()` 释放。通过 `plugin.cache()` 获取共享缓存实例。

### `NotificationPlugin` （[notification.rs](file:///Volumes/zdh/projects/alun/alun/alun-plugin/src/notification.rs)）

SMTP 邮件通知（基于 lettre），支持纯文本和 HTML 格式。根据发件域名自动选择传输方式——`@icloud.com` / `@swisscows.email` 走 `STARTTLS`，其余走 `RELAY`。

- `send_text(to, subject, body)` — 发送纯文本邮件
- `send_html(to, subject, html_body)` — 发送 HTML 邮件（内部自动生成纯文本备用版本）
- `is_configured()` — 检查邮件功能是否已配置

### `AsyncTaskPlugin` （[async\_task.rs](file:///Volumes/zdh/projects/alun/alun/alun-plugin/src/async_task.rs)）

基于 Semaphore 的后台任务队列。`submit(task)` 提交异步任务，`stop()` 等待所有任务完成。

### `SchedulerPlugin` （[scheduler.rs](file:///Volumes/zdh/projects/alun/alun/alun-plugin/src/scheduler.rs)）

定时任务注册中心。`register(name, cron, desc, runner)` 注册任务，`trigger(name)` 手动触发，`list()` 列出所有。

### `create_plugins_from_config(config)` （[lib.rs](file:///Volumes/zdh/projects/alun/alun/alun-plugin/src/lib.rs)）

工厂函数——根据 `AppConfig.plugins.enabled` 列表创建所有启用的插件并返回 `PluginManager`。

### 使用示例

```rust
use alun_plugin::*;
use alun_plugin::scheduler::SchedulerPlugin;

// ── 配置驱动加载 ──
// config.toml: [plugins]\nenabled = ["cache", "notification", "async-task", "scheduler"]
let plugin_manager = create_plugins_from_config(&config)?;

// ── 缓存插件（自动生命周期） ──
// App 启动时 CachePlugin 自动创建 SharedCache，注入全局资源
// Handler 中直接使用 cache() 获取

// ── 定时任务 ──
let scheduler = SchedulerPlugin::new(4);

scheduler.register(
    "clean_temp_files",                     // 任务名
    "0 */2 * * *",                          // cron: 每2小时
    "清理临时文件",                           // 描述
    || Box::pin(async {
        clean_temp_files().await;
        Ok(())
    }),
);

scheduler.register(
    "daily_report",
    "0 8 * * *",                            // 每天早上8点
    "日报生成",
    || Box::pin(async {
        generate_daily_report().await;
        Ok(())
    }),
);

let jobs: Vec<(String, String)> = scheduler.list();      // 列出所有任务
scheduler.trigger("clean_temp_files").await?;             // 手动触发

// ── 邮件通知 ──
use alun_plugin::notification::NotificationPlugin;
let notif = NotificationPlugin::from_config(&config.notification);
notif.send_text("admin@example.com", "告警", "磁盘使用率 > 90%").await?;
notif.send_html("user@example.com", "验证码", "<h1>您的验证码</h1><p><b>123456</b></p>").await?;

// ── 异步任务队列 ──
use alun_plugin::async_task::AsyncTaskPlugin;
let task_pool = AsyncTaskPlugin::new(4);                   // 4 个 worker
task_pool.submit(async { heavy_computation().await; });
task_pool.submit(async { send_batch_emails().await; });
// stop() 时等待所有任务完成

// ── 自定义插件注册 ──
#[alun::plugin]
struct MetricsPlugin;
#[async_trait]
impl Plugin for MetricsPlugin {
    fn name(&self) -> &str { "metrics" }
    async fn start(&self) -> Result<()> {
        tracing::info!("指标收集已启动");
        Ok(())
    }
    async fn stop(&self) -> Result<()> { Ok(()) }
}

App::new()?.plugin(MetricsPlugin).serve("8080").await?;
```

***

## 2.11 `alun-log` — 日志初始化

**路径**: `alun-log/src/lib.rs`

`alun_log::init(config)` — 根据 `LogConfig` 配置 tracing：

- `format = "text"`：彩色文本
- `format = "json"`：JSON 结构化日志（对接 ELK/Loki）
- `dir` 非空：日滚文件输出
- 优先从 `RUST_LOG` 环境变量读取过滤级别

### 使用示例

```rust
// ── 自动初始化（App 启动时自动调用） ──
// App::new() 内部调用 alun_log::init(&config.log)
// 无需手动初始化

// ── 手动初始化 ──
use alun_log;
alun_log::init(&config.log)?;

// ── 在代码中使用 tracing ──
use tracing::{info, warn, error, debug, trace};

trace!("进入函数: process_order({})", order_id);
debug!("数据库查询耗时: {}ms", duration_ms);
info!("用户登录成功: user_id={}", user_id);
warn!("缓存即将过期: key={}", cache_key);
error!("支付回调异常: order_id={}, 原因={}", order_id, reason);

// ── 结构化字段 ──
info!(method = "POST", path = "/api/order", status = 200, duration_ms = 5, "请求完成");

// ── Span（分布式追踪上下文） ──
use tracing::instrument;
#[instrument(skip(db))]
async fn create_order(db: &Db, req: CreateOrderReq) -> Result<Order, Error> {
    info!("开始创建订单");
    // ... span 自动记录函数名、参数、耗时
    Ok(order)
}

// ── 配置文件示例 ──
// [log]
// level = "debug"         # 开发环境
// format = "json"         # 生产环境（对接 ELK/Loki）
// dir = "logs"            # 同时输出到文件
// file_prefix = "alun"
```

***

## 2.12 `alun-kafka` / `alun-fs` — 扩展 Crate

### `alun-kafka`

| 导出项             | 说明                    |
| --------------- | --------------------- |
| `KafkaProducer` | Kafka 生产者（基于 rdkafka） |
| `KafkaConsumer` | Kafka 消费者             |
| `KafkaPlugin`   | Kafka 插件（管理连接生命周期）    |

#### 使用示例

```rust
// ── Kafka 生产者 ──
use alun_kafka::{KafkaProducer, KafkaPlugin};

let producer = KafkaProducer::new("localhost:9092")?;
producer.send("order-events", "order.created", &order_json).await?;

// Kafka 插件自动管理连接生命周期
let kafka_plugin = KafkaPlugin::from_config(&config);

// ── Kafka 消费者 ──
use alun_kafka::KafkaConsumer;
let consumer = KafkaConsumer::new("localhost:9092", "order-group", &["order-events"]);
consumer.start(|msg| async move {
    tracing::info!("收到消息: {:?}", msg);
    Ok(())
}).await?;
```

### `alun-fs`

**路径**: `alun-fs/src/`

多后端文件存储框架，提供统一的 `StorageBackend` trait 抽象，支持本地文件系统、MinIO、AWS S3 等存储后端。核心设计理念：**Trait 定义契约 + Registry 管理实例 + Plugin 组装门面** —— 业务层零感知存储介质差异，切换后端仅需改配置。

#### 目录结构

| 模块 | 职责 |
| ---- | ---- |
| `backend` | `StorageBackend` trait（统一存储接口：write / read / delete / exists / presign_url / health_check） |
| `registry` | `BackendRegistry`（按 backend_type 管理多后端实例，支持运行时注册 + linkme 编译期自动发现） |
| `types` | `BackendConfig`（每种后端的配置）、`FsPluginConfig`（全局运行时配置）、`BackendEntry` / `STORAGE_BACKENDS`（linkme 分布式切片） |
| `local` | `LocalFs`（本地文件系统实现，按日期分目录 YYYY/MM/DD/uuid.ext，自动 MIME 推断） |
| `minio` | `MinioBackend`（MinIO / S3 兼容实现，条件编译 `feature = "minio"`，支持预签名 URL） |
| `plugin` | `FsPlugin`（多后端门面，实现 `alun_core::Plugin`，统一生命周期管理） |

#### 公共 API

| 导出项 | 说明 |
| ------ | ---- |
| `StorageBackend` | 存储后端统一行为 trait |
| `BackendRegistry` | 后端注册中心（register / from_discovered / get / default_backend / health_check_all） |
| `BackendConfig` | 单个后端配置（endpoint / region / access_key / secret_key / bucket） |
| `FsPluginConfig` | 全局运行时配置（default_backend_type / local_root_dir / max_file_size_bytes / presign_url_ttl_secs） |
| `LocalFs` | 本地文件系统后端 |
| `MinioBackend` | MinIO / S3 兼容后端（需启用 `minio` feature） |
| `FsPlugin` | 多后端文件存储插件门面 |
| `FileMeta` | 文件元信息结构体（file_id / original_name / stored_path / size / content_type / created_at） |
| `StoreResult<T>` | 存储操作结果类型（`Result<T, String>`） |

#### 使用示例

```rust
// ── 基本用法：local 后端（向后兼容） ──
use alun_fs::{FsPlugin, LocalFs, FileMeta};

// 一行创建本地存储插件
let plugin = FsPlugin::new_local("uploads");

// 写入文件（自动按 YYYY/MM/DD/uuid.ext 存储）
let meta = plugin.write("report.pdf", &file_data).await?;
// meta.stored_path → "2026/05/19/a1b2c3d4.pdf"
// meta.file_id → UUID v4

// 读取文件
let data = plugin.read(&meta.stored_path).await?;

// 删除文件（幂等）
plugin.delete(&meta.stored_path).await?;

// ── 多后端：注册中心 + trait ──
use alun_fs::{BackendRegistry, BackendConfig, FsPluginConfig, StorageBackend};
use std::sync::Arc;

let config = FsPluginConfig::default();

let mut registry = BackendRegistry::new();
// 注册 local 后端
registry.register(
    LocalFs::new("uploads"),
    BackendConfig { backend_type: "local".into(), root_path: "uploads".into(), ..Default::default() },
);
// 从 linkme 分布式切片自动发现编译期声明的后端（如 #[storage_backend] 宏）
registry.from_discovered().with_default("local");

let plugin = FsPlugin::new(config, registry);

// 按 backend_type 写入到指定后端
let meta = plugin.write_to(Some("local"), "avatar.png", &img_data).await?;

// ── 自定义后端：impl StorageBackend trait ──
#[alun_fs::storage_backend(backend_type = "my-oss")]
struct MyOssBackend { /* ... */ }

#[async_trait]
impl StorageBackend for MyOssBackend {
    fn backend_type(&self) -> &str { "my-oss" }
    async fn write(&self, name: &str, data: &[u8]) -> Result<FileMeta, String> { /* ... */ }
    async fn read(&self, path: &str) -> Result<Vec<u8>, String> { /* ... */ }
    async fn delete(&self, path: &str) -> Result<(), String> { /* ... */ }
    async fn exists(&self, path: &str) -> bool { /* ... */ }
    async fn presign_download_url(&self, path: &str, ttl: Option<u64>) -> Result<String, String> { /* ... */ }
}
// 编译后自动收集到 STORAGE_BACKENDS 切片，from_discovered() 一键注册

// ── 配置示例（config.toml） ──
// [fs]
// default_backend_type = "local"
// local_root_dir = "uploads"
// max_file_size_bytes = 52428800
// presign_url_ttl_secs = 3600

// MinIO 后端（feature = "minio"）
use alun_fs::MinioBackend;
let minio = MinioBackend::from_config(&BackendConfig {
    backend_type: "minio".into(),
    endpoint: "http://localhost:9000".into(),
    region: "us-east-1".into(),
    access_key: "minioadmin".into(),
    secret_key: "minioadmin".into(),
    root_path: "my-bucket".into(),
    ..Default::default()
}).await?;

// 注册到 registry
registry.register(minio, minio_config).with_default("minio");
```

***

## 2.13 `alun-task` — 异步任务框架

**路径**: `alun-task/src/`

基于 Kafka 消息队列的异步任务分发与处理框架。核心设计理念：**插件零 SQL 依赖**——所有持久化通过 `TaskStorage` trait 委托给业务方，不与任何表结构耦合。

**结构概览**：

| 模块         | 职责                                                                                |
| ---------- | --------------------------------------------------------------------------------- |
| `storage`  | `TaskStorage` trait（泛型持久化接口）+ `RetryableTask`（重试数据结构）                             |
| `handler`  | `TaskHandler` trait（业务处理器接口）                                                      |
| `registry` | `HandlerRegistry`（按 task\_type 索引 handler）+ `from_discovered()` 宏自动发现             |
| `producer` | `TaskProducer`（Kafka 消息发送 + storage 持久化）                                          |
| `worker`   | `TaskWorker`（Kafka 消费循环 + handler 分发执行 + DLQ）                                     |
| `retry`    | `RetryScanner`（定期扫描待重试任务 + 重推 Kafka）                                              |
| `metrics`  | `TaskMetrics`（原子计数：total / completed / failed）                                    |
| `plugin`   | `TaskPlugin`（实现 `alun_core::Plugin`，统一启停）                                         |
| `types`    | 枚举和配置：`TaskStatus`、`TaskPriority`、`RetryStrategy`、`TaskConfig`、`TaskWorkerConfig` |

### 关键 Trait

#### `TaskStorage` （[storage.rs](file:///Volumes/zdh/projects/alun/alun/alun-task/src/storage.rs)）

**插件不持有 SQL 语句、表名、字段名**。所有持久化逻辑由业务方通过实现此 trait 控制。共 8 个方法：

| 方法                                                            | 调用时机              | 说明                                             |
| ------------------------------------------------------------- | ----------------- | ---------------------------------------------- |
| `save_task_log(task_id, task_type, priority, config, params)` | 提交任务              | 持久化任务日志（完整入参）                                  |
| `save_task_queue(task_id, topic, priority)`                   | 提交任务              | 持久化队列记录                                        |
| `update_task_status(task_id, status)`                         | 状态变更              | Pending→Processing→Completed/Failed/DeadLetter |
| `get_retry_count(task_id)`                                    | 失败时               | 获取当前重试次数                                       |
| `update_retry(task_id, retry_count)`                          | 重试时               | 递增 retry\_count                                |
| `save_task_result(task_id, output)`                           | 执行完成              | 保存成功/失败输出                                      |
| `log_execution(task_id, status, error, elapsed_ms)`           | 每次执行              | 记录执行日志（含重试）                                    |
| `scan_retryable_tasks(task_types, limit)`                     | RetryScanner 定期调用 | 查询可重试任务列表                                      |

所有方法返回 `Result<(), String>`，失败原因通过 `Err(String)` 传递。业务方可自由选择后端（PG / MySQL / MongoDB / 文件等）。

```rust
#[async_trait]
pub trait TaskStorage: Send + Sync {
    async fn save_task_log(&self, task_id: &str, task_type: i16, priority: i16,
        config: &TaskConfig, params: &SubmitTaskParams) -> Result<(), String>;
    async fn save_task_queue(&self, task_id: &str, topic: &str, priority: i16) -> Result<(), String>;
    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<(), String>;
    async fn get_retry_count(&self, task_id: &str) -> Result<i64, String>;
    async fn update_retry(&self, task_id: &str, retry_count: i64) -> Result<(), String>;
    async fn save_task_result(&self, task_id: &str, output: &serde_json::Value) -> Result<(), String>;
    async fn log_execution(&self, task_id: &str, status: TaskStatus, error: Option<&str>, elapsed_ms: i64) -> Result<(), String>;
    async fn scan_retryable_tasks(&self, task_types: &[i16], limit: usize) -> Result<Vec<RetryableTask>, String>;
}
```

#### `TaskHandler` （[handler.rs](file:///Volumes/zdh/projects/alun/alun/alun-task/src/handler.rs)）

业务处理器特质——每种任务类型对应一个实现。

```rust
#[async_trait]
pub trait TaskHandler: Send + Sync {
    fn task_type(&self) -> i16;
    async fn execute(&self, payload: Value) -> Result<Value, String>;
}
```

### 关键结构体

#### `RetryableTask`

扫描到的可重试任务——由 `TaskStorage::scan_retryable_tasks()` 返回。

| 字段            | 类型       | 说明      |
| ------------- | -------- | ------- |
| `task_id`     | `String` | 任务 ID   |
| `task_type`   | `i16`    | 任务类型    |
| `retry_count` | `i64`    | 当前重试次数  |
| `max_retries` | `i64`    | 最大重试次数  |
| `payload`     | `Value`  | 任务携带的数据 |

#### `HandlerRegistry` （[registry.rs](file:///Volumes/zdh/projects/alun/alun/alun-task/src/registry.rs)）

处理器注册中心——按 `task_type` 索引 handler 和 config。

| 方法                          | 说明                                               |
| --------------------------- | ------------------------------------------------ |
| `new()`                     | 创建空注册中心                                          |
| `register(handler, config)` | 手动注册一个 handler                                   |
| `from_discovered()`         | **批量注册编译期发现的 handler**（读取 `TASK_HANDLERS` 分布式切片） |
| `get(task_type)`            | 按 task\_type 获取 handler 和 config                 |
| `get_config(task_type)`     | 按 task\_type 获取配置                                |
| `task_types()`              | 获取所有已注册的 task\_type                              |
| `len()` / `is_empty()`      | 统计信息                                             |

#### `TaskProducer` （[producer.rs](file:///Volumes/zdh/projects/alun/alun/alun-task/src/producer.rs)）

任务生产者——提交任务到 Kafka 并委托 storage 持久化。

| 方法                                | 说明                                 |
| --------------------------------- | ---------------------------------- |
| `new(brokers, storage, registry)` | 创建生产者，返回 `Result`                  |
| `submit(params)`                  | 提交单个任务（持久化 + Kafka 发送），返回 task\_id |
| `submit_batch(params)`            | 批量提交，返回 `(成功数, 失败列表)`              |
| `send_to_dlq(msg, topic, reason)` | 发送消息到死信队列                          |

#### `TaskWorker` （[worker.rs](file:///Volumes/zdh/projects/alun/alun/alun-task/src/worker.rs)）

任务消费者——Kafka 消费循环 + handler 分发执行。

| 方法                                        | 说明                    |
| ----------------------------------------- | --------------------- |
| `new(config, storage, registry, metrics)` | 创建 worker，返回 `Result` |
| `run(topics)`                             | 订阅 topic 并启动消费循环      |
| `stop()`                                  | 发送停止信号                |

内部逻辑：

1. 消费 Kafka 消息 → 反序列化 `TaskMessage`
2. `check_message_age()` 过滤超时消息
3. 查找 `HandlerRegistry` 找到对应 handler
4. `std::time::timeout` 限时执行
5. 成功 → 更新 status + 保存 result + 记执行日志
6. 失败 → 检查重试次数：
   - 未超限 → `update_retry()`（等待 RetryScanner 重推）
   - 超限 → 检查 `dead_letter_topic`：
     - 有 DLQ → 推入死信队列 + 状态 DeadLetter
     - 无 DLQ → 状态 Failed

#### `RetryScanner` （[retry.rs](file:///Volumes/zdh/projects/alun/alun/alun-task/src/retry.rs)）

后台定期扫描待重试任务。

| 方法                                                               | 说明                |
| ---------------------------------------------------------------- | ----------------- |
| `new(brokers, storage, registry, interval_secs, max_batch_size)` | 创建扫描器，返回 `Result` |
| `run()`                                                          | 启动扫描循环            |
| `stop()`                                                         | 发送停止信号            |

#### `TaskPlugin` （[plugin.rs](file:///Volumes/zdh/projects/alun/alun/alun-task/src/plugin.rs)）

任务插件入口——实现 `alun_core::Plugin` trait。

| 方法                               | 说明                                        |
| -------------------------------- | ----------------------------------------- |
| `new(config, storage, registry)` | 创建插件（配置从 `[task]` section 读取），返回 `Result` |
| `name()`                         | 返回 `"task"`                               |
| `start()`                        | 在后台 tokio task 中启动 Worker + RetryScanner  |
| `stop()`                         | AtomicBool 信号停止 → join 所有后台任务             |
| `depends_on()`                   | 无依赖                                       |
| `metrics()`                      | 返回 `Arc<TaskMetrics>`                     |
| `topics()`                       | 返回已注册的 topic 列表                           |

#### `TaskMetrics` （[metrics.rs](file:///Volumes/zdh/projects/alun/alun/alun-task/src/metrics.rs)）

原子计数器。

| 字段          | 类型          | 说明      |
| ----------- | ----------- | ------- |
| `total`     | `AtomicU64` | 接收的总任务数 |
| `completed` | `AtomicU64` | 成功完成数   |
| `failed`    | `AtomicU64` | 失败数     |

### 枚举与配置

#### `TaskStatus` — 任务状态

| 变体           | 值 | 说明          |
| ------------ | - | ----------- |
| `Pending`    | 1 | 等待处理        |
| `Processing` | 2 | 处理中         |
| `Completed`  | 3 | 已完成         |
| `Failed`     | 4 | 失败（等待重试）    |
| `Cancelled`  | 5 | 已取消         |
| `Scheduled`  | 6 | 已调度（定时任务预留） |
| `DeadLetter` | 7 | 死信（超过最大重试）  |

`is_terminal()` 判断是否为终态（Completed / Cancelled / DeadLetter）。

#### `RetryStrategy` — 重试策略

| 变体            | 公式                   | 说明   |
| ------------- | -------------------- | ---- |
| `Fixed`       | `base`               | 固定延迟 |
| `Linear`      | `base × (attempt+1)` | 线性增长 |
| `Exponential` | `base × 2^attempt`   | 指数退避 |

#### `TaskConfig` — 每种 task\_type 一份配置

| 字段                        | 类型               | 说明                            |
| ------------------------- | ---------------- | ----------------------------- |
| `task_type`               | `i16`            | 任务类型标识                        |
| `priority`                | `TaskPriority`   | 优先级（Low/Normal/High/Critical） |
| `topic`                   | `String`         | Kafka topic 名称                |
| `timeout_seconds`         | `u64`            | 执行超时（秒）                       |
| `max_retries`             | `u32`            | 最大重试次数                        |
| `retry_strategy`          | `RetryStrategy`  | 重试策略                          |
| `retry_delay_seconds`     | `u64`            | 基础重试延迟                        |
| `max_retry_delay_seconds` | `u64`            | 最大重试延迟（上限）                    |
| `description`             | `&'static str`   | 任务描述                          |
| `dead_letter_topic`       | `Option<String>` | 死信队列 topic（None 不启用）          |

#### `TaskWorkerConfig` — 运行时配置（支持从 `[task]` section 反序列化）

| 字段                     | 类型       | 默认值                | 说明              |
| ---------------------- | -------- | ------------------ | --------------- |
| `brokers`              | `String` | `localhost:9092`   | Kafka broker 地址 |
| `group_id`             | `String` | `alun-task-worker` | 消费组 ID          |
| `scan_interval_secs`   | `u64`    | 30                 | 重试扫描间隔（秒）       |
| `max_batch_size`       | `usize`  | 100                | 每批次扫描最大任务数      |
| `max_message_age_secs` | `u64`    | 3600               | 消息最大时效（秒）       |
| `auto_create_topics`   | `bool`   | false              | 启动时自动创建 topic   |
| `topic_partitions`     | `i32`    | 1                  | topic 分区数       |
| `topic_replication`    | `i16`    | 1                  | topic 副本数       |

### `#[task_handler]` Proc Macro

位于 `alun-macros/src/task_handler.rs`，由 `alun` 门面 re-export 为 `alun::task_handler`。

**参数一览**：

| 参数                        | 类型                                 | 默认值                  | 必填    |
| ------------------------- | ---------------------------------- | -------------------- | ----- |
| `task_type`               | `i16`                              | —                    | **是** |
| `topic`                   | `&str`                             | `"task_{task_type}"` | 否     |
| `priority`                | `"Normal"/"High"/"Low"/"Critical"` | Normal               | 否     |
| `timeout_seconds`         | `u64`                              | 300                  | 否     |
| `max_retries`             | `u32`                              | 3                    | 否     |
| `retry_strategy`          | `"Fixed"/"Linear"/"Exponential"`   | Linear               | 否     |
| `retry_delay_seconds`     | `u64`                              | 30                   | 否     |
| `max_retry_delay_seconds` | `u64`                              | 300                  | 否     |
| `description`             | `&str`                             | ""                   | 否     |
| `dead_letter_topic`       | `Option<&str>`                     | None                 | 否     |

**编译期行为**：

1. 为结构体生成 `linkme::distributed_slice(TASK_HANDLERS)` 静态项
2. `TASK_HANDLERS` 在链接期自动汇集所有标注了 `#[task_handler]` 的 handler
3. `HandlerRegistry::from_discovered()` 读取切片并一键注册

### 使用示例

```rust
use alun::alun_task::*;
use async_trait::async_trait;

// ── 1. 定义 Handler（宏自动发现） ──
#[alun::task_handler(
    task_type = 1,
    topic = "export_tasks",
    timeout_seconds = 60,
    max_retries = 3,
    retry_strategy = "Exponential",
    retry_delay_seconds = 10,
    description = "数据导出任务",
    dead_letter_topic = "export_dlq"
)]
struct ExportHandler;

#[async_trait]
impl TaskHandler for ExportHandler {
    fn task_type(&self) -> i16 { 1 }
    async fn execute(&self, payload: Value) -> Result<Value, String> {
        let file_id = payload["file_id"].as_str().unwrap_or("");
        // 执行导出逻辑 ...
        Ok(json!({"url": "https://...", "file_id": file_id}))
    }
}

// ── 2. 实现 TaskStorage（内部通过 db() 全局函数操作数据库，无需持有连接） ──
struct DbTaskStorage;

#[async_trait]
impl TaskStorage for DbTaskStorage {
    async fn save_task_log(&self, task_id: &str, task_type: i16, priority: i16,
        config: &TaskConfig, params: &SubmitTaskParams) -> Result<(), String>
    {
        let row = Row::table("task_logs")
            .id(Sid::uuid())
            .set("task_id", task_id)
            .set("task_type", task_type)
            .set("priority", priority)
            .set("topic", &config.topic)
            .set("payload", params.payload.to_string());
        db().insert(&row).await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn save_task_queue(&self, task_id: &str, topic: &str, priority: i16) -> Result<(), String> {
        let row = Row::table("task_queue")
            .id(Sid::uuid())
            .set("task_id", task_id)
            .set("topic", topic)
            .set("priority", priority);
        db().insert(&row).await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn update_task_status(&self, task_id: &str, status: TaskStatus) -> Result<(), String> {
        db().execute("UPDATE task_logs SET status = $1 WHERE task_id = $2",
            &[&serde_json::to_value(status.to_i16()).unwrap(), &serde_json::to_value(task_id).unwrap()])
            .await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn get_retry_count(&self, task_id: &str) -> Result<i64, String> {
        let row = db().query_one("SELECT retry_count FROM task_logs WHERE task_id = $1",
            &[&serde_json::to_value(task_id).unwrap()]).await.map_err(|e| e.to_string())?;
        Ok(row.and_then(|r| r.get_as::<i64>("retry_count")).unwrap_or(0))
    }

    async fn update_retry(&self, task_id: &str, retry_count: i64) -> Result<(), String> {
        db().execute("UPDATE task_logs SET retry_count = $1, status = 1 WHERE task_id = $2",
            &[&serde_json::to_value(retry_count).unwrap(), &serde_json::to_value(task_id).unwrap()])
            .await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn save_task_result(&self, task_id: &str, output: &Value) -> Result<(), String> {
        let row = Row::table("task_results")
            .id(Sid::uuid())
            .set("task_id", task_id)
            .set("output", output.to_string());
        db().insert(&row).await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn log_execution(&self, task_id: &str, status: TaskStatus, error: Option<&str>,
        elapsed_ms: i64) -> Result<(), String>
    {
        let row = Row::table("task_execution_logs")
            .id(Sid::uuid())
            .set("task_id", task_id)
            .set("status", status.to_i16())
            .set("error", error.unwrap_or(""))
            .set("elapsed_ms", elapsed_ms);
        db().insert(&row).await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn scan_retryable_tasks(&self, task_types: &[i16], limit: usize) -> Result<Vec<RetryableTask>, String> {
        let types = task_types.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT task_id, task_type, retry_count, max_retries, payload FROM task_logs \
             WHERE status = 4 AND task_type IN ({}) AND retry_count < max_retries ORDER BY priority DESC LIMIT {}",
            types, limit
        );
        let rows = db().query(&sql, &[]).await.map_err(|e| e.to_string())?;
        rows.iter().map(|r| {
            Ok(RetryableTask {
                task_id: r.get_as::<String>("task_id").unwrap_or_default(),
                task_type: r.get_as::<i16>("task_type").unwrap_or(0),
                retry_count: r.get_as::<i64>("retry_count").unwrap_or(0),
                max_retries: r.get_as::<i64>("max_retries").unwrap_or(0),
                payload: r.get_as::<String>("payload")
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or(Value::Null),
            })
        }).collect()
    }
}

// ── 3. 启动时创建插件（TaskStorage 无字段，Arc::new 即可） ──
#[tokio::main]
async fn main() {
    App::new().unwrap();  // 初始化全局资源（DB/Cache/Config）

    let task_cfg: TaskWorkerConfig = cfg().custom.get("task")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    App::new().unwrap()
        .plugin(TaskPlugin::new(
            task_cfg,
            Arc::new(DbTaskStorage),
            HandlerRegistry::new().from_discovered(),
        ).unwrap())
        .scan().start().await.unwrap();
}

// ── 4. 在 Handler 中提交任务（直接 Arc::new，无需传 db） ──
#[alun::post("/api/export")]
async fn export() -> Result<Res<String>, ApiError> {
    let task_cfg: TaskWorkerConfig = cfg().custom.get("task")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let producer = TaskProducer::new(
        &task_cfg.brokers,
        Arc::new(DbTaskStorage),
        HandlerRegistry::new().from_discovered(),
    ).map_err(|e| ApiError::internal(e))?;

    let task_id = producer.submit(SubmitTaskParams {
        task_type: 1,
        payload: json!({"file_id": "f1", "format": "xlsx"}),
        priority: Some(TaskPriority::High),
        user_id: Some("u1".into()),
        resource_id: None, resource_type: None,
    }).await.map_err(|e| ApiError::internal(e))?;

    Ok(Res::ok_with_msg(task_id, "任务已提交"))
}

// ── 5. 批量提交 ──
let batch = SubmitBatchParams {
    tasks: (0..10).map(|i| SubmitTaskParams {
        task_type: 1,
        payload: json!({"file_id": format!("f{}", i)}),
        priority: None, user_id: None, resource_id: None, resource_type: None,
    }).collect(),
};
let (succeeded, failures) = producer.submit_batch(batch).await;

// ── 6. 配置文件示例 ──
// [task]
// brokers = "kafka.internal:9092"
// group_id = "my-app-task-worker"
// scan_interval_secs = 30
// max_batch_size = 100
// max_message_age_secs = 3600
// auto_create_topics = false
```


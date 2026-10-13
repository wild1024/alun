---
id: reference.crates.web
title: `alun-web` — Web 核心层
summary: `alun-web` — Web 核心层——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [web, reference, routing, middleware, jwt]
status: active
owner: alun-core-team
source_of_truth: [alun-web/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-web` — Web 核心层

> 本文件是 [`crates.md`](../crates.md) 的 `web` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun-web/src/`

提供 App 构建器、路由注册、中间件体系和共享状态。基于 axum + tower 构建。

## 关键结构体

### `App` （[app.rs](../../../alun-web/src/app.rs)）

框架唯一入口，Builder 模式，所有路由注册方法返回 `Self`。无泛型状态参数——运行时资源（Db/Cache/Config/Template）通过全局函数访问。

| 方法                                              | 说明                                  |
| ----------------------------------------------- | ----------------------------------- |
| `App::new()`                                    | 等价 `from_config_dir("config")`，从 `config/` 加载配置 |
| `App::from_config()`                            | 同上，语义明确                             |
| `App::from_config_dir(dir)`                     | 从指定目录加载配置                           |
| `App::with_config(cfg)`                         | 直接使用 `AppConfig`（跳过文件加载）            |
| `App::with_config_manager(cm)`                  | 使用已有的 `Arc<ConfigManager>`             |
| `.get/.post/.put/.delete(path, handler)`        | 注册对应方法的单条路由                         |
| `.route(method, path, handler)`                 | 通用路由注册（`method` 为 `&str`，如 `"GET"`）   |
| `.group(prefix, closure)`                       | 路由分组（嵌套前缀）                          |
| `.merge(prefix, sub_router)`                    | 合并已构建的 AlunRouter                   |
| `.scan()`                                       | 扫描 Proc Macro 注解自动注册路由              |
| `.with_permission(method, path, handler, perm)` | 注册带方法级权限校验的路由                       |
| `.with_role(method, path, handler, role)`       | 注册带方法级角色校验的路由                       |
| `.on_startup(hook)`                             | 注册启动回调（全局资源就绪后、插件启动前执行）             |
| `.layer(L)`                                     | 挂 tower 层：**整站最外层**（含 401/403/429、404、`/metrics`），**读不到身份** |
| `.layer_from_fn(fn(Router) -> Router)`          | 同上，函数式写法（要 `nest` / `merge` 时用）；旧名 `.with_middleware_hook` 已废弃 |
| `.route_layer(L)`                               | 挂 tower 层：**链内**（位于 Auth/权限之内），只作用于匹配路由，**能**读身份 |
| —                                               | 子树级 layer：`merge_with_layer` 是 **`AlunRouter`** 的方法，经 `.routes(register)` 拿到 `&mut AlunRouter` 后调用（见下表） |
| `.plugin(plugin)`                               | 注册插件                                |
| `.parse_cli()`                                  | 解析 CLI（`--gen-config` / `--print-config` / `--profile`） |
| `.start()`                                      | 启动，监听地址取自配置文件 `[server] listen`      |
| `.serve(addr)`                                  | 指定地址启动（无配置文件时用该地址构造默认配置）            |

> `App` 没有泛型状态参数，也没有 `App::empty()`；运行时资源（Db/Cache/Config/Template）一律通过 `alun_web::resources` 的全局函数访问。

### `AlunRouter` （[router.rs](../../../alun-web/src/router.rs)）

路由注册器——延迟构建 axum Router。所有路由以闭包暂存，`into_axum()` 一次性构建。

| 方法                                                            | 说明                                              |
| ------------------------------------------------------------- | ----------------------------------------------- |
| `new()`                                                       | 创建空注册器                                          |
| `add_get(path, handler)` / `add_post` / `add_put` / `add_delete` | 注册 GET/POST/PUT/DELETE（**不是** `get`/`post`…）      |
| `add_route(method, path, handler)`                            | 通用方法注册（`method` 为 `&str`）                       |
| `add_get_with_layer(path, handler, wrap)`                     | 带 tower Layer 的 GET（`add_post_with_layer` 等同理）    |
| `merge(prefix, sub)`                                          | 合并子路由（只搬路由，不搬 layer）                            |
| `merge_with_layer(prefix, sub, wrap)`                         | 合并子路由并只给该前缀下的整组路由挂一层（`wrap: FnOnce(Router) -> Router`） |
| `into_axum()`                                                 | 构建最终 axum Router                                |

> `AlunRouter` 的方法都取 `&mut self`（原地注册，无返回值），`App` 的同名方法才是 builder 风格返回 `Self`。
> 路由级 layer 也可以写成属性宏 `#[alun::layer(L)]` —— 需放在路由注解**上方**，宏会摘掉该路由的
> 编译期自注册、改生成「带 layer 的注册」，因此不会重复挂载。

### `resources` 模块 （[resources.rs](../../../alun-web/src/resources.rs)）

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

### `JWT` （[jwt.rs](../../../alun-web/src/jwt.rs)）

Token 签发与校验（内部缓存 `jsonwebtoken` 的 `EncodingKey`/`DecodingKey`）。

| 方法                                                                                | 说明                                             |
| --------------------------------------------------------------------------------- | ---------------------------------------------- |
| `JWT::from_config()`                                                              | 从全局配置构建                                        |
| `JWT::with_config(Arc<ConfigManager>)`                                            | 指定配置管理器                                        |
| `JWT::with_config_and_cache(cm, SharedCache)`                                     | **需 `cache` feature**                           |
| `jwt_secret()` / `access_token_expire_secs()` / `refresh_token_expire_secs()`       | 读取密钥与过期秒数                                      |
| `create_access_token(user_id, username: Option<&str>, roles, permissions)`        | `-> Result<String, String>`                    |
| `create_refresh_token(user_id)`                                                   | `-> Result<String, String>`                    |
| `validate(token)`                                                                 | `-> Result<TokenClaims, String>`               |
| `blacklist(&claims)` / `is_blacklisted(&claims)` / `logout(&claims)` / `refresh(refresh_token)` | **需 `cache` feature**，均为 async                |

### `TokenClaims` （[middleware/mod.rs](../../../alun-web/src/middleware/mod.rs)）

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

**关键方法**：`user_id()`（即 `sub`）、`has_role()`、`has_any_role()`、`has_all_roles()`、`has_permission()`、`has_any_permission()`、`is_super_admin()`

配套类型（同样定义在 `middleware/mod.rs`）：

| 类型                                                   | 形态                       | 说明                                                     |
| ---------------------------------------------------- | ------------------------ | ------------------------------------------------------ |
| `TokenType`                                          | 枚举                       | `Access` / `Refresh`（serde 重命名为 `"access"`/`"refresh"`） |
| `UserId`                                             | **tuple struct** `UserId(pub String)` | 也可作为提取器取出当前用户 ID                                        |
| `AuthClaims`                                         | **tuple struct** `AuthClaims(pub TokenClaims)` | **提取器**，双通道：① 读请求扩展（认证层已注入）② 回退自行验签 + 查黑名单；因此 `#[no_auth]` 路由、`into_router()` 测试/嵌入路径都取得到。也可用 `Extension(AuthClaims(claims))` 解构，**不是**扁平结构 |
| `Principal`                                          | struct（字段公开）              | 身份**业务视图**：`user_id: UserId`（`sub` 原样字符串，**非** `Uuid`——主键形态由业务决定）/ `username: Option<String>` / `roles: Vec<String>` / `claims: Arc<TokenClaims>`，另有 `has_role` / `has_permission` 等（透传通配语义）。实现 `FromRequestParts` + `OptionalFromRequestParts`，已进 prelude；**不含** `is_super_admin()`（用 `p.claims.is_super_admin()`） |
| `UserId(pub String)`                                 | 身份 ID                       | JWT `sub` 的原样标识；`as_str()` / `Display` / `AsRef<str>`，需要强类型时 `parse::<T>()`（失败得 `None`，身份不丢） |
| `Option<AuthClaims>` / `Option<Principal>`            | 可选提取                    | 用于「登录与否都能访问」的接口：无 token 得 `None`，**不报 401**                  |

### `ValidatedJson<T>` （[extract.rs](../../../alun-web/src/extract.rs)）

带自动 JSON 解析校验的请求提取器（实现 `FromRequest`）。它是 **tuple struct** `ValidatedJson<T>(pub T)`，实现了 `Deref` / `DerefMut`，可通过 `req.0` 或 `*req` 取出内部值。

若目标类型实现 `validator::Validate`，可通过 `.validate()` 方法执行字段级校验（失败返回 `ApiError`）；提取失败时使用其 rejection 类型 `ValidatedJsonRejection(pub ApiError)` 返回统一错误响应。

### `ValidateExt` Trait （[extract.rs](../../../alun-web/src/extract.rs)）

为所有实现 `validator::Validate` 的类型提供便捷的 `validate_or_reject()` 方法，一行完成字段级校验，校验失败自动返回 422 `ApiError`。

> `alun-web` **不**再导出 `validator` 本身，其公开签名（9 个 `validate_*` 的返回类型、`ValidateExt`/`ValidatedJson` 的约束）会泄漏 `validator::ValidationError` / `validator::Validate`，因此使用方需自行依赖 `validator`。

### 自定义验证函数

`alun-web` 内置了以下 validator 自定义校验函数，可直接用于 `#[validate(custom(function = "..."))]` 属性：

| 函数                                       | 说明                                     |
| ---------------------------------------- | -------------------------------------- |
| `validate_uuid(value)`                   | 验证 UUID 格式（v1\~v7 均支持）                 |
| `validate_mobile(value)`                 | 验证手机号（中国大陆）                            |
| `validate_password_strength(value)`      | 验证密码强度（≥8位，含大小写+数字+特殊字符）               |
| `validate_id_card(value)`                | 验证中国居民身份证号（含校验位）                       |
| `validate_date(value)`                   | 验证日期格式（YYYY-MM-DD）                     |
| `validate_datetime(value)`               | 验证日期时间格式（ISO 8601 / RFC 3339 或 YYYY-MM-DD） |
| `validate_date_or_datetime(value)`       | 验证日期（YYYY-MM-DD）或日期时间（ISO 8601/RFC 3339） |
| `validate_email(value)`                  | 验证邮箱格式                                  |
| `validate_url(value)`                    | 验证 URL 格式                               |

### `Res<T>` （[api.rs](../../../alun-web/src/api.rs)）

统一 API 响应体，实现 `axum::IntoResponse`。

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

### `ApiError` （[api.rs](../../../alun-web/src/api.rs)）—— handler 的唯一错误类型

对外暴露的统一 API 错误类型，实现 `IntoResponse`，5xx 自动把内部详情写日志、前端只看通用提示。

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

**内置 `From`（下层错误 → HTTP 语义，业务端直接 `?` 即可）**：

| 来源 | 结果 |
| ---- | ---- |
| `DbError`（feature `db`） | `NotFound`→404；`UniqueViolation`/`ForeignKeyViolation`/`CheckViolation`→409；`Deadlock`/`SerializationFailure`/`LockTimeout`→500「服务繁忙，请重试」；其余→500 脱敏 |
| `alun_core::Error` | 500 脱敏（`internal_detail` 记原文） |
| `std::io::Error` | 500 脱敏 |

另有反向通道 `From<ApiError> for alun_core::Error`，供启动 / 插件 / 定时任务复用同一份文案。

### `PageQuery` / `PageData<T>` （[api.rs](../../../alun-web/src/api.rs)）

分页查询公共类型。

**`PageQuery`** — 分页参数（全框架唯一来源，Web 提取器与数据库分页共用）：

- `new(page, page_size)` — 创建，自动规整到合法范围（page>=1, 1<=page\_size<=1000）
- `offset()` — 计算 SQL OFFSET（越界值先规整，不会下溢）
- `limit()` — 获取规整后的 LIMIT 值
- `calc_pages(total)` — 计算总页数，口径与 `alun_db::Page::calc_pages` 一致
- 反序列化即规整：`?page=0`、参数缺省都会落到合法范围（不会 400，也不会 panic）
- 常量 `DEFAULT_PAGE_SIZE`(10) / `MAX_PAGE_SIZE`(1000)

数据库分页结果 `alun_db::Page<T>`（字段 `list/total/page/size/pages`）转标准响应：

```rust,ignore
let page = db().paginate("SELECT * FROM users ORDER BY id", pq.page, pq.page_size)
    .await?
    .map(|r| UserRes { id: r.get_typed("id").unwrap_or_default() });   // Row → DTO
Ok(page.into_res())   // PageResponseExt：Page<T> → Res<PageData<Vec<T>>>
```

**`PageData<T>`** — 分页数据结构（响应侧）：

- `list: T` — 数据列表
- `total: u64` — 总条数
- `page: u64` — 当前页码
- `page_size: u64` — 每页条数
- `new(list, total, page, page_size)` — 构造
- 与数据库侧 `alun_db::Page<T>` 字段名不同（`size` ↔ `page_size`），转换统一走
  `alun_web::PageResponseExt`，不要手工搬运字段

### 错误码常量 （`alun_web::api::codes`）

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

## 中间件体系

| 中间件               | Layer 类型                        | 类型  | 说明                                                                                         |
| ----------------- | ------------------------------- | --- | ------------------------------------------------------------------------------------------ |
| SecurityHeaders   | `SecurityHeadersLayer`          | 全局  | 注入 6 个安全响应头（nosniff/frame/HSTS/CSP/Referrer/Permissions）                                   |
| RequestId         | `RequestIdLayer`                | 全局  | 为每个请求生成请求 ID 并写入**请求头与响应头** `x-request-id`；客户端传入时原样回带（大小写不敏感），未传时生成 16 位十六进制 |
| RequestLog        | `RequestLogLayer`               | 全局  | 记录请求路径/状态码/耗时/客户端IP（tracing 输出）；操作审计日志（含请求参数、响应体、脱敏、写库）请通过 `App::layer` / `layer_from_fn` 注入项目级中间件（必须留在最外层，才能记到 401/403/429） |
| Auth              | `AuthLayer`                     | 全局  | JWT Bearer Token 验证 + 黑名单检查；支持 `ignore_paths` 配置和 `#[no_auth]` 宏注解                         |
| CORS              | `CorsLayer` (tower-http)        | 全局  | 跨域配置                                                                                       |
| Compression       | `CompressionLayer` (tower-http) | 全局  | Gzip 响应压缩                                                                                  |
| RateLimit         | `RateLimitLayer`                | 全局  | IP 滑动窗口限流                                                                                  |
| PermissionCheck   | `PermissionCheckLayer`          | 全局  | 路径权限规则校验（配置文件规则受 `enabled` 控制；`#[permission]` 宏注解规则**默认挂载**）                          |
| RequirePermission | `RequirePermissionLayer`        | 方法级 | 权限标识校验（any 匹配）                                                                             |
| RequireRole       | `RequireRoleLayer`              | 方法级 | 角色校验（any 匹配）                                                                               |
| Nonce             | `NonceLayer`                    | 方法级 | Nonce 防重放（按需，建议写操作路由）                                                                      |
| Idempotency       | `IdempotencyLayer`              | 方法级 | 幂等键（按需，建议订单/支付路由）                                                                          |

> 装配方式：`[middleware.nonce] enabled = true` / `[middleware.idempotency] enabled = true`（`ttl_secs` 可调）时由 `App` **按配置自动入链**（需 `cache` feature 且 `[cache]` 已初始化，否则打 warn 跳过）；也可手工 `NonceLayer::new(cache, ttl)` / `IdempotencyLayer::new(cache, ttl)`（同样**需 `cache` feature**）只挂在指定路由上。`AuthLayer` **没有** `new()`，用结构体字面量 `AuthLayer { jwt_secret, ignore_paths, .. }`（`cache` 字段在 `cache` feature 下存在）由 `App` 按配置组装；`RequireRoleLayer::any(roles)` / `RequirePermissionLayer::any(perms)` 是便捷构造；`PermissionCheckLayer::from_config(&cfg_rules).with_macro_rules(&PERMISSION_ROUTES)` 用于全局路径权限。

### Auth 中间件认证行为

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

### PermissionCheck 中间件权限校验行为

`PermissionCheckLayer` 支持两种方式定义权限规则：

1. **配置文件**：`[middleware.permission.rules]` 列表
2. **宏注解**：`#[alun::permission(path = "/api/admin", method = "GET", permission = "admin:access")]` 标记

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

```rust,ignore
#[alun::permission(path = "/api/admin/users", method = "GET", permission = "admin:read")]
#[alun::get("/api/admin/users")]
async fn list_users() -> Res<Vec<User>> {
    Res::ok(users)
}
```

## 分布式切片

| 静态变量                                 | 说明                                        |
| ------------------------------------ | ----------------------------------------- |
| `discovered_routes()`                | 遍历编译期注册的路由项（原 `ROUTES` 切片）——`#[get]`/`#[post]` 宏注解在此汇集 |
| `discovered_permissions()`           | 收集编译期注册的权限规则（原 `PERMISSION_ROUTES` 切片） |
| `discovered_no_auth()`               | 收集编译期注册的免认证路径（原 `NO_AUTH_ROUTES` 切片） |

## 使用示例

```rust,ignore
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

// ── 带权限的路由（method 为字符串） ──
#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        .with_permission("GET", "/api/admin", admin_handler, "admin:read")
        .with_role("DELETE", "/api/users/{id}", delete_user, "admin")
        .serve("8080")
        .await
}

// ── 自定义中间件注入 ──
// 审计 / 访问日志「必须记全」⇒ 用最外层 layer（能记到 401/403/429 与真实端到端耗时）
#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        .layer_from_fn(|router| {
            router.layer(axum::middleware::from_fn(my_audit_log_middleware))
        })
        .serve("8080")
        .await
}

// ── 链内中间件：要用到「当前身份」时用 route_layer ──
// 位于 Auth / 权限之内 ⇒ 可提取 AuthClaims / Principal
#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        .route_layer(DataScopeLayer::new())
        .serve("8080")
        .await
}

// ── 从指定目录加载配置后启动 ──
#[tokio::main]
async fn main() -> alun::Result<()> {
    App::from_config_dir("config")?
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
use alun_web::middleware::IdempotencyLayer;
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

// ── 获取上传/下载目录路径（顶层不导出，走 resources） ──
use alun_web::resources::{upload_path, download_path};

#[alun::post("/api/file/upload")]
async fn upload_file() -> Res<String> {
    let path = upload_path();  // 返回 "uploads"（或 config.toml 中配置的值）
    let full = format!("{}/{}", path, "report_2024.pdf");
    // 执行文件保存 ...
    Res::ok(full)
}

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

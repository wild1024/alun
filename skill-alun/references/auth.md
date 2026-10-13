# Authentication, Authorization & Middleware

## JWT Configuration (`config.toml`)

```toml
[middleware.auth]
enabled = true
jwt_secret = "your-secret-minimum-32-characters-long"
ignore_paths = ["/api/login", "/api/register"]
access_token_expire_secs = 7200
refresh_token_expire_secs = 604800
```

字段与默认值（`alun-config/src/lib.rs:308-327`、`Default` 在 `alun-config/src/lib.rs:768`）：

| 字段 | 默认 | 说明 |
|------|------|------|
| `enabled` | `false` | 认证中间件总开关 |
| `jwt_secret` | `""` | HS256 密钥 |
| `ignore_paths` | `[]` | 免认证白名单（不含 router prefix，启动时自动拼接） |
| `access_token_expire_secs` | `7200` | Access Token 有效期 |
| `refresh_token_expire_secs` | `604800` | Refresh Token 有效期 |

装配条件：**`enabled = true` 且 `jwt_secret` 非空**，否则整层不挂载
（`alun-web/src/app.rs:780`）。

> `ignore_paths` 支持精确匹配和**前缀匹配**，但前缀必须以 `/` 为边界：配置 `/api` 会让
> `/api/login`、`/api/user/info` 等被忽略认证，`/api2` 不会
> （`alun-web/src/middleware/auth.rs:89-92`）。
>
> `jwt_secret` 可被环境变量覆盖：`FACAI_JWT_SECRET`，或通用变量
> `ALUN_MIDDLEWARE_AUTH_JWT_SECRET`（`alun-config/src/env.rs:53-60`、`:86`）。

## JWT Manager

```rust,no_run
use alun::prelude::*;
use alun::JWT;

# async fn demo() -> Result<()> {
let jwt = JWT::from_config();

// Login — generate tokens
let access = jwt.create_access_token(
    "user_id", Some("username"),
    &["admin".into()],
    &["user:read".into()],
)?;

let refresh = jwt.create_refresh_token("user_id")?;

// Validate — returns TokenClaims（只校验签名与有效期）
let claims: TokenClaims = jwt.validate(&access)?;

// Refresh — validates old token, blacklists it, returns new pair
let (new_access, new_refresh) = jwt.refresh(&refresh).await?;

// Logout — adds token to blacklist
jwt.logout(&claims).await;
# let _ = (new_access, new_refresh);
# Ok(())
# }
```

构造与取值（`alun-web/src/jwt.rs:43-84`）：

| 方法 | 说明 |
|------|------|
| `JWT::from_config()` | 从全局 `config()` 创建；全局缓存已初始化时自动关联（黑名单可用） |
| `JWT::with_config(Arc<ConfigManager>)` | 不依赖全局单例；不带缓存。`ConfigManager` 未被门面转发，此路径需直接声明 `alun-config` |
| `JWT::with_config_and_cache(config, cache)` | 需 `cache` feature；参数类型来自 `alun-config` / `alun-cache`，同样需直接声明 |
| `jwt_secret()` / `access_token_expire_secs()` / `refresh_token_expire_secs()` | 读取配置 |

> 常规用法直接用 `JWT::from_config()`（门面已导出 `JWT`，`alun/src/lib.rs:1152`）。

**黑名单相关方法是 `cache` feature 门控的**（`alun-web/src/jwt.rs:152-231`）：

- `blacklist(&claims)`：把 `jti` 写入 `token:blacklist:{jti}`，TTL = Token 剩余有效期
  （已过期则 1 分钟）；未初始化缓存时静默跳过，且 `jti` 为 `None` 时不写入。
- `is_blacklisted(&claims)`：缓存未初始化或无 `jti` 时返回 `false`。
- `logout(&claims)`：就是 `blacklist(&claims)`。
- `refresh(&refresh_token)`：校验 → 类型必须是 `Refresh`（否则 `Err`）→ 查黑名单
  （已撤销则 `Err`）→ 把旧 Refresh Token 加黑名单 → 签发新的一对
  （`alun-web/src/jwt.rs:205-231`）。
- `validate()` **不检查黑名单**，黑名单校验由 `AuthLayer` 中间件在请求路径上完成。

## Accessing Current User in Handlers

**推荐用提取器**（不是一个 `Extension<AuthClaims>` 参数也没问题）：参数解析阶段自动取值，零顺序依赖。

```rust,no_run
use alun::prelude::*;

// ① 只要身份视图：Principal（推荐 —— 字段是业务语言）
#[alun::get("/api/auth/me")]
async fn me(p: Principal) -> Res<JsonValue> {
    Res::ok(json!({
        // 原样 sub：UUID / 自增整型 / 复合键 / IdP 主体都可用，永不丢失
        "user_id": p.user_id.as_str(),
        "username": p.username,
        "roles": p.roles,
    }))
}

// ② 要权限列表等完整载荷：直接用 AuthClaims
#[alun::get("/api/auth/claims")]
async fn claims(claims: AuthClaims) -> Res<JsonValue> {
    Res::ok(json!({
        "user_id": claims.0.sub,
        "roles": claims.0.roles,
        "permissions": claims.0.permissions,
    }))
}

// ③ 登录与否都能访问：Option<...>（无 token 得 None，不报 401）
#[alun::get("/api/feed")]
async fn feed(who: Option<Principal>) -> Res<String> {
    let name = who.and_then(|p| p.username).unwrap_or_else(|| "游客".into());
    Res::ok(format!("你好，{name}"))
}
```

### 提取器的两条通道（重要）

`AuthClaims` 取值的顺序是：

1. **通道 1**：读请求 extensions（`AuthLayer` 已注入）—— 常规路径；
2. **通道 2**：扩展里没有时，**自行解析 `Authorization: Bearer <token>`、验签并查黑名单**。

带来三个直接好处：

- **`#[no_auth]` 路由上也能拿到身份**（登录用户仍可被识别；匿名则为「无身份」）——
  以前只有被认证层覆盖到的路由才拿得到；
- **不依赖顺序**：写在 `App::route_layer` 里的中间件、或测试里直接发包，都不影响提取；
- `App::into_router()` 的测试 / 嵌入路径同样可用（通道 2 依赖全局配置，请先 `set_config`）。

> 通道 2 在**无 token** 时返回 **401**（不是 500）；坏 token / 已登出（命中黑名单）同样是 401。

`Principal` 是 `AuthClaims` 的**业务视图**：`user_id: UserId` / `username: Option<String>` /
`roles: Vec<String>` / `claims: Arc<TokenClaims>`，以及 `has_role` / `has_any_role` /
`has_permission` / `has_any_permission`（透传通配语义 —— `*` / `*:*:*` 超级管理员恒为 true）。
它**不**内置 `is_super_admin()`：那属于框架策略；业务要判断请写 `p.claims.is_super_admin()`。

### `user_id` 不是 `Uuid`，而是原样字符串

`user_id` 的类型是 `UserId`（`sub` 的**原样标识**），因为主键形态由业务决定，框架无从假设：

| 主键形态 | `sub` 实例 | 取法 |
|---|---|---|
| UUID | `22222222-2222-…` | `p.user_id.parse::<uuid::Uuid>()` |
| 自增 / 雪花整型 | `1024` | `p.user_id.parse::<i64>()` |
| 复合作用域键 | `u7:t1` | `p.user_id.as_str().split_once(':')` |
| 外部 IdP 主体 | `auth0\|5f3c…`、微信 `openid` | `p.user_id.as_str()` |

```rust,no_run
use alun::prelude::*;

#[alun::get("/api/orders")]
async fn orders(p: Principal) -> Res<String> {
    let raw = p.user_id.as_str();                    // 永远可用
    let id = p.user_id.parse::<i64>();                // 强类型在调用点声明
    Res::ok(format!("{raw} / {id:?}"))
}
```

`UserId` 另有 `Display` 与 `AsRef<str>`，可直接进 `format!`。需要严格类型时就在调用点
`parse::<T>()`（解析失败得 `None`，**身份本身不丢**）——这比把 `user_id` 定成
`Option<Uuid>` 更稳：后者会让非 UUID 系统**整体拿不到身份**，只能回退去读 `claims.sub`。

旧写法仍然兼容：

```rust,no_run
use alun::prelude::*;

#[alun::get("/api/auth/me-ext")]
async fn me_ext(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<JsonValue> {
    Res::ok(json!({ "user_id": claims.sub }))
}
```

认证通过后 `AuthLayer` 还会向请求 extensions 注入两个键（`alun-web/src/middleware/auth.rs`）：

- `UserId(String)` —— 只关心用户 ID 时用它；
- `AuthClaims(TokenClaims)` —— 需要角色/权限时用它。

> 这两个键**只在认证层覆盖到的路由上存在**；用提取器则无此限制（扩展里没有会自动走通道 2）。

## `TokenClaims` 与 Helper Methods

字段（`alun-web/src/middleware/mod.rs:59-86`）：
`jti: Option<String>`、`sub: String`、`username: Option<String>`、`roles: Vec<String>`、
`permissions: Vec<String>`、`token_type: Option<TokenType>`、`exp: usize`、`iat: usize`。

```rust,no_run
use alun::prelude::*;

# fn check(claims: &TokenClaims) {
claims.user_id();                     // &str（= sub）
claims.has_role("admin");
claims.has_any_role(&["admin", "mod"]);
claims.has_all_roles(&["read", "write"]);
claims.has_permission("user:read");   // 超级管理员（"*" 或 "*:*:*"）恒为 true
claims.has_any_permission(&["user:read", "user:write"]);
claims.is_super_admin();              // 拥有 "*" 或 "*:*:*" 权限
# }
```

`has_permission` 只做**权限字符串精确匹配**（或通配 `*` / `*:*:*`），没有 `user:*` 这类
前缀/模式匹配（`alun-web/src/middleware/mod.rs:108-113`）。

## Permission & Role Guards

**Builder style**（`alun-web/src/app.rs:307-366`）：

```rust,no_run
use alun::prelude::*;

# async fn demo() -> Result<()> {
App::new()?
    .with_permission("GET", "/api/admin/stats", admin_stats, "admin:access")
    .with_role("DELETE", "/api/users/{id}", delete_user, "admin")
    .start()
    .await
# }
# async fn admin_stats() -> Res<String> { Res::ok("".into()) }
# async fn delete_user(Path(id): Path<String>) -> ResResult<()> { let _ = id; Ok(Res::ok_empty()) }
```

- `with_permission(method, path, handler, permission)` 给该路由挂 `RequirePermissionLayer::any`：
  拥有**任一**所需权限即放行，未认证返回 401，权限不足返回 403
  （`alun-web/src/middleware/permission.rs:52-83`）。
- `with_role(...)` 同理走 `RequireRoleLayer::any`，拥有任一角色即放行
  （`alun-web/src/middleware/role.rs:52-84`）。
- `method` 大小写不敏感（内部 `to_uppercase()`）；**未识别的 method 会退化为 GET 注册**
  （`alun-web/src/app.rs:317-324`）。

**Proc macro style**（`alun-macros/src/route.rs:114-147`）：

```rust,no_run
use alun::prelude::*;

#[alun::permission(path = "/api/admin/users", method = "GET", permission = "admin:read")]
#[alun::get("/api/admin/users")]
async fn list_users() -> Res<Vec<JsonValue>> { Res::ok(vec![]) }
```

宏只接受 `path` / `method` / `permission` 三个键（其他键忽略，缺失的键变成空字符串 →
该规则无法匹配任何请求）。它把 `PermissionDef { path, method, permission }` 放进
`PERMISSION_ROUTES` 分布式切片，启动时由 `App` 合并进 `PermissionCheckLayer`
（`alun-web/src/app.rs:739-778`、`alun-web/src/middleware/permission_check.rs:71-80`）。

> **宏规则不受 `enabled` 影响**：只要存在宏注解规则，权限校验层必然挂载；
> `[middleware.permission] enabled = false` 只关闭配置文件里的 `rules`
> （`alun-web/src/app.rs:755-778`、`alun-config/src/lib.rs:439-448`）。

**自省接口**：

| 名称 | 类型 | 说明 |
|------|------|------|
| `alun::discovered_permissions() -> Vec<PermissionDef>` | inventory 收集 | 所有 `#[alun::permission]` 声明的宏规则（不读配置文件） |
| `alun::discovered_no_auth()` | inventory 收集 | 所有 `#[alun::no_auth]` 声明的免鉴权路径 |
| `alun::discovered_routes()` | `impl Iterator<Item = fn(&mut AlunRouter)>` | `#[alun::get]`… 收集到的路由注册项 |

（旧的 `PERMISSION_ROUTES` / `NO_AUTH_ROUTES` **常量**早已不存在 —— 0.3 起收集项用
`inventory`，自省走上面三个函数：`alun-web/src/lib.rs:102-130`。）

**配置里的路径级规则**：

```toml
[middleware.permission]
enabled = true          # 默认 false：只关配置规则，不关宏规则

[[middleware.permission.rules]]
path = "/api/admin"
methods = ["GET", "POST", "PUT", "DELETE"]   # 空数组 = 所有方法
permission = "admin:access"
```

匹配规则（`alun-web/src/middleware/permission_check.rs:17-33`）：路径按 `/` 分段逐段比对，
`{param}` 占位符匹配任意非空段，**段数必须一致**——因此 `path = "/api/admin"` 只匹配
`/api/admin` 本身，**不会**覆盖 `/api/admin/users`；要覆盖子路径就在规则里写出对应段或
`{id}` 占位段。`methods` 为空表示所有方法；多条规则命中时取**第一条**
（`permission_check.rs:125-131`）。`AuthClaims` 缺失时直接判定无权限（403）
（`permission_check.rs:133-148`）。

> 已知文档/实现不一致（能力缺口）：`alun-config` 中 `PermissionRule.path` 的注释写的是
> “支持前缀匹配”，但实现走的是上面的分段精确匹配（`alun-config/src/lib.rs:455-457`）。

## `#[no_auth]` Macro

标记无需认证的端点：

```rust,no_run
use alun::prelude::*;

#[alun::no_auth("/api/public")]
#[alun::get("/api/public")]
async fn public_api() -> Res<String> { Res::ok("ok".into()) }
```

语义（`alun-macros/src/route.rs:154-175`、`alun-web/src/app.rs:785-791`、
`alun-web/src/middleware/auth.rs:97-156`）：

- 宏把路径放进 `NO_AUTH_ROUTES`；启动时与 `ignore_paths` 合并，并拼接 router prefix，
  再交给 `AuthLayer`。
- 参数必须与真实注册的路由路径一致（`/api/public` 而非 `/api/public/*`）；
  与 `#[alun::get(...)]` 组合时两个宏各自独立注册，顺序无关。
- 免认证路径上若携带**有效** Token，仍会解析并注入 `UserId` / `AuthClaims`；
  Token 无效则忽略错误放行（不注入）。
- 免认证路径同样受 `OPTIONS` 预检放行逻辑影响。

## Middleware Stack

全部中间件由 `config.toml` 驱动，无需业务代码手工挂载。请求进入顺序（**由外到内**，
按 `alun-web/src/app.rs:682-855` 的 `.layer()` 装配顺序反推）：

1. **SecurityHeaders** — 注入 6 个安全头（`nosniff` / `X-Frame-Options` / HSTS / CSP /
   Referrer-Policy / Permissions-Policy），默认全开
2. **CORS** — 条件
3. **RequestLog** — 条件
4. **RequestId** — 条件
5. **AuthLayer** — 条件（JWT 验证 + 黑名单检查；`token_type` 必须是 `Access`）
6. **PermissionCheck** — 条件（路径规则 + 宏规则）
7. **RateLimit** — 条件（IP 滑动窗口）
8. **Compression** — 条件
9. **Nonce** — 条件（需 `cache` feature）
10. **Idempotency** — 条件（需 `cache` feature）

安全头/CORS 在最外层，因此 **401/403/500 响应也带安全头与 CORS 头**；Nonce/Idempotency
在最内层（压缩之内，避免缓存到已压缩的响应体；认证之内，避免被未认证请求探测）
（`alun-web/src/app.rs:692-699`）。

### Configuration Example

```toml
[middleware]
request_id = true
request_log = true

[middleware.request_log_config]
exclude_paths = ["/health"]
log_duration = true

[middleware.security_headers]
enabled = true

[middleware.cors]
enabled = true
allow_origins = ["http://localhost:3000"]
allow_methods = ["GET", "POST", "PUT", "DELETE"]
allow_headers = ["Content-Type", "Authorization"]
allow_credentials = true

[middleware.compression]
enabled = true

[middleware.rate_limit]
enabled = true
requests_per_window = 100
window_secs = 60

[middleware.nonce]
enabled = true
ttl_secs = 300

[middleware.idempotency]
enabled = true
ttl_secs = 86400
```

### Method-Level Security Middleware

**NonceLayer**（重放防护，需 `cache` feature）：客户端携带 `X-Nonce` 请求头，服务端校验后
立即消费（一次有效）；重复 nonce → **409 Conflict**。请求头缺失则不校验、直接放行
（`alun-web/src/middleware/nonce.rs:95-129`）。

**IdempotencyLayer**（订单/支付等写操作，需 `cache` feature）：客户端携带
`x-idempotency-key`，同 key 的首次响应被缓存（状态码 + Content-Type + body），后续相同 key
直接回放缓存响应；请求头缺失则原样透传
（`alun-web/src/middleware/idempotency.rs:106-183`）。

### Custom Middleware Injection

**先选位置，再写代码** —— 位置决定这一层能看到什么：

```rust,no_run
use alun::prelude::*;

# async fn demo() -> Result<()> {
App::new()?
    // 链外层：整站最外层。能记到 401/403/429 与真实耗时，但【读不到身份】
    .layer_from_fn(|router| {
        router.layer(axum::middleware::from_fn(my_custom_middleware))
    })
    // 链内层：位于 Auth / 权限之内。【能】提取 AuthClaims / Principal
    .route_layer(axum::middleware::from_fn(data_scope_guard))
    .scan()
    .start()
    .await
# }
# async fn my_custom_middleware(
#     req: axum::http::Request<axum::body::Body>,
#     next: axum::middleware::Next,
# ) -> axum::response::Response {
#     next.run(req).await
# }
# async fn data_scope_guard(
#     req: axum::http::Request<axum::body::Body>,
#     next: axum::middleware::Next,
# ) -> axum::response::Response {
#     next.run(req).await
# }
```

- `App::layer_from_fn` 接收 `FnOnce(Router) -> Router`，在默认中间件链与 404/静态文件
  fallback **之后**执行 ⇒ 该层在最外层，**此时认证还没跑，`AuthClaims` 不在请求扩展里**；
- `App::route_layer(L)` 在建链**之前**应用 ⇒ 该层落在链内，**能**读到身份；
  代价是只覆盖匹配到的路由（不含 404 与 `/metrics`）；
- 若只是**想在 handler 里拿身份**，根本不用中间件 —— 直接写提取器参数
  （`claims: AuthClaims` / `p: Principal` / `Option<AuthClaims>`）即可，零顺序依赖。

各挂载点的完整对照见 [`references/extension-points.md`](extension-points.md) 的第 2、3 节。

## Request Validation（`ValidatedJson` + `validator`）

`ValidatedJson<T>` 是自动 JSON 解析 + 字段级校验的提取器，要求
`T: DeserializeOwned + validator::Validate`（`alun-web/src/extract.rs:43-79`）：

- JSON 解析失败 → **400**，消息 `请求体格式错误: …`；
- 反序列化成功但 `validate()` 失败 → **422**，body 为格式化的校验错误列表；
- 实现 `Deref` / `DerefMut`，可直接访问内部 `T`；
- 另提供 `.validate()`（自行调用）与 `ValidateExt::validate_or_reject()`（失败返回 422 `ApiError`）。

```rust,no_run
use alun::prelude::*;
use alun::{validate_email, validate_mobile, ValidateExt};
use serde::Deserialize;
use validator::Validate;

#[derive(Debug, Deserialize, Validate)]
struct SignupReq {
    #[validate(length(min = 2, max = 50))]
    username: String,
    #[validate(custom(function = "validate_email"))]
    email: String,
    #[validate(custom(function = "validate_mobile"))]
    mobile: String,
}

#[alun::post("/api/signup")]
async fn signup(ValidatedJson(req): ValidatedJson<SignupReq>) -> ResResult<String> {
    req.validate_or_reject()?;   // 提取阶段已校验；显式再校验一次（可选）
    Ok(Res::ok(req.username.clone()))
}
```

9 个 `validate_*` 函数签名统一为 `fn(&str) -> Result<(), validator::ValidationError>`，
可直接用于 `#[validate(custom(function = "…"))]`（也可写全路径
`function = "alun::validate_email"`）。从 `alun` 根导出（不在 prelude 里，需显式 `use`）
（`alun/src/lib.rs:1155`、`alun-web/src/extract.rs:120-306`）：

| 函数 | 校验内容 | 空字符串 |
|------|----------|----------|
| `validate_uuid` | UUID v1~v7 | 跳过（`Ok`） |
| `validate_mobile` | 中国大陆手机号/固话 | 跳过 |
| `validate_password_strength` | ≥8 位，含大小写字母、数字、特殊字符 | **不跳过，报错** |
| `validate_id_card` | 中国大陆身份证号（含校验位） | 跳过 |
| `validate_date` | `YYYY-MM-DD` | 跳过 |
| `validate_datetime` | ISO 8601 / RFC 3339 | 跳过 |
| `validate_date_or_datetime` | 日期或日期时间 | 跳过 |
| `validate_email` | 邮箱格式 | 跳过 |
| `validate_url` | URL 格式 | 跳过 |

> `validator` **不**由 `alun` / `alun-web` 再导出，业务工程需自行声明 `validator` 依赖
> （版本与工作区一致：0.21）。校验函数与 `ValidatedJson` 的更多示例见
> `references/routing.md`。

## 租户 / 权限作用域边界

框架的鉴权信息**全部来自 JWT claims**（`sub` / `roles` / `permissions`），只提供三类判定：

1. Token 级：`TokenClaims::has_role` / `has_permission` / `is_super_admin`；
2. 路由级：`with_permission` / `with_role` 与 `#[alun::permission]`（路径 + 方法匹配）；
3. 路径白名单：`#[alun::no_auth]` / `ignore_paths`。

**框架不提供多租户/数据作用域能力**：`alun-web`、`alun-macros`、`alun-config`、`alun` 中
没有任何 `tenant` / `tenant_id` / `scope` 相关类型、中间件钩子或 `#[alun::tenant_scope]`
宏（全仓库检索无匹配）。需要按租户/组织隔离数据时，由业务代码基于
`AuthClaims`（或自建 claims 字段）在查询层自行过滤。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [进阶 DB（联查 / 白名单 / 租户安全 / 多数据源）](advanced-db.md) | [配置系统](config.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

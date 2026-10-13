# 路由与响应写法

## 四种写法，按推荐度从高到低

### Style 0（默认）· 宏注解 + `App::scan()`

**零注册代码**：`scan()` 会把整个 crate 里所有带 `#[alun::get]` / `#[alun::post]` /
`#[alun::put]` / `#[alun::patch]` / `#[alun::delete]` 的函数收起来，路由分散在几个文件都一样，
编译期经 `inventory` 收集，无运行时反射：

> 收集机制说明：0.3 已把早期的 `linkme` 换成 `inventory`（不依赖链接器自定义段，
> 不受 `--gc-sections` 影响）。所以 **`scan()` 是可靠的默认通道**，不需要为了绕开工具链
> 而改用显式注册 —— 详见文末 Style D。

```rust,no_run
use alun::prelude::*;
use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Serialize)]
struct UserModel { id: String }

#[derive(Deserialize, Validate)]
struct CreateUserReq {
    #[validate(length(min = 1, max = 50))]
    name: String,
}

#[alun::get("/api/users")]
async fn list_users() -> Res<Vec<UserModel>> {
    Res::ok(vec![])
}

#[alun::patch("/api/users/{id}")]         // 部分更新：与 put 同语义，仅 HTTP 方法不同
async fn update_user() -> Res<()> { Res::ok_empty() }

#[alun::post("/api/users")]
async fn create_user(ValidatedJson(req): ValidatedJson<CreateUserReq>) -> ResResult<UserModel> {
    Ok(Res::ok(UserModel { id: req.name }))
}

// main.rs:
# async fn demo() -> Result<()> {
App::new()?.scan().start().await
# }
```

### Style D（可选通道）· `#[alun::routes]` 模块级显式注册

**可选通道，不是默认**。它不依赖任何编译期收集机制 —— 宏在展开期扫描模块、
直接生成一个普通注册函数，纯 Rust 调用，因此在**分发带接口的库 / 插件**时是唯一可行通道
（编译期收集发生在最终二进制里，依赖方拿不到库里的 handler）。
另外需要自己完全掌控装配顺序、希望 IDE 能一键跳到 register 时也可以用它。

⚠️ 别把它当默认：项目自身的业务 handler 用 `#[alun::get]` + `.scan()` 就够了。

```rust,no_run
use alun::prelude::*;

#[alun::routes]
mod api {
    use alun::prelude::*;

    #[alun::get("/users")]
    pub async fn list_users() -> Res<String> {
        Res::ok("[]".into())
    }

    /// 权限规则随模块一起注册（不依赖切片，因此不会失效）
    #[alun::permission(path = "/users", method = "POST", permission = "user:create")]
    #[alun::post("/users")]
    pub async fn create_user() -> Res<()> {
        Res::ok_empty()
    }

    /// 免认证路径同理
    #[alun::no_auth("/api/health")]
    #[alun::get("/health")]
    pub async fn health() -> Res<String> {
        Res::ok("ok".into())
    }
}

# async fn demo() -> Result<()> {
App::new()?
    .module(api::register)   // ← 由宏生成，一行挂载
    .start()
    .await
# }
```

要点：

- 模块内的 `#[alun::get]` / `#[alun::permission]` / `#[alun::no_auth]` 会被**剥离**，
  不再展开成 `distributed_slice`；它们在此上下文中只是声明性元数据。
- 生成的函数签名是 `pub fn register(app: App) -> App`，因此可与其他 `App` 方法链式混用。
- 也可用 `App::routes(fn(&mut AlunRouter))` 做最底层的批量注册（可放在循环/条件里）。
- 测试时用 `App::into_router()` 直接拿 `Router`，无需绑端口（配 `tower::ServiceExt::oneshot`）。
### Style C（可选）· Controller 分组（适合大项目）

`#[alun::controller("/前缀")]` 标注在 **`impl` 块**上，块内方法用 `#[alun::get]` 等标注；宏会摘除这些内部属性，并在 impl 之外生成路由注册（路径 = `前缀` + 方法路径，handler 为 `类型::方法名`；只有 **async** 且带路由宏的方法会被注册）：

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

## `App` 方法总览

| 方法 | 签名要点 |
|------|---------|
| `App::new()` | = `App::from_config_dir("config")`；读 `config/` 目录配置并初始化日志，返回 `Result<App>`（DB / 缓存 / 模板等资源在 `start()` 里才初始化） |
| `App::from_config()` / `App::from_config_dir(dir)` | 显式目录（`from_config()` 等价 `new()`） |
| `App::with_config(cfg)` / `App::with_config_manager(cm)` | 直接注入配置（跳过文件加载，常用于测试） |
| `.parse_cli()` | 解析 `gen-config` / `print-config` / `profile=xxx` 等命令行参数（须在 `start()` 前调用，否则 `cargo run -- gen-config` 不生效） |
| `.get/.post/.put/.patch/.delete(path, handler)` | 注册单个路由 |
| `.route(method, path, handler)` | 任意方法（字符串；`GET/POST/PUT/PATCH/DELETE` 之外回退 `any`） |
| `.group(prefix, \|app\| ...)` | 前缀分组 |
| `.merge(prefix, sub: AlunRouter)` | 合并子路由 |
| `.scan()` | 收集宏注册的处理器（`ROUTES` 分布式切片） |
| `.with_permission(method, path, handler, permission)` | 方法级权限守卫 |
| `.with_role(method, path, handler, role)` | 方法级角色守卫 |
| `.plugin(plugin)` | 注册插件（按依赖拓扑排序启动） |
| `.on_startup(hook)` / `.layer(L)` / `.layer_from_fn(f)` / `.route_layer(L)` | 启动钩子（资源初始化之后、插件启动之前）；自定义中间件共 5 个挂载点，**位置决定能否读身份**（见 [extension-points](extension-points.md) 第 2、3 节） |
| `.start()` | 读配置、初始化全局资源、启动插件与服务器（**无端口参数**；端口取自 `[server].listen`） |
| `.serve(addr)` | 覆盖 `[server].listen` 后用显式地址启动，如 `"8080"` / `"0.0.0.0:8080"` |

没有 `App::with_state` —— 资源通过全局访问器（`db()` / `cache()` / `cfg()` / `config()`）获取。

注意 `[router].prefix`：当前实现里只用于给中间件的路径列表（`[middleware.auth].ignore_paths`、日志 `exclude_paths`、`#[no_auth]` 收集的路径）拼前缀做匹配，**不会**改写已注册路由的路径；要加统一前缀用 `.group(prefix, ...)` 或 `.merge(prefix, sub)`。

## Sub-Router Merging

`AlunRouter` 的注册方法是 `add_get` / `add_post` / `add_put` / `add_patch` / `add_delete` / `add_route`（带中间件版本为 `add_get_with_layer` … `add_patch_with_layer` 等，`wrap` 闭包接收 `MethodRouter` 以便 `.route_layer(...)`），另有 `merge(prefix, sub)`、`merge_with_layer(prefix, sub, wrap)`（只给该前缀下的整组路由挂层）与 `into_axum()`：

```rust,no_run
use alun::prelude::*;

fn user_routes() -> AlunRouter {
    let mut r = AlunRouter::new();
    r.add_get("/", list_users);
    r.add_post("/", create_user);
    r
}

# async fn demo() -> Result<()> {
App::new()?
    .merge("/api/users", user_routes())
    .start()
    .await
# }
# async fn list_users() -> Res<String> { Res::ok("".into()) }
# async fn create_user() -> Res<String> { Res::ok("".into()) }
```

### 给路由挂自定义 layer（5 个档位）

不改配置、要往请求管线里插自己一层时，**先选位置** —— 位置决定这一层能否读到当前身份
（完整对照见 [extension-points](extension-points.md) 第 2、3 节）。

```rust,no_run
use alun::prelude::*;

// ① 路由级（属性宏）：写在路由注解【上方】。
//    宏会摘掉该路由的编译期自注册、改生成「带 layer 的注册」，因此不会重复挂载。
#[alun::layer(axum::middleware::from_fn(require_tenant))]
#[alun::get("/orders")]
async fn list_orders(claims: AuthClaims) -> Res<String> {
    Res::ok(claims.0.sub.clone())
}

// ② 子树级：只给 /admin 这一片挂层。
//    merge_with_layer 是 AlunRouter 的方法，经 App::routes 拿到 &mut AlunRouter。
# async fn demo() -> Result<()> {
App::new()?
    .routes(register_admin)
    .scan()
    .start()
    .await
# }
# fn register_admin(r: &mut AlunRouter) {
#     r.merge_with_layer("/admin", admin_routes(), |sub| {
#         sub.route_layer(axum::middleware::from_fn(audit_guard))
#     });
# }
# fn admin_routes() -> AlunRouter {
#     let mut r = AlunRouter::new();
#     r.add_get("/ping", || async { Res::ok("pong".to_string()) });
#     r
# }
# async fn require_tenant(
#     req: axum::http::Request<axum::body::Body>,
#     next: axum::middleware::Next,
# ) -> axum::response::Response {
#     next.run(req).await
# }
# async fn audit_guard(
#     req: axum::http::Request<axum::body::Body>,
#     next: axum::middleware::Next,
# ) -> axum::response::Response {
#     next.run(req).await
# }
```

| 档位 | 写法 | 覆盖范围 | 能读身份 |
| ---- | ---- | ---- | ---- |
| 链外全局 | `App::layer(L)` / `App::layer_from_fn(f)` | 整站（含 401/403/429、404、`/metrics`） | ✗ |
| 链内全局 | `App::route_layer(L)` | 仅匹配到的路由 | ✓ |
| 子树 | `AlunRouter::merge_with_layer(prefix, sub, wrap)` | `prefix` 下的整组 | ✓ |
| 单路由（宏） | `#[alun::layer(L)]` 写在路由注解上方 | 那一条 | ✓ |
| 单路由（代码） | `AlunRouter::add_get_with_layer(path, h, wrap)` | 那一条 | ✓ |

> 一句话：**审计 / 访问日志留在最外层 `layer`**（否则漏记 401 / 403 / 429、耗时也不准）；
> **要用「当前是谁」就用 `route_layer` 及更细的档位**；只是想在 handler 里拿身份则根本不用中间件，
> 直接写提取器参数即可。

## 提取请求参数

Use standard axum extractors (`Path` / `Query` / `Extension` 均在 `alun::prelude` 中）：

```rust,no_run
use alun::prelude::*;
use std::collections::HashMap;

// 路径参数
#[alun::get("/users/{id}")]
async fn get_user(Path(id): Path<String>) -> Res<JsonValue> {
    Res::ok(JsonValue::String(id))
}

// 查询参数
#[alun::get("/users")]
async fn search(Query(params): Query<HashMap<String, String>>) -> Res<JsonValue> {
    Res::ok(json!({ "count": params.len() }))
}

// 当前认证用户（AuthClaims 是包装 TokenClaims 的元组结构体）
#[alun::get("/me")]
async fn me(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<JsonValue> {
    Res::ok(json!({ "user_id": claims.sub, "roles": claims.roles }))
}
```

## 校验请求体：`ValidatedJson`

Auto-deserializes JSON + validates (失败在处理器执行前返回 422)：

```rust,no_run
use alun::prelude::*;
use serde::Deserialize;
use validator::Validate;

#[derive(Debug, Deserialize, Validate)]
struct CreateUserReq {
    #[validate(length(min = 1, max = 50, message = "名称需 1-50 个字符"))]
    name: String,
    #[validate(email(message = "邮箱格式不正确"))]
    email: String,
}

#[alun::post("/api/users")]
async fn create(ValidatedJson(req): ValidatedJson<CreateUserReq>) -> ResResult<String> {
    // req 已自动校验；ValidatedJson<T> 通过 Deref 访问 T
    Ok(Res::ok(req.name.clone()))
}
```

### Custom Validator Functions

Alun provides pre-built validator functions for common field types. Use with `#[validate(custom(function = "..."))]`:

| Function | Description | Empty field |
|----------|-------------|-------------|
| `validate_uuid` | UUID v1~v7 format | Skips |
| `validate_mobile` | China mobile/landline | Skips |
| `validate_password_strength` | 8+ chars, upper+lower+digit+special | **Always** |
| `validate_id_card` | China 18-digit ID (with check digit) | Skips |
| `validate_date` | YYYY-MM-DD format | Skips |
| `validate_datetime` | ISO 8601 / RFC 3339 | Skips |
| `validate_date_or_datetime` | YYYY-MM-DD **or** ISO 8601/RFC 3339 | Skips |
| `validate_email` | Email format | Skips |
| `validate_url` | HTTP/HTTPS URL | Skips |

```rust,no_run
use alun::prelude::*;
use alun::{validate_date_or_datetime, validate_uuid, ValidateExt};
use serde::Deserialize;
use validator::Validate;

#[derive(Debug, Deserialize, Validate)]
struct EventReq {
    #[validate(custom(function = "validate_uuid"))]
    pub id: String,

    #[validate(custom(function = "validate_date_or_datetime"))]
    pub release_date: Option<String>,  // 支持纯日期或完整时间戳
}

// 手动验证（ValidateExt 是 validator::Validate 的 blanket 扩展）
# fn demo(req: &EventReq) -> std::result::Result<(), ApiError> {
req.validate_or_reject()?;   // 失败返回 ApiError(422)
# Ok(())
# }
```

## 统一响应 `Res<T>`

All handlers return `Res<T>` (always success) or `Result<Res<T>, ApiError>` — 后者就是框架别名 `ResResult<T>`：

```rust,no_run
use alun::prelude::*;

# fn demo() {
// 成功
let a = Res::ok("data");                                // { code:0, msg:"ok", data:"data" }
let b = Res::ok_with_msg(1, "创建成功");                  // 自定义消息
let c: Res<()> = Res::ok_empty();                        // 无 data 字段
let d = Res::ok_msg("操作成功");                          // 仅消息

// 独立失败体（不通过 Result）
let e: Res<()> = Res::fail(codes::BAD_REQUEST, "用户名不能为空");

// 分页成功体：data = { list, total, page, page_size }
let f = Res::page(vec!["a"], 100, 1, 20);
# let _ = (a, b, c, d, e, f);
# }

# fn errors() {
// 错误响应（通过 Result）：ResResult<T> = Result<Res<T>, ApiError>
let a = ApiError::bad_request("参数错误");                 // 400
let b = ApiError::unauthorized("请先登录");                // 401
let c = ApiError::forbidden("权限不足");                   // 403
let d = ApiError::not_found("用户不存在");                 // 404
let e = ApiError::method_not_allowed("方法不允许");        // 405
let f = ApiError::conflict("用户名已存在");                // 409
let g = ApiError::unprocessable_entity("邮箱格式不正确");   // 422
let h = ApiError::too_many_requests("请求过于频繁");        // 429
let i = ApiError::internal("数据库连接失败");              // 500（前端看到"服务器内部错误"）
let j = ApiError::internal_masked("服务器内部错误", "detail");  // 自定义前端消息 + 内部详情
let k = ApiError::service_unavailable("服务暂不可用");      // 503
# let _ = (a, b, c, d, e, f, g, h, i, j, k);
# }
```

错误码常量在 `alun::prelude::codes`：`OK=0`、`BAD_REQUEST=400`、`UNAUTHORIZED=401`、`FORBIDDEN=403`、`NOT_FOUND=404`、`METHOD_NOT_ALLOWED=405`、`CONFLICT=409`、`UNPROCESSABLE_ENTITY=422`、`TOO_MANY_REQUESTS=429`、`INTERNAL=500`、`SERVICE_UNAVAILABLE=503`。

### 分页响应

分页参数是全框架唯一的 `alun_core::PageQuery`（`page` / `page_size`，作为 `Query` 提取器时自动规整）；数据库返回的 `Page<T>` 通过 `PageResponseExt::into_res()` 直接转成标准分页响应，无需手工搬运字段：

```rust,no_run
use alun::prelude::*;

#[derive(serde::Serialize)]
struct UserRes { id: String, name: String }

#[alun::get("/api/users")]
async fn list_users(Query(pq): Query<PageQuery>) -> ResResult<PageData<Vec<UserRes>>> {
    // `ResResult` 收得下 `DbError`（`From<DbError> for ApiError`），`?` 直接用
    let page = db()
        .paginate("SELECT * FROM users ORDER BY id", pq.page, pq.page_size)
        .await?;

    // Row 不实现 Serialize：先映射成 DTO
    let page = page.map(|r| UserRes {
        id: r.get_typed::<String>("id").unwrap_or_default(),
        name: r.get_typed::<String>("name").unwrap_or_default(),
    });

    Ok(page.into_res())   // data = { list, total, page, page_size }
}
```

也可用 `PageQuery::new(1, 20)` 手工构造；`pq.offset()` / `pq.limit()` / `pq.calc_pages(total)` 均对任意取值安全。

## 权限守卫（`#[alun::permission]` / `#[alun::no_auth]`）

认证配置与 `JWT` 见 `references/auth.md`。路由侧有两条路径：构建器方法，或叠加在路由宏上的两个宏。

```rust,no_run
use alun::prelude::*;

# async fn demo() -> Result<()> {
// 构建器方式：权限 / 角色
App::new()?
    .with_permission("GET", "/api/admin/stats", admin_stats, "admin:access")
    .with_role("DELETE", "/api/users/{id}", delete_user, "admin")
    .start()
    .await
# }

// 宏方式：#\[alun::permission\] 与路由宏叠加，命中时校验 claims 的权限
#[alun::permission(path = "/api/admin/users", method = "GET", permission = "admin:read")]
#[alun::get("/api/admin/users")]
async fn list_admin_users() -> Res<Vec<String>> {
    Res::ok(vec![])
}

// 免认证路径（支持前缀匹配）
#[alun::no_auth("/api/public")]
#[alun::get("/api/public")]
async fn public_api() -> Res<String> {
    Res::ok("免认证".into())
}

# async fn admin_stats() -> Res<String> { Res::ok("".into()) }
# async fn delete_user() -> ResResult<()> { Ok(Res::ok_empty()) }
```

`#[alun::no_auth("/path")]` 会被收集进 `NO_AUTH_ROUTES` 分片，与 `[middleware.auth] ignore_paths` 的精确/前缀匹配共同决定免认证集合。

## Handler 里怎么报错

Never use `unwrap()`/`expect()` in handler code:

```rust,no_run
use alun::prelude::*;

#[derive(serde::Serialize)]
struct UserModel { id: String, name: String }

#[alun::get("/api/users/{id}")]
async fn find_user(Path(id): Path<String>) -> ResResult<UserModel> {
    if id.is_empty() {
        return Err(ApiError::bad_request("ID 不能为空"));
    }

    // 动态 Row 按主键查询是 find_row_by_pk(*表*, *主键列*, *主键值*)
    let row = db()
        .find_row_by_pk("users", "id", id)
        .await?
        .ok_or_else(|| ApiError::not_found("用户不存在"))?;

    Ok(Res::ok(UserModel {
        id: row.get_typed::<String>("id").unwrap_or_default(),
        name: row.get_typed::<String>("name").unwrap_or_default(),
    }))
}
```

## Production Error Masking

```rust,no_run
use alun::prelude::*;
use std::result::Result;

# async fn do_something() -> std::result::Result<(), String> { Ok(()) }

#[alun::post("/api/risky")]
async fn risky_operation() -> Result<Res<()>, ApiError> {
    do_something().await.map_err(|e| {
        tracing::error!("内部错误: {:?}", e);
        ApiError::internal_masked("服务器内部错误", format!("{:?}", e))
    })?;
    Ok(Res::ok_empty())
}
```

`ApiError::internal(msg)` 与 `internal_masked(public, detail)` 都会把细节只写进日志，前端只看到 `"服务器内部错误"`（或 `public` 参数）。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [返回总目录](../SKILL.md) | [数据库 CRUD / 事务 / 迁移](database.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

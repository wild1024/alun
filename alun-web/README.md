# alun-web

Alun 的 Web 层：应用装配、路由、中间件、提取器、统一响应、JWT 认证、全局资源、SSE、可观测。

> 门面 `alun` 会把本 crate 再导出为 `alun::web`（`App` / `AlunRouter` / `JWT` / 中间件等都在
> 其中；`prelude` 不含中间件层，需要时写全路径）。

## 主要 API

### 应用装配 `App`

- 构造：`new()`（= `from_config_dir("config")`）、`from_config()`、`from_config_dir(dir)`、
  `with_config(cfg)`、`with_config_manager(Arc<ConfigManager>)`、`parse_cli()`
- 路由：`get` / `post` / `put` / `patch` / `delete` / `route(method, path, handler)`、`group(prefix, f)`、
  `merge(prefix, sub)`、`module(f)`、`routes(f)`、`scan()`、`into_router()`
- 鉴权：`with_permission(method, path, handler, permission)`、`with_role(method, path, handler, role)`
- 生命周期：`plugin(p)`、`web_plugin(p)`（贡献路由 / 权限 / 免鉴权）、`on_startup(hook)`、
  `layer(L)` / `layer_from_fn(f)`（链外，覆盖整站但读不到身份）、`route_layer(L)`（链内，可读身份）
- 启动：`start()`（端口取自配置）、`serve(addr)`
- 扩展点：`permission_defs(v)` / `no_auth_defs(v)` / `routes(f)` / `module(f)`

### 路由 `AlunRouter`

`new()`、`add_get` / `add_post` / `add_put` / `add_patch` / `add_delete` / `add_route`、
`add_*_with_layer`（给单个路由额外挂 layer）、`merge(prefix, sub)`、
`merge_with_layer(prefix, sub, wrap)`（给某前缀下的整组路由挂 layer）、`into_axum()`

### 认证鉴权

- `JWT` —— `from_config()`、`with_config(Arc<ConfigManager>)`、`with_config_and_cache(cm, cache)`（feature `cache`）、
  `create_access_token(user_id, username, roles, permissions)`、`create_refresh_token(user_id)`、
  `issue ClaimsBuilder`（带自定义声明）、`validate(token) -> Result<TokenClaims, String>`、
  `jwt_secret()`、`access_token_expire_secs()`、`refresh_token_expire_secs()`；
  开 `cache` 后另有 `blacklist` / `is_blacklisted` / `logout` / `refresh`
- `TokenClaims` —— `jti` / `sub` / `username` / `roles` / `permissions` / `token_type` / `exp` / `iat`、
  扩展位 `extra`，以及 `user_id()`、`has_role()`、`has_any_role()`、`has_all_roles()`、
  `has_permission()`、`has_any_permission()`、`is_super_admin()`（`*` / `*:*:*` 通配）
- `TokenType`（`Access` / `Refresh`）、`UserId(pub String)` —— `sub` 的**原样标识**，提供
  `as_str()` / `Display` / `AsRef<str>`，需要强类型时 `parse::<T>()`（如 `parse::<Uuid>()`、
  `parse::<i64>()`；失败得 `None`，身份不丢）
- `AuthClaims(pub TokenClaims)` —— **提取器**（双通道：请求扩展 → 回退验签 + 黑名单），
  旧的 `Extension(AuthClaims(claims))` 写法仍可用
- `Principal` —— 身份业务视图：`user_id: UserId` / `username` / `roles` / `claims`，
  以及 `has_role()` / `has_permission()` 等（**不含** `is_super_admin()`）；
  `Option<AuthClaims>` / `Option<Principal>` 表示「可选身份」（无 token 得 `None`，不报 401）
- 中间件层：`AuthLayer`、`RequireRoleLayer::any(roles)`、`RequirePermissionLayer::any(permissions)`、
  `PermissionCheckLayer::from_config(rules)` / `with_macro_rules(defs)`、`RequestIdLayer`、
  `RequestLogLayer`、`RateLimitLayer`、`SecurityHeadersLayer`，以及 `NonceLayer` / `IdempotencyLayer`
  （feature `cache`）
  - `RequestIdLayer` 同时把同一个 `x-request-id` 写进请求上下文**和**响应头
  - `NonceLayer` / `IdempotencyLayer` **不需要手挂**：写了 `[middleware.nonce]` /
    `[middleware.idempotency]`（`enabled` / `ttl_secs`）就会自动进默认链
  - `TenantLayer`（租户上下文注入）：`[tenant].enabled = true` 时由 `App` 自己挂，
    且必须比 `AuthLayer` **更内层**——先验签、再信任载荷里的租户 claim

### 提取器与校验

- `ValidatedJson<T>` —— 在进 handler 前先跑 `validator::Validate`；`ValidatedJson::validate()` 可手动触发
- `ValidateExt` —— `validate_or_reject() -> Result<(), ApiError>`（`validator::Validate` 的 blanket 实现）
- `validate_uuid` / `validate_mobile` / `validate_password_strength` / `validate_id_card` / `validate_date` /
  `validate_email` / `validate_url` / `validate_datetime` / `validate_date_or_datetime` —— 自定义
  `validator` 函数（空输入默认跳过，除 `validate_password_strength`）

### 全局资源（不用 axum `State` 注入）

- 配置：`cfg()`（`&AppConfig`）、`config()` / `try_config()` / `set_config()`（`Arc<ConfigManager>`）
- 数据库：`db()` / `try_db()` / `set_db()`（feature `db`），多数据源 `db_named(name)` / `set_db_named(name, db)`
- 缓存：`cache()` / `try_cache()` / `set_cache()`（feature `cache`），租户视图 `cache_tenant()`
- 模板：`render_template(name, ctx)` / `try_template()` / `set_template(engine)`（feature `template`）
- 路径：`upload_path()` / `set_upload_path()` / `download_path()` / `set_download_path()` 及各自的 `try_*`
- 实时推送：`Broadcaster` / `sse_json`（feature `sse`）；可观测：`metrics`（feature `metrics`）

### 单一出口：handler 只有 `ResResult<T>`

响应形状类型（`Res` / `ApiError` / `ResResult` / `PageQuery` / `PageData` / `codes`）
住在 `alun-web`，因此 `ApiError` 是**本 crate 的本地类型**——`From<任意外部错误>`
都合法，下层错误可以直接 `?` 进 handler：

- `ResResult<T> = Result<Res<T>, ApiError>` —— handler 的**唯一**返回类型
- 内置 `From`：`DbError`（feature `db`）、`alun_core::Error`、`std::io::Error`
- `DbError` → 状态码：`NotFound`→404、`UniqueViolation` / `ForeignKeyViolation` /
  `CheckViolation`→409、`Deadlock` / `SerializationFailure` / `LockTimeout`→500
  「服务繁忙，请重试」、其余→500 脱敏；原文（含 SQL）只进 `internal_detail` 与日志
- 反向通道 `From<ApiError> for alun_core::Error` 也在本 crate，供启动 / 插件 /
  定时任务这些非 HTTP 流程复用同一份文案

**为什么这些类型不在 `alun-core`**：它们满身 HTTP 概念（`status`、`404`…），
而 `alun-core` 定位是「可被非 Web 项目复用的骨架层」。更实际的是孤儿规则——
`From<DbError> for ApiError` 只能写在拥有 `DbError`（`alun-db-core`）或拥有
`ApiError` 的 crate 里；`ApiError` 下沉之后，这条 impl 落在 `alun-web` 天然合法，
既不必让核心依赖数据库内核，也不必再新造一个 `WebError` 当落点。

### 分页桥接（feature `db`）

`PageResponseExt` —— 为 `alun_db::Page<T>` 实现：`into_page_data() -> PageData<Vec<T>>` 与
`into_res() -> Res<PageData<Vec<T>>>`（把 `Page::size` 映射成 `PageData::page_size`，丢弃 `Page::pages`）。

## Feature

| Feature | 启用内容 |
|---------|---------|
| `config` | 基于配置的应用构造 |
| `db` | `alun-db`、`resources::{db, try_db, set_db}`、`PageResponseExt`（隐含 `config`） |
| `cache` | `alun-cache`、`resources::{cache, try_cache, set_cache}`、JWT 黑名单 / 刷新、`NonceLayer`、`IdempotencyLayer` |
| `template` | `alun-template`、`resources::{render_template, try_template, set_template}` |
| `sse` | `alun_web::sse::{Broadcaster, sse_json}` |
| `metrics` | `metrics` 门面（HTTP 自动埋点）；`metrics-prometheus` 额外装 recorder 并挂 `/metrics` |

## 示例

最小服务（合并子路由 + 宏注册）：

```rust,ignore
use alun_macros::get;
use alun_web::{AlunRouter, App, Res};

#[get("/health")]
async fn health() -> Res<&'static str> {
    Res::ok("ok")
}

#[tokio::main]
async fn main() -> alun_core::Result<()> {
    let mut api = AlunRouter::new();
    api.add_get("/ping", || async { "pong" });

    App::new()?
        .merge("/api", api)
        .scan()
        .start()
        .await
}
```

数据库 + 分页接口：

```rust,ignore
use alun_web::{ApiError, PageData, PageQuery, PageResponseExt, Res, ResResult, db};
use axum::extract::Query;
use serde::Serialize;

#[derive(Serialize)]
struct UserRes { id: String, name: String }

async fn list_users(Query(pq): Query<PageQuery>) -> ResResult<PageData<Vec<UserRes>>> {
    let page = db()
        .paginate("SELECT * FROM users ORDER BY id", pq.page, pq.page_size)
        .await?
        .map(|row| UserRes {
            id: row.get_typed("id").unwrap_or_default(),
            name: row.get_typed("name").unwrap_or_default(),
        });

    Ok(page.into_res())
}
```

> ⚠️ 中间件顺序：`TenantLayer` 必须在 `AuthLayer` **之前**添加（即成为认证层的内层），
> 否则未验签的载荷会被当真、直接造成租户越权。`App` 已按此顺序装配，手写中间件时务必遵守。

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)
- 速查参考：`skill-alun/references/routing.md`、`skill-alun/references/auth.md`、
  `skill-alun/references/extension-points.md`、`skill-alun/references/realtime-sse.md`、
  `skill-alun/references/observability.md`

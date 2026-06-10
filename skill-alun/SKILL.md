---
name: alun
description: >-
  Help users develop web applications and services using the alun Rust web framework.
  alun is a configuration-driven, plugin-architected Rust web framework built on axum.
  Use this skill whenever the user writes, debugs, or asks questions about alun-based
  Rust projects — including routing, database CRUD, JWT authentication, middleware,
  async tasks, caching, file storage, Kafka integration, procedural macros, plugin
  lifecycle, and configuration management. Also use this skill when the user mentions
  specific alun crates (alun-core, alun-web, alun-db, alun-macros, alun-task,
  alun-cache, alun-config, alun-kafka, alun-fs, alun-plugin, alun-template, alun-utils)
  or alun-specific types/annotations like `App`, `Res`, `Db`, `Row`, `#[get]`,
  `#[post]`, `#[controller]`, `#[task_handler]`, `#[plugin]`, `#[permission]`.
---

# Alun Framework Skill

Alun is a **configuration-driven, plugin-architected Rust web framework** built on axum + tower + tokio. Its philosophy is "Just Config + Just Code" — behavior is controlled by `config.toml`, and business logic is expressed as plain async functions with proc-macro annotations.

## When To Use This Skill

Invoke this skill whenever the user:
- Writes new alun-based Rust web application code
- Debugs or asks questions about alun's API, types, macros, or patterns
- Needs help with alun's routing, database CRUD, JWT auth, middleware, plugins, or config
- Mentions any alun crate or type (e.g., `App`, `Res`, `Db`, `Row`, `#[get]`, `#[controller]`)

## Core Principles (Always Keep In Mind)

1. **Configuration-Driven**: All infrastructure behavior (DB, auth, CORS, rate limiting, compression) is declared in `config.toml` — never hardcoded in `main.rs`
2. **Global Resource Access**: Use global accessor functions (`db()`, `cache()`, `cfg()`, `render_template()`) instead of axum `State` injection. No `with_state()` needed.
3. **Zero-Cost Abstraction**: Pure Rust traits + generics, compiled at build time — no reflection, no dynamic dispatch
4. **Compiler-Enforced Safety**: Transaction rollback is guaranteed by Rust `Drop` + `?` operator. Error handling is type-safe.
5. **Progressive Enhancement**: Start from `App::new().get("/", h).serve("8080")` and layer on features as needed
6. **Security by Default**: Security headers auto-injected. 5xx errors masked from clients — details go only to logs.

## Project Architecture

The alun workspace consists of 14 crates:

| Layer | Crate | Purpose |
|-------|-------|---------|
| **Facade** | `alun` | Single user dependency. Re-exports via `alun::prelude::*` and proc macros. |
| **Core** | `alun-core` | Zero web-framework dependency. `Error`, `Result`, `Plugin`, `Res<T>`, `ApiError`, `PageQuery`. |
| **Features** | `alun-web` | `App` builder, router, middleware stack, JWT, global singletons, `ValidatedJson`. |
| | `alun-db` | `Db` facade, `Row` CRUD, RAII transactions, `Hook` lifecycle, SQL templates, migrations. |
| **Infrastructure** | `alun-config` | TOML config, multi-profile, `ALUN_*` env overrides, runtime dynamic config. |
| | `alun-log` | tracing init (text/json/file). |
| | `alun-macros` | Proc macros: `#[get]`, `#[post]`, `#[put]`, `#[delete]`, `#[controller]`, `#[plugin]`, etc. |
| | `alun-utils` | 200+ utilities: strings, dates, masking, IDs, validation, crypto, export, XSS, serial number generation. |
| **Extensions** | `alun-cache` | `Cache` trait, `LocalCache`, `SharedCache` (Local/Redis). |
| | `alun-template` | minijinja (Jinja2) template engine. |
| | `alun-plugin` | Built-in plugins: cache, notification (SMTP), async-task, scheduler (cron), serial number generator. |
| | `alun-kafka` | Kafka producer/consumer (rdkafka). |
| | `alun-task` | Kafka-driven distributed async task engine with retry/DLQ. |
| | `alun-fs` | Local file system abstraction. |

## Feature Flags

```toml
alun = { default-features = false, features = [] }              # Minimal web
alun = { default-features = false, features = ["db"] }          # + Database
alun = { default-features = false, features = ["db", "cache"] } # + Cache
alun = { default-features = false, features = ["db", "cache", "template"] }
alun = { default-features = false, features = ["task"] }        # Kafka tasks
alun = { default-features = false, features = ["xss"] }         # XSS sanitization
alun = { features = ["full"] }                                  # Everything (default)
```

## Quick Start

```rust
use alun::prelude::*;

#[alun::get("/")]
async fn hello() -> Res<String> {
    Res::ok("Hello, alun!".into())
}

#[tokio::main]
async fn main() {
    App::new()
        .expect("初始化失败")
        .scan()
        .start()
        .await
        .unwrap();
}
```

```bash
cargo run -- gen-config   # Generate default config
cargo run                 # Starts at http://127.0.0.1:8023
```

## Routing Patterns → see `references/routing.md`

Three styles:
- **Builder chaining**: `App::new()?.get("/api/users", h).serve("8080").await`
- **Proc macro + scan()** (recommended): `#[alun::get("/api/users")]` + `App::new()?.scan().start()`
- **Controller grouping**: `#[alun::controller("/api/admin")]` on an `impl` block

Use standard axum extractors (`Path`, `Query`, `Extension`, `ValidatedJson`).

## Unified Response (`Res<T>`) → see `references/routing.md`

All handlers return `Res<T>` or `Result<Res<T>, ApiError>`:
- `Res::ok(data)` → `{ code:0, msg:"ok", data }`
- `Res::page(list, total, page, size)` for pagination
- `Err(ApiError::bad_request("msg"))` / `.unauthorized()` / `.forbidden()` / `.not_found()` / `.internal_masked(public, detail)` etc.

## Database → see `references/database.md`

Requires `features = ["db"]`. Key patterns:
- `Row` CRUD: `Row::table("users").id(Sid::uuid7()).set("name", val)` → `db().insert(&row)`
- Raw SQL: `db().query(sql, &[])`, `db().query_page(sql, &[], &PageQuery::new(1,20))`
- RAII Transactions: `db().transaction(|tx| async move { ... Ok(()) }).await?`
- Hooks: `impl Hook` for CRUD lifecycle callbacks; built-in `TimestampHook`
- SQL Templates: Jinja2 syntax via `SqlTemplate`; Migrations: `Migrator::new(&pool, "migrations").run()`

## Authentication & Authorization → see `references/auth.md`

Configured in `[middleware.auth]`:
- `JWT::from_config()` for token management
- `Extension(AuthClaims(claims))` to access current user in handlers
- `claims.has_role()`, `claims.has_permission()` for RBAC checks
- `App::new()?.with_permission(...)` or `#[alun::permission(...)]` macro for guards
- `#[alun::no_auth("/path")]` to skip auth on specific endpoints

## Middleware Stack → see `references/auth.md`

Execution order: SecurityHeaders → RequestLog → RequestId → CORS → Compression → RateLimit → AuthLayer → PermissionCheck

## Configuration System → see `references/config.md`

1. `config/config.toml` → 2. `config-{profile}.toml` → 3. `ALUN_*` env vars
- `cfg().server.listen` for static config; `config().set_dynamic()` for runtime
- CLI: `cargo run -- gen-config`, `cargo run -- profile=prod`, `ALUN_SERVER_LISTEN=3000 cargo run`

## Plugins → see `references/plugins.md`

`#[async_trait] impl Plugin for MyPlugin { fn name(), start(), stop(), depends_on() }`
- Built-in: `CachePlugin`, `SchedulerPlugin` (cron), `NotificationPlugin` (SMTP), `AsyncTaskPlugin`, `SerialPlugin`
- Register: `App::new()?.plugin(MyPlugin).scan().start()`

## Caching & Templates → see `references/plugins.md`

Global: `cache().set_ex("k", "v", 300)`, `cache().get("k")`, `cache().keys("user:*")`
Template: `render_template("page.html", &json!({...}))` — Jinja2 syntax, files in `templates/`

## Utilities → see `references/utils.md`

200+ functions: string conversion (`to_snake()`, `to_camel()`), date formatting, data masking (`Mask::mobile()`), ID generation (`Sid::uuid7()`), validation (`Valid::is_email()`), crypto (`Crypto::hash_password()`), CSV/JSON export, XSS sanitization.

## Async Task Framework → see `references/task.md`

Requires `features = ["task"]`. Kafka-driven distributed tasks with retry/DLQ:
- `#[alun::task_handler(task_type=1, topic="...", max_retries=3, retry_strategy="Exponential")]`
- Implement `TaskHandler` + `TaskStorage` traits
- Submit via `TaskProducer::new().submit(SubmitTaskParams{...})`

## Recommended Project Structure

```
src/
├── main.rs          # App::new()?.scan().start()
├── controllers/     # #[alun::get/post] handlers
├── models/          # Data structs (*Model, *Req, *Res)
├── services/        # Business logic
└── plugins/         # Custom Plugin impls
config/config.toml
migrations/          # NNN_desc.up.sql / .down.sql
templates/           # .html Jinja2 files
uploads/ downloads/
```

## Quick Reference

| Category | Key types/macros |
|----------|-----------------|
| App | `App::new()`, `.get()/.post()`, `.scan()`, `.start()`, `.plugin()`, `.merge()` |
| Routing | `#[get/post/put/delete]`, `#[controller]`, `AlunRouter` |
| Response | `Res::ok()`, `Res::page()`, `ApiError::bad_request()` etc. |
| DB | `db()`, `Row`, `db().insert/query/transaction`, `PageQuery` |
| Auth | `JWT`, `AuthClaims`, `#[permission]`, `#[no_auth]`, `TokenClaims` |
| Config | `cfg()`, `config()`, `ALUN_*` env vars |
| Cache | `cache()`, `Cache` trait, `LocalCache`, `SharedCache` |
| Plugins | `Plugin` trait, `#[plugin]`, `SchedulerPlugin` |
| Utils | `Mask`, `Sid`, `Valid`, `Crypto`, `Date`, `Export` |
| Task | `#[task_handler]`, `TaskHandler`, `TaskStorage`, `TaskPlugin` |

Error codes: `OK=0`, `BAD_REQUEST=400`, `UNAUTHORIZED=401`, `FORBIDDEN=403`, `NOT_FOUND=404`, `CONFLICT=409`, `UNPROCESSABLE_ENTITY=422`, `TOO_MANY_REQUESTS=429`, `INTERNAL=500`, `SERVICE_UNAVAILABLE=503`
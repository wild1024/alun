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

> `ignore_paths` 支持精确匹配和**前缀匹配**：若配置 `/api`，则 `/api/login`、`/api/user/info` 等所有以 `/api/` 开头的路径均被忽略认证。

## JWT Manager

```rust
use alun::prelude::JWT;

let jwt = JWT::from_config();

// Login — generate tokens
let access = jwt.create_access_token(
    "user_id", Some("username"),
    &["admin".into()],
    &["*:*".into()]
).map_err(|e| ApiError::internal(e))?;

let refresh = jwt.create_refresh_token("user_id")
    .map_err(|e| ApiError::internal(e))?;

// Refresh — validates old token, blacklists it, returns new pair
let (new_access, new_refresh) = jwt.refresh(&refresh_token).await
    .map_err(|e| ApiError::unauthorized(e))?;

// Logout — adds token to blacklist
jwt.logout(&claims).await;
```

## Accessing Current User in Handlers

```rust
#[alun::get("/api/auth/me")]
async fn me(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<JsonValue> {
    Res::ok(json!({
        "user_id": claims.sub,
        "roles": claims.roles,
        "permissions": claims.permissions,
    }))
}
```

## TokenClaims Helper Methods

```rust
claims.has_role("admin")
claims.has_any_role(&["admin", "mod"])
claims.has_all_roles(&["read", "write"])
claims.has_permission("user:read")
claims.has_any_permission(&["user:read", "user:write"])
claims.is_super_admin()               // Checks for "super_admin" role
```

## Permission & Role Guards

**Builder style:**
```rust
App::new()?
    .with_permission("GET", "/api/admin/stats", admin_stats, "admin:access")
    .with_role("DELETE", "/api/users/{id}", delete_user, "admin")
    .start().await
```

**Proc macro style:**
```rust
#[alun::permission(path = "/api/admin/users", method = "GET", permission = "admin:read")]
#[alun::get("/api/admin/users")]
async fn list_users() -> Res<Vec<UserModel>> { ... }
```

**Path-level rules in config:**
```toml
[middleware.permission]
enabled = true

[[middleware.permission.rules]]
path = "/api/admin"
methods = ["GET", "POST", "PUT", "DELETE"]
permission = "admin:access"
```

## `#[no_auth]` Macro

Marks endpoints that skip auth. A valid token still injects user info if present:

```rust
#[alun::no_auth("/api/public")]
#[alun::get("/api/public")]
async fn public_api() -> Res<String> { ... }
```

## Middleware Stack

All middleware is configured via `config.toml`. Execution order (fixed):

1. **SecurityHeaders** — always first, injects 6 security headers
2. **RequestLog** — conditional, excludes paths in config
3. **RequestId** — conditional
4. **CORS** — conditional
5. **Compression** — conditional
6. **RateLimit** — conditional
7. **AuthLayer** — conditional (JWT verification + blacklist check)
8. **PermissionCheck** — conditional (path-level permission rules)

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
```

### Method-Level Security Middleware

**NonceLayer** (anti-replay for write ops): Client sends `x-nonce` header. Same nonce → 409 Conflict.

**IdempotencyLayer** (for orders/payments): Client sends `x-idempotency-key`. Same key returns cached response.

### Custom Middleware Injection

```rust
App::new()?
    .with_middleware_hook(|router| {
        router.layer(axum::middleware::from_fn(my_custom_middleware))
    })
    .scan()
    .start()
    .await
```
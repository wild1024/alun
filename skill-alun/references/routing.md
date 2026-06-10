# Routing & Response Patterns

## Three Routing Styles

### Style A: Builder Chaining (prototypes)

```rust
App::new()?
    .get("/api/users", list_users)
    .post("/api/users", create_user)
    .put("/api/users/{id}", update_user)
    .delete("/api/users/{id}", delete_user)
    .serve("8080")
    .await
```

### Style B: Proc Macro + scan() (recommended)

Zero manual registration — `scan()` auto-discovers all `#[alun::get]`/`#[alun::post]` etc. functions:

```rust
#[alun::get("/api/users")]
async fn list_users() -> Res<Vec<UserModel>> { ... }

#[alun::post("/api/users")]
async fn create_user(ValidatedJson(req): ValidatedJson<CreateUserReq>) -> Result<Res<UserModel>, ApiError> { ... }

// main.rs:
App::new()?.scan().start().await.unwrap();
```

### Style C: Controller Grouping (large projects)

```rust
#[alun::controller("/api/admin")]
impl AdminController {
    #[alun::get("/dashboard")]
    async fn dashboard() -> Res<String> { ... }

    #[alun::delete("/users/{id}")]
    async fn delete_user(Path(id): Path<String>) -> Result<Res<()>, ApiError> { ... }
}
```

## Sub-Router Merging

```rust
fn user_routes() -> AlunRouter {
    let mut r = AlunRouter::new();
    r.add_get("/", list_users);
    r.add_post("/", create_user);
    r
}

App::new()?
    .merge("/api/users", user_routes())
    .serve("8080")
    .await
```

## Extracting Request Parameters

Use standard axum extractors:

```rust
use axum::extract::{Path, Query};
use axum::Extension;

async fn get_user(Path(id): Path<String>) -> Res<UserModel> { ... }
async fn search(Query(params): Query<HashMap<String, String>>) -> Res<Vec<Row>> { ... }
async fn me(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<JsonValue> { ... }
```

## ValidatedJson Extractor

Auto-deserializes JSON + provides validation:

```rust
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
    Ok(Res::ok(user))
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

```rust
#[derive(Debug, Deserialize, Validate)]
struct EventReq {
    #[validate(custom(function = "validate_uuid"))]
    pub id: String,

    #[validate(custom(function = "validate_date_or_datetime"))]
    pub release_date: Option<String>,  // 支持纯日期或完整时间戳
}

// 手动验证
use alun::ValidateExt;
req.validate_or_reject()?;  // 失败返回 ApiError(422)
```

## Unified Response (`Res<T>`)

All handlers return `Res<T>` (always success) or `Result<Res<T>, ApiError>` (may fail):

```rust
// Success
Res::ok(user)                                     // { code:0, msg:"ok", data:user }
Res::ok_with_msg("u1", "创建成功")                  // Custom message
Res::ok_empty()                                    // No data field

// Standalone failure (not via Result)
Res::fail(codes::BAD_REQUEST, "用户名不能为空")

// Error responses (via Result)
Err(ApiError::bad_request("参数错误"))
Err(ApiError::unauthorized("请先登录"))
Err(ApiError::forbidden("权限不足"))
Err(ApiError::not_found("用户不存在"))
Err(ApiError::conflict("用户名已存在"))
Err(ApiError::unprocessable_entity("邮箱格式不正确"))
Err(ApiError::too_many_requests("请求过于频繁"))
Err(ApiError::internal("服务器内部错误"))
Err(ApiError::internal_masked("服务器内部错误", format!("{:?}", e)))  // Public msg + private detail
Err(ApiError::service_unavailable("服务暂不可用"))

// Pagination
Res::page(users_list, total_count, page_num, page_size)  // → { code:0, data:{list, total, page, page_size} }
```

## Handler Error Handling Best Practices

Never use `unwrap()`/`expect()` in handler code:

```rust
async fn find_user(Path(id): Path<String>) -> Result<Res<UserModel>, ApiError> {
    if id.is_empty() {
        return Err(ApiError::bad_request("ID 不能为空"));
    }
    let user = db().find_by_id("users", &id).await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("用户不存在"))?;
    Ok(Res::ok(user))
}
```

## Production Error Masking

```rust
async fn risky_operation() -> Result<Res<()>, ApiError> {
    do_something().await.map_err(|e| {
        tracing::error!("内部错误: {:?}", e);
        ApiError::internal_masked("服务器内部错误", format!("{:?}", e))
    })
}
```
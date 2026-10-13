# alun-macros

Alun 的过程宏：`#[get]`、`#[post]`、`#[put]`、`#[patch]`、`#[delete]`、`#[controller]`、
`#[routes]`、`#[plugin]`、`#[permission]`、`#[no_auth]`、`#[layer]`、`#[task_handler]`。

编译期代码生成替代运行期反射：被标注的项收集成**编译期注册表**，由 `App::scan()` /
`App::module()` 在启动时装配。

> 门面 `alun` 会再导出这些宏（`#[alun::get("/")]`），业务**不需要**直接依赖本 crate。

## 宏一览

| 宏 | 标注对象 | 作用 |
|-------|---------|------|
| `#[get("/path")]` / `#[post]` / `#[put]` / `#[patch]` / `#[delete]` | `async fn` | 把处理器注册进路由表 |
| `#[routes]` | `mod` | 整块显式注册，宏生成 `register` 函数交给 `App::module()` |
| `#[controller("/prefix")]` | `impl` 块 | 收集块内 `#[get]` 等标注，按前缀注册 `Type::method` |
| `#[permission(path = …, method = …, permission = …)]` | `async fn` | 注册一条权限规则 |
| `#[no_auth("/path")]` | `async fn` | 记入免鉴权路径 |
| `#[layer(L)]` | `async fn` | **路由级 layer**：摘掉该路由的自注册、改生成带 layer 的注册（须写在路由注解**上方**） |
| `#[plugin]` | `struct` | 标记插件类型，并生成固有的 `alun_plugin_name()` 访问器 |
| `#[task_handler(task_type = …, …)]` | `struct` | 注册异步任务处理器 |

### `#[routes]`（新代码推荐）

```rust,ignore
#[alun::routes]
mod api {
    use alun::prelude::*;

    #[alun::get("/users")]
    pub async fn list_users() -> Res<String> { Res::ok("[]".into()) }
}

App::new()?.module(api::register).start().await?;   // register 由宏生成
```

它不依赖链接器段收集，任何工具链都成立，IDE 可跳转、可按模块裁剪。
`#[alun::get]` + `App::scan()` 是另一条并存通道，适合老项目与单文件小服务。

### `#[controller]`

属性写在 `impl` 块上（不是单个函数上）。宏会剥掉块内路由属性并把注册语句放到
`impl` 块之外（处理器按 `Type::method` 引用），前缀与路径自动拼接：

```rust,ignore
use alun_macros::{controller, get, post};

struct UserController;

#[controller("/users")]
impl UserController {
    #[get("/{id}")]
    async fn detail(Path(id): Path<String>) -> Res<String> {
        Res::ok(id)
    }

    #[post("/")]
    async fn create(Json(body): Json<serde_json::Value>) -> Res<String> {
        Res::ok(body.to_string())
    }
}
```

### `#[no_auth]` / `#[permission]`

写在自由处理器的路由属性旁边：

```rust,ignore
use alun_macros::{get, no_auth, permission};

#[no_auth("/api/public")]
#[get("/api/public")]
async fn public_api() -> Res<&'static str> {
    Res::ok("public")
}

#[permission(path = "/api/admin/stats", method = "GET", permission = "admin:access")]
#[get("/api/admin/stats")]
async fn admin_stats() -> Res<&'static str> {
    Res::ok("stats")
}
```

### `#[task_handler]`

参数：`task_type`（`i16`，必填）、`topic`、`priority`（`Low` / `Normal` / `High` / `Critical`）、
`timeout_seconds`、`max_retries`、`retry_strategy`（`Fixed` / `Linear` / `Exponential`）、
`retry_delay_seconds`、`max_retry_delay_seconds`、`description`、`dead_letter_topic`。

```rust,ignore
use alun_macros::task_handler;
use alun_task::TaskHandler;

#[task_handler(task_type = 1, topic = "export", timeout_seconds = 60)]
struct ExportHandler;

#[async_trait::async_trait]
impl TaskHandler for ExportHandler {
    fn task_type(&self) -> i16 { 1 }

    async fn execute(&self, payload: serde_json::Value) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({ "file": payload["order_id"] }))
    }
}
```

## Feature

本 crate 不声明任何 cargo feature。

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)
- 速查参考：`skill-alun/references/routing.md`、`skill-alun/references/extension-points.md`

# Alun Framework

> **Rust Web 开发的终极形态** —— 继承 aifei 极简哲学，以 Rust 零成本抽象重新定义高性能 Web 开发。
>
> 让 Rust Web 开发比 Python 还简单，性能比 Go 还快。

Alun 是世界上首个**面向 AI Coding 的 Rust Web 框架**。它将 aifei 的 Just Service 设计范式与 Rust 的类型安全、零成本抽象、异步运行时深度融合，开创 **Just Config + Just Code** 开发新范式。

**Just Config. Just Code. Only Alun can do.**

---

## 为什么是 Alun

Rust 生态不缺 Web 框架。Actix、Axum、Rocket、Warp —— 它们各有所长，但都诞生于**传统手工编码**时代：概念繁多、分层复杂、样板代码充斥。

Alun 截然不同。它不是又一个 Rust Web 框架，而是**从 AI Coding 视角重新发明的 Rust 开发体验**：

- **配置驱动**：行为由 `config.toml` 决定，修改配置无需重新编译，AI 只需改一行配置就能切换数据库、开启 JWT、调整中间件栈
- **零概念噪音**：消除传统框架的 Controller / Service / Repository / DTO 多层映射，AI 只需关注 **一行代码一个功能** 的极简表达
- **编译期安全**：Rust 的类型系统 + Ownership 在编译期消灭空指针、数据竞争、内存泄漏，AI 生成的代码天然安全
- **一行命令上线**：`cargo run` 即可启动完整生产服务，无需 Docker、无需 Nginx 反代、无需手动配置中间件链

Alun 让 Rust Web 开发从"高手专属"变为"AI 可生成"。

---

## 快速开始

### 1. 添加依赖

```toml
[dependencies]
alun = "0.1"
```

### 2. 三行代码启动服务

```rust
use alun::prelude::*;

#[alun::get("/")]
async fn hello() -> Res<String> {
    Res::ok("Hello, alun!".into())
}

#[tokio::main]
async fn main() {
    App::new().expect("初始化失败").scan().start().await.unwrap();
}
```

### 3. 运行

```bash
# 生成默认配置
cargo run -- gen-config

# 启动服务
cargo run
# → Alun 启动 -> http://127.0.0.1:8023
```

**就这么简单。** 没有样板代码，没有多层目录结构，没有隐式约定。

---

## 核心理念

### Just Config + Just Code

Alun 将 aifei 的 Just Service 范式进一步极致化 —— **Just Config + Just Code**：

- **配置决定行为，而非代码决定配置**。数据库切换？改一行 TOML。开启 JWT？改一行 TOML。调整限流策略？改一行 TOML。
- **代码只表达业务**。不写 Controller，不写 Repository，不写 Mapper —— 写一个函数，加一个宏注解，就完成了。

这一设计直接将 Rust Web 开发的学习曲线从"一个月入门"砍到"一小时上手"。

### 为什么这对 AI Coding 至关重要

大模型生成代码时，上下文长度和注意力浓度是决定性变量。

传统框架的层层抽象、分散配置、隐式约定会严重稀释模型的注意力 —— 大量 Token 消耗在非业务结构上，真正反映业务逻辑的 Token 占比极低。

Alun 从框架层面解决这个问题：

- 统一入口、统一响应、统一数据访问 —— 消除概念切换带来的上下文开销
- 宏注解路由 —— 路由定义和函数实现同屏可见，模型无需跨文件追踪
- 配置集中化 —— 所有行为变数收敛于一份 TOML，模型无需猜测隐式行为

**在 Alun 之上，AI 生成的代码就是生产级代码。**

---

## 核心功能概览

| 功能 | 说明 | 一键启用 |
| --- | --- | --- |
| 🌐 **Web 服务** | axum + tower 中间件栈，编译期零成本抽象 | 默认 |
| 📋 **路由注册** | Builder 链式 / Proc Macro 注解 / Controller 分组 | `#[alun::get("/")]` |
| 📦 **统一响应** | `Res<T>` 标准 JSON 体 + 14 种 `ApiError` 工厂方法 | 默认 |
| 🗄️ **数据库** | PG/MySQL/SQLite，Row 模式 CRUD，RAII 事务，断连自动恢复 | `features = ["db"]` |
| 🔐 **JWT 认证** | Access/Refresh Token + 黑名单 + 角色权限，配置即启用 | `[middleware.auth]` |
| ⚡ **缓存** | LocalCache（内存，零序列化开销）/ RedisCache | `features = ["cache"]` |
| 📝 **模板** | Jinja2 语法（minijinja），服务端渲染开箱即用 | `features = ["template"]` |
| 🔧 **工具集** | 字符串、日期、脱敏、ID、验证、加密、导出、清理、格式化、随机生成 | 默认 |
| 📡 **中间件** | 请求ID、日志、CORS、压缩、限流、权限校验，配置开启即可 | 配置开启 |
| 🛡️ **安全防护** | 安全响应头（标配）+ Nonce 防重放 + 幂等键 + XSS 净化 | 配置/按需 |
| 🧩 **插件** | 拓扑排序生命周期 + 定时任务 + 异步任务 + 邮件通知 | `[plugins]` |
| 📨 **Kafka** | 生产者/消费者（rdkafka），高吞吐消息处理 | `features = ["kafka"]` |
| 📋 **异步任务** | Kafka 驱动 + 指数退避重试 + 死信队列 + 宏自动注册 | `features = ["task"]` |
| 📁 **文件系统** | 本地文件存储抽象，统一读写接口 | `features = ["fs"]` |
| ⚙️ **配置系统** | TOML + 多环境 Profile + `ALUN_*` 环境变量覆盖 | 默认 |

**一份 TOML 配置，掌控全部功能。** 不需要翻阅文档寻找 API，不需要写 YAML 再写代码再写注解 —— 打开 `config.toml`，改一行，重启，功能上线。

---

## 架构概览

```
用户应用代码
     │
     ▼
┌──────────────────────┐
│   alun (facade)      │  ← 统一入口 crate
│   prelude + re-export │
└──┬───┬───┬───┬───┬──┘
   │   │   │   │   │
   ▼   ▼   ▼   ▼   ▼
┌────┐┌────┐┌───┐┌───┐┌─────┐
│ web││ db ││tpl││json││cache│  ← 功能 crate（features 可选）
└──┬─┘└──┬─┘└───┘└───┘└─────┘
   │     │      ← alun-core（核心抽象）
   │     │      ← alun-config / alun-log / alun-utils（基础设施）
   │     │      ← alun-macros（编译期 Proc Macro）
```

```
alun/                              # 工作空间根目录
├── alun/                          # 门面 crate
├── alun-core/                     # 核心抽象：Error、Plugin、Res、ApiError
├── alun-macros/                   # 过程宏：get/post/put/delete/controller/plugin
├── alun-config/                   # 配置：TOML 加载、多环境 Profile
├── alun-log/                      # 日志：tracing 初始化
├── alun-web/                      # Web：App、Router、Middleware、JWT、全局资源
├── alun-db/                       # DB：Row CRUD、事务、Hook、迁移、SQL 模板
├── alun-cache/                    # 缓存：LocalCache + Redis
├── alun-template/                 # 模板：Jinja2
├── alun-utils/                    # 工具：字符串、日期、脱敏、ID、验证、加密、导出、清理、格式化、随机生成
├── alun-plugin/                   # 插件：定时任务、异步任务、邮件通知
├── alun-kafka/                    # Kafka 集成
├── alun-task/                     # 异步任务框架（Kafka 驱动、宏注册、泛型存储）
├── alun-fs/                       # 文件系统抽象
├── docs/                          # 📖 Code Wiki 文档
└── examples/                      # 示例项目
```

---

## 📖 完整文档

详细的 Code Wiki 文档请参阅 [docs/](./docs/) 目录：

| 文档 | 说明 |
| --- | --- |
| [docs/README.md](./docs/README.md) | 文档导航与项目概览 |
| [docs/01-architecture.md](./docs/01-architecture.md) | 整体架构、设计理念、分层架构 |
| [docs/02-crates-reference.md](./docs/02-crates-reference.md) | 各 Crate 详细参考 + 使用示例 |
| [docs/03-dependencies.md](./docs/03-dependencies.md) | 内部依赖关系、外部核心依赖 |
| [docs/04-getting-started.md](./docs/04-getting-started.md) | 运行方式、配置详解、**功能使用示例速查** |

---

## 核心示例速览

### 路由注册（三种方式，自由选择）

```rust
// 方式 A：Builder 链式 —— 极简起步
App::new()?.get("/", hello).post("/", create).serve("8080").await

// 方式 B：Proc Macro + scan —— 零手写注册，编译期自动发现
#[alun::get("/")] async fn hello() -> Res<String> { Res::ok("Hi".into()) }
App::new()?.scan().start().await.unwrap();

// 方式 C：Controller 分组 —— 大型项目结构化
#[alun::controller("/api/admin")]
impl Admin { #[alun::get("/dashboard")] async fn dashboard() -> Res<String> { ... } }
```

### 数据库 CRUD —— 一行代码完成

```rust
let row = Row::table("users").id(Sid::uuid()).set("name", "张三").set("age", 28);
let inserted = db().insert(&row).await?;
let user = db().find_by_id("users", "u1").await?;
let mut row = user.unwrap(); row.set("age", 29); db().update(&row).await?;
db().delete_by_id("users", "u1").await?;
```

**没有 ORM 配置，没有 Entity 注解，没有 Migration 脚本。** Row 模式将数据操作收敛为单一抽象，所有 CRUD 操作共享同一套语义。

### 统一响应 + 错误处理 —— 错误永不泄露

```rust
async fn find_user(Path(id): Path<String>) -> Result<Res<UserModel>, ApiError> {
    let user = db().find_by_id("users", &id).await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("用户不存在"))?;
    Ok(Res::ok(user))
}
```

**14 种 `ApiError` 工厂方法**覆盖全部 HTTP 状态码场景。5xx 错误前端只看到"服务器内部错误"，详细信息仅写入日志 —— 安全性内建于框架。

### JWT 认证 —— 配置即启用

```rust
use alun::prelude::JWT;

// 创建 JWT 管理器（自动读取 config.toml 中的 jwt_secret 等配置）
let jwt = JWT::from_config();

// 登录：生成 Token
let access = jwt.create_access_token("u1", Some("admin"), &["admin".into()], &["*:*".into()])?;
let refresh = jwt.create_refresh_token("u1")?;

// 刷新 Token（一次调用完成验证+黑名单+新Token生成）
let (new_access, new_refresh) = jwt.refresh(&refresh_token).await?;

// 登出（Token 进入黑名单，即时失效）
jwt.logout(&claims).await;

// 获取当前用户 —— 一行解构即可
async fn me(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<JsonValue> {
    Res::ok(json!({"user_id":claims.sub, "roles":claims.roles}))
}
```

### 工具集 —— 200+ 开箱即用函数

```rust
// ── 字符串转换 ──
"helloWorld".to_snake();               // → hello_world
"hello_world".to_camel();               // → helloWorld
"".is_blank();                          // → true

// ── 日期 ──
Date::relative(now.timestamp());        // → "3分钟前"
Date::fmt(&now, "%Y-%m-%d %H:%M:%S");  // → "2026-01-01 12:00:00"

// ── 脱敏 ──
Mask::mobile("13812345678");            // → "138****5678"
Mask::email("a@b.com");                 // → "a***@b.com"
Mask::name("张三丰");                    // → "张**"

// ── ID 生成 ──
Sid::uuid();                            // UUID v4
Sid::uuid7();                           // UUID v7（时间有序）
Sid::tsid();                            // 时间戳+随机数
Sid::short();                           // 16位hex

// ── 验证 ──
Valid::is_email("a@b.com");             // → true
Valid::is_mobile("13812345678");        // → true
Valid::is_strong_password("Abc@12345"); // → true

// ── 加密 ──
Crypto::hash_password("pass123");       // Argon2 哈希
Crypto::verify_password("pass123", &hash)?;
Crypto::random_token(32);               // 随机 hex Token

// ── 字符串清理 ──
sanitize_filename("file<name>.txt");    // → "file_name.txt"
clean_email("  User@Mail.COM  ");       // → "user@mail.com"
InputCleaner::clean_register_input(email, pwd, nickname);

// ── 格式化 ──
format_file_size(1_500_000);            // → "1.43 MB"
parse_json_value(r#"{"key": 1}"#);      // → Ok(Value)

// ── 随机生成 ──
generate_random_digits(6);              // → "573192"（不含0）
generate_random_alphanum(8);            // → "aB3kM9xQ"（无易混淆字符）
generate_invite_code();                 // → 12位随机邀请码
```

### 安全防护 —— 默认安全，无需配置

```rust
// ── 安全响应头（标配，全站自动注入） ──
// X-Content-Type-Options, X-Frame-Options, HSTS, CSP, Referrer-Policy
// 启动即有，通过 config.toml 按需开关

// ── Nonce 防重放（按需，挂载到写操作路由） ──
// router.route("/api/transfer", post(transfer)).route_layer(
//     NonceLayer::new(cache, Duration::from_secs(300))
// );

// ── Idempotency-Key 幂等键（按需，订单/支付路由） ──
// router.route("/api/order/create", post(create)).route_layer(
//     IdempotencyLayer::new(cache, Duration::from_secs(86400))
// );

// ── XSS 净化（按需，需启用 xss feature） ──
use alun_utils::xss;
let safe = xss::sanitize_html("<script>alert(1)</script><p>Hello</p>");
// → "<p>Hello</p>"
```

**安全不是功能，是底线。** Alun 的安全防护体系以"默认开启、按需增强"为原则，启动即有基础防护，关键业务按需叠加。

### 异步任务引擎 —— 企业级任务分发

`features = ["task"]` — 基于 Kafka 的分布式异步任务框架，四大核心能力：

- **编译期自动注册**：`#[task_handler]` 宏利用 linkme 实现编译期 Handler 发现，零手动注册
- **三种重试策略**：Fixed / Linear / Exponential，失败任务自动重试
- **死信队列（DLQ）**：耗尽重试次数的任务进入 DLQ，永不丢失
- **泛型存储解耦**：`TaskStorage` trait 抽象数据库操作，框架零 SQL 依赖

```toml
# config.toml —— 一行配置启用任务引擎
[task]
brokers = "localhost:9092"
group_id = "my-app-task-worker"
scan_interval_secs = 30
max_batch_size = 100
max_message_age_secs = 3600
```

```rust
use alun::alun_task::*;
use async_trait::async_trait;

// 1. 定义 Handler —— 一个宏完成注册+配置
#[alun::task_handler(
    task_type = 1,
    topic = "export_tasks",
    timeout_seconds = 60,
    max_retries = 3,
    retry_strategy = "Exponential",
    description = "数据导出任务"
)]
struct ExportHandler;

#[async_trait]
impl TaskHandler for ExportHandler {
    fn task_type(&self) -> i16 { 1 }
    async fn execute(&self, payload: serde_json::Value) -> Result<serde_json::Value, String> {
        let file_id = payload["file_id"].as_str().unwrap_or("");
        Ok(serde_json::json!({"url": "https://...", "file_id": file_id}))
    }
}

// 2. 实现 TaskStorage —— 通过 db() 全局函数操作数据库，无需传参
struct DbTaskStorage;

#[async_trait]
impl TaskStorage for DbTaskStorage {
    async fn save_task_log(&self, task_id: &str, task_type: i16, priority: i16,
        config: &TaskConfig, params: &SubmitTaskParams) -> Result<(), String> {
        db().execute("INSERT INTO task_logs ...", &[...]).await.map_err(|e| e.to_string())?;
        Ok(())
    }
    // ... 其余 7 个方法
}

// 3. 启动时注册插件
#[tokio::main]
async fn main() {
    App::new().unwrap();  // 初始化全局资源（DB/Cache/Config）
    let task_cfg = cfg().custom.get("task")
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

// 4. 在任意 Handler 中提交任务
#[alun::post("/api/export")]
async fn export_handler() -> Result<Res<String>, ApiError> {
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
        payload: json!({"file_id": "f1"}),
        priority: None, user_id: None, resource_id: None, resource_type: None,
    }).await.map_err(|e| ApiError::internal(e))?;

    Ok(Res::ok(task_id))
}
```

**核心设计**：

- `TaskStorage` trait：8 个泛型方法，插件零 SQL 依赖，由业务方自由适配
- `#[task_handler]` 宏：linkme 编译期自动发现，无需手动注册
- `TaskPlugin`：实现 `alun_core::Plugin`，统一启停管理
- 重试策略：Fixed / Linear / Exponential，支持死信队列（DLQ）
- 配置从 `[task]` section 读取，`TaskWorkerConfig` 实现 Deserialize

---

## 启动参数

```bash
cargo run                           # 启动（端口从 config 读取）
cargo run -- gen-config             # 生成默认 config/config.toml
cargo run -- print-config           # 打印当前配置
cargo run -- profile=prod           # 指定 Profile
ALUN_PROFILE=prod ALUN_LOG_LEVEL=debug cargo run   # 环境变量覆盖
```

---

## 设计原则

1. **配置驱动**：行为由 `config.toml` 决定，修改配置无需重新编译 —— **改一行，重启，上线**
2. **零成本抽象**：纯 Rust trait + 泛型，编译期展开 —— **零反射、零动态分发、零性能损耗**
3. **错误不泄露**：5xx 错误前端模糊化，详细信息仅进入日志 —— **安全性内建于框架**
4. **渐进增强**：从 `App::new().get("/", h).serve("8080")` 起步，按需叠加插件 —— **不绑架用户**
5. **编译器强制安全**：事务 Rollback 由 Rust `Drop` + `?` 天然保证 —— **忘写 rollback？编译器不答应**
6. **按需引入**：功能 crate 通过 `features` 可选引入 —— **不为没用到的代码买单**
7. **安全默认**：安全响应头标配全站注入，防重放/幂等中间件按需挂载 —— **默认安全，按需增强**

---

## 技术栈

| 类别 | 技术选型 |
| --- | --- |
| 语言 | Rust (Edition 2021) |
| HTTP 框架 | axum 0.8 + tower 0.5 |
| 异步运行时 | tokio |
| 数据库 | sqlx 0.8（PG/MySQL/SQLite） |
| 模板 | minijinja 2（Jinja2） |
| 日志 | tracing + tracing-subscriber |
| 配置 | TOML（serde） |
| JWT | jsonwebtoken 9 |
| 密码哈希 | argon2 |
| HTML净化 | ammonia 4（可选 feature） |
| 消息队列 | rdkafka 0.36 |
| 缓存 | 内置 + Redis |

---
交流QQ群：1022721856
<img width="973" height="1208" alt="image" src="https://github.com/user-attachments/assets/9aef0dd2-ea57-4c9e-bd93-29c2bfb49964" />

## License

Apache-2.0

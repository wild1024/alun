# 04 — 项目运行与配置指南

## 1. 环境要求

| 项目        | 要求                          |
| --------- | --------------------------- |
| Rust      | 1.95.0+                     |
| Cargo     | 随 Rust 安装                   |
| 数据库（可选）   | PostgreSQL / MySQL / SQLite |
| Redis（可选） | 6.0+                        |

***

## 2. 添加依赖

### 引入门面 crate

```toml
[dependencies]
alun = "0.1"
```

### 按需引入（最小化编译）

```toml
# 仅 Web 基础功能
alun = { version = "0.1", default-features = false, features = [] }

# Web + 数据库
alun = { version = "0.1", default-features = false, features = ["db"] }

# 全功能
alun = { version = "0.1", features = ["full"] }
```

***

## 3. 快速启动

### 方式一：最简启动（配置驱动）

```rust
use alun::prelude::*;

async fn hello() -> Res<&'static str> {
    Res::ok("Hello, Alun!")
}

#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?                      // 从 config/ 目录加载配置
        .get("/", hello)
        .parse_cli()                 // 支持 gen-config / print-config
        .start()                     // 端口从 config 读取
        .await
}
```

### 方式二：指定端口

```rust
#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        .get("/", hello)
        .serve("8080")               // 指定端口，覆盖配置
        .await
}
```

### 方式三：无状态（用于测试或极简场景）

```rust
#[tokio::main]
async fn main() -> alun::Result<()> {
    App::empty()
        .get("/", || async { "OK" })
        .serve("8080")
        .await
}
```

### 方式四：Proc Macro 路由 + scan

```rust
use alun::{App, Res};

#[alun::get("/")]
async fn index() -> Res<String> {
    Res::ok("Hello Alun!".into())
}

#[alun::post("/api/user")]
async fn create_user() -> Res<String> {
    Res::ok("created".into())
}

#[tokio::main]
async fn main() {
    App::new()
        .expect("初始化失败")
        .scan()                      // 扫描 #[get]/#[post] 等宏注解
        .start()
        .await
        .unwrap();
}
```

***

## 4. 命令行参数

```bash
# 生成默认配置文件到 config/config.toml
cargo run -- gen-config

# 打印当前配置
cargo run -- print-config

# 指定 Profile（加载 config/config-prod.toml）
cargo run -- profile=prod
```

### 环境变量

```bash
# 指定 Profile
ALUN_PROFILE=prod cargo run

# 覆盖配置项
ALUN_SERVER_LISTEN=3000 cargo run
ALUN_LOG_LEVEL=debug cargo run
ALUN_DATABASE_HOST=10.0.0.1 cargo run
ALUN_REDIS_URL=redis://cache.internal:6379 cargo run
ALUN_DATABASE_MIGRATION_AUTO_MIGRATE=true cargo run
```

***

## 5. 配置文件详解

### 生成默认配置

```bash
mkdir -p config
cargo run -- gen-config
```

### 配置结构概览

```toml
# ==================== 应用信息 ====================
[app]
name = "Alun"

# ==================== 服务器 ====================
[server]
listen = "8023"              # 监听地址（端口号 或 ip:port）

# ==================== 日志 ====================
[log]
level = "info"               # trace | debug | info | warn | error
format = "text"              # text（彩色） | json（结构化）
dir = ""                     # 文件输出目录（空=仅 stdout）
file_prefix = "alun"         # 日滚文件名前缀

# ==================== 路由 ====================
[router]
prefix = ""                  # 全局路由前缀（如 "/api/v1"）

[router.not_found]
enabled = true               # 是否启用自定义 404（返回 JSON 统一错误响应）
message = "请求的资源不存在"    # 自定义 404 提示消息

# ==================== 数据库 ====================
[database]
enabled = true
type = "postgres"            # postgres | mysql | sqlite
host = "localhost"
port = 5432
name = "mydb"
user = "app_user"
password = ""
password_encrypted = false   # 开启后密码字段为 AES-GCM Base64 密文
max_connections = 10
min_connections = 2
sql_logging = false
slow_query_ms = 0            # 慢查询阈值（毫秒），0=关闭

[database.migration]
enabled = false
path = "migrations"          # 迁移文件目录（*.up.sql / *.down.sql）
auto_migrate = false         # 启动时自动执行未执行的迁移

# ==================== Redis ====================
[redis]
enabled = false
url = "redis://127.0.0.1:6379"
max_connections = 10

# ==================== 缓存 ====================
[cache]
type = "local"               # local | redis
max_capacity = 10000         # 本地缓存最大条目数
default_ttl = 3600           # 默认过期秒数（0=永不过期）

# ==================== 中间件 ====================
[middleware]
request_id = false           # 注入 x-request-id
request_log = false          # 请求日志

[middleware.request_log_config]
# 支持精确匹配（"/health"）和通配符前缀匹配（"/swagger*"）
exclude_paths = ["/health"]
log_duration = true

[middleware.auth]
enabled = false
jwt_secret = ""              # 优先用环境变量 ${ALUN_JWT_SECRET}
ignore_paths = ["/api/login", "/api/register"]
access_token_expire_secs = 7200
refresh_token_expire_secs = 604800

[middleware.cors]
enabled = false
allow_origins = []
allow_methods = []
allow_headers = []
allow_credentials = true

[middleware.compression]
enabled = false
level = 6                    # Gzip 压缩级别 0-9

[middleware.rate_limit]
enabled = false
requests_per_window = 100
window_secs = 60

[middleware.security_headers]
enabled = true              # 安全头默认开启
nosniff = true              # X-Content-Type-Options: nosniff
frame_options = true        # X-Frame-Options: DENY
hsts = true                 # Strict-Transport-Security
hsts_max_age_secs = 31536000
hsts_include_subdomains = true
csp = true                  # Content-Security-Policy
csp_value = "default-src 'self'"
referrer_policy = true      # Referrer-Policy
referrer_policy_value = "strict-origin-when-cross-origin"
permissions_policy = false  # Permissions-Policy（默认关闭）

# ==================== 上传 / 下载 ====================
[upload]
path = "uploads"
max_size_mb = 10

[download]
path = "downloads"

# ==================== 模板 ====================
[template]
path = "templates"

# ==================== 静态文件 ====================
[static_files]
enabled = false
path = "static"

# ==================== 插件 ====================
[plugins]
enabled = []                 # 按需: cache, notification, async-task, scheduler

[plugins.notification]
smtp_host = ""
smtp_port = 587
smtp_user = ""
smtp_pass = ""
from_email = ""               # 发件人邮箱（为空时使用 smtp_user）
from_name = "系统通知"

[plugins.async_task]
workers = 4

[plugins.scheduler]
workers = 4

# ==================== 自定义配置 ====================
[custom]
# 插件运行时可通过 ConfigManager::get_dynamic() / set_dynamic() 读写

# ==================== 文件存储 ====================
[fs]
default_backend_type = "local"   # 默认后端：local | minio | s3
local_root_dir = "uploads"       # 本地存储根目录
max_file_size_bytes = 52428800   # 上传文件大小上限（字节），默认 50MB
presign_url_ttl_secs = 3600      # 预签名 URL 有效期（秒）

# MinIO / S3 后端的连接信息（从业务存储桶表动态加载，也可在此声明）
# 示例：sys_storage_bucket 表中按 backend_type 拉取 endpoint/access_key/secret_key/region

# ==================== 异步任务 ====================
[task]
brokers = "localhost:9092"       # Kafka broker 地址
group_id = "alun-task-worker"    # 消费组 ID
scan_interval_secs = 30          # 重试扫描间隔（秒）
max_batch_size = 100             # 每批次扫描最大任务数
max_message_age_secs = 3600      # 消息最大时效（秒）
auto_create_topics = false       # 启动时自动创建 topic
topic_partitions = 1             # topic 分区数
topic_replication = 1            # topic 副本数
```

### 多环境 Profile

```bash
alun/
├── config/
│   ├── config.toml           # 基础配置
│   ├── config-dev.toml       # 开发环境覆盖
│   ├── config-test.toml      # 测试环境覆盖
│   └── config-prod.toml      # 生产环境覆盖
```

加载顺序：`config.toml` → `config-{profile}.toml`（按需）→ `ALUN_*` 环境变量

***

## 6. 运行示例项目

```bash
# 进入项目根目录
cd /Volumes/zdh/projects/alun/alun

# 方式一：运行指定的 example
cargo run --example quick-start

# 方式二：从 examples 子目录运行
cd examples/01-basic
cargo run

# 带参数运行
cargo run --example quick-start -- gen-config
cargo run --example quick-start -- print-config
cargo run --example quick-start -- profile=prod

# 环境变量覆盖
ALUN_LOG_LEVEL=debug cargo run --example quick-start
ALUN_SERVER_LISTEN=3000 cargo run --example quick-start
```

### 示例项目说明

| 示例               | Cargo 命令                          | 说明                           |
| ---------------- | --------------------------------- | ---------------------------- |
| `00-quick-start` | `cargo run --example quick-start` | 全功能演示：字符串、日期、脱敏、验证、加密 API    |
| `01-basic`       | （进入目录）`cargo run`                 | 基础启动：配置驱动，`#[alun::get]` 宏路由 |
| `02-auth`        | （进入目录）`cargo run`                 | 认证示例：JWT 登录/刷新/登出/获取当前用户     |
| `03-db-crud`     | （进入目录）`cargo run`                 | 数据库 CRUD + CSV 导出            |

### 示例 03-db-crud 数据库准备

需要先配置 `examples/03-db-crud/config/config.toml` 中的 `[database]` 段，确保数据库可用且存在 `sys_user` 表。

***

## 7. 构建与测试

```bash
# 编译整个 workspace
cargo build

# Release 构建
cargo build --release

# 运行测试
cargo test

# 运行特定 crate 的测试
cargo test -p alun-core
cargo test -p alun-db
cargo test -p alun-cache

# 生成文档
cargo doc --no-deps --open
```

***

## 8. 项目目录建议

在实际使用时，建议按以下结构组织项目：

```
my-project/
├── Cargo.toml               # alun 依赖
├── config/
│   ├── config.toml
│   └── config-prod.toml
├── migrations/              # 数据库迁移文件
│   ├── 001_create_users.up.sql
│   └── 001_create_users.down.sql
├── templates/               # Jinja2 模板文件
│   └── index.html
├── uploads/                 # 文件上传目录
├── downloads/               # 文件下载目录
├── static/                  # 静态文件
└── src/
    ├── main.rs              # 入口
    ├── config.rs            # 自定义配置
    ├── controllers/         # 控制器模块
    ├── models/              # 数据模型
    ├── services/            # 业务逻辑
    └── plugins/             # 自定义插件
```

***

## 9. 关键常见任务

### 添加数据库支持

```toml
# Cargo.toml
[dependencies]
alun = { version = "0.1", features = ["db"] }
tokio = { version = "1", features = ["full"] }
```

```toml
# config/config.toml
[database]
enabled = true
type = "postgres"
host = "localhost"
port = 5432
name = "mydb"
user = "postgres"
password = "postgres"
```

```rust
use alun::prelude::*;

async fn list_users() -> Result<Res<Vec<Row>>, ApiError> {
    let rows = db().query("SELECT * FROM users LIMIT 50", &[])
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(rows))
}
```

### 添加 JWT 认证

```toml
# config/config.toml
[middleware.auth]
enabled = true
jwt_secret = "your-secret-key-min-32-chars"
ignore_paths = ["/api/login"]
```

### 在 Handler 中获取当前用户

```rust
use alun::web::AuthClaims;
use axum::Extension;

async fn me(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<serde_json::Value> {
    Res::ok(json!({
        "user_id": claims.sub,
        "roles": claims.roles,
        "permissions": claims.permissions,
    }))
}
```

### 添加自定义中间件

自定义中间件需实现 `tower::Layer` + `tower::Service` trait，然后在 `App.start()` 前通过 `.route_layer()` 或 `AlunRouter` 的 `add_xxx_with_layer` 方法挂载。

***

## 10. 功能使用示例速查

本节按使用场景归类，方便按需快速查找。

### 10.1 路由注册

```rust
// 方式 A：Builder 链式注册
App::new()?
    .get("/api/users", list_users)
    .post("/api/users", create_user)
    .put("/api/users/{id}", update_user)
    .delete("/api/users/{id}", delete_user)
    .merge("/api/admin", admin_routes())
    .serve("8080").await

// 方式 B：Proc Macro 注解 + scan
#[alun::get("/api/users")]
async fn list_users() -> Res<Vec<UserModel>> { ... }

#[alun::post("/api/users")]
async fn create_user(Json(req): Json<CreateUserReq>) -> Result<Res<UserModel>, ApiError> { ... }

#[alun::controller("/api/admin")]
impl AdminController {
    #[alun::get("/dashboard")]
    async fn dashboard() -> Res<String> { ... }
}

App::new()?.scan().start().await.unwrap();

// 方式 C：分组子路由
fn user_routes() -> AlunRouter {
    let mut r = AlunRouter::new();
    r.add_get("/", list_users);
    r.add_post("/", create_user);
    r
}
```

### 10.2 统一响应

```rust
// 成功响应
async fn get_user() -> Res<UserModel> { Res::ok(user) }
async fn create() -> Res<String> { Res::ok_with_msg("u1", "创建成功") }
async fn delete() -> Res<()> { Res::ok_empty() }

// 分页响应
async fn list() -> Res<PageData<Vec<UserModel>>> {
    Res::page(users, total, 1, 20)
}

// 错误响应
async fn find_user(Path(id): Path<String>) -> Result<Res<UserModel>, ApiError> {
    if id.is_empty() { return Err(ApiError::bad_request("ID 不能为空")); }
    let user = db.find_user(&id).await
        .ok_or_else(|| ApiError::not_found("用户不存在"))?;
    Ok(Res::ok(user))
}

// 生产环境错误脱敏
.map_err(|e| ApiError::internal_masked("服务器内部错误", format!("{:?}", e)))
```

### 10.3 数据库 CRUD

```rust
use alun::prelude::*;
use alun_db::Row;

async fn handler() -> Result<Res<Vec<Row>>, ApiError> {
    // 插入
    let row = Row::table("users").id(Sid::uuid()).set("name", "张三").set("age", 28);
    let inserted = db().insert(&row).await?;

    // 查询单条
    let user = db().find_by_id("users", "u1").await?;

    // 查询多条
    let users = db().query("SELECT * FROM users WHERE active = $1 LIMIT 50", &["true"]).await?;

    // 分页
    let (rows, total) = db().query_page("SELECT * FROM users ORDER BY id", &[], &PageQuery::new(1, 20)).await?;

    // 更新（只更新 changes 字段）
    let mut row = db().find_by_id("users", "u1").await?.unwrap();
    row.set("age", 29);
    db().update(&row).await?;

    // 删除
    db().delete_by_id("users", "u1").await?;
    db().batch_delete_by_ids("users", &["u1", "u2", "u3"]).await?;

    Ok(Res::ok(users))
}
```

### 10.4 事务

```rust
db.transaction(|tx| async move {
    tx.execute("UPDATE accounts SET balance = balance - 100 WHERE id = $1", &["from"]).await?;
    tx.execute("UPDATE accounts SET balance = balance + 100 WHERE id = $1", &["to"]).await?;
    // Ok → Commit / Err → Rollback
    Ok(())
}).await?;
```

### 10.5 JWT 认证

```rust
use alun::prelude::JWT;

// 创建 JWT 管理器（自动读取 config.toml 的 jwt_secret 等配置）
let jwt = JWT::from_config();

// 登录：生成 Access Token + Refresh Token
let access = jwt.create_access_token("u1", Some("admin"), &["admin".into()], &["*:*".into()])?;
let refresh = jwt.create_refresh_token("u1")?;

// 刷新 Token（验证 + 黑名单旧 RefreshToken + 生成新 Token 对）
let (new_access, new_refresh) = jwt.refresh(&refresh_token).await?;

// 登出（将当前 Access Token 加入黑名单）
jwt.logout(&claims).await;

// 获取当前用户
#[alun::get("/api/auth/me")]
async fn me(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<serde_json::Value> {
    Res::ok(json!({"user_id":claims.sub, "roles":claims.roles}))
}
```

### 10.6 缓存

```rust
// 配置 [cache] type="local", 启用 [plugins] enabled=["cache"]
// Handler 中通过全局函数获取
async fn handler() -> Res<String> {
    cache().set_ex("key", "value", 3600).await.unwrap();
    let val: Option<String> = cache().get("key").await.unwrap();
    Res::ok(val.unwrap_or_default())
}

// 手动创建
let cache = alun_cache::create_cache(&cfg().cache, &cfg().redis).await?;
cache.set("k", "v").await?;           // 永不过期
cache.set_ex("k", "v", 300).await?;   // 5分钟过期
let v: Option<String> = cache.get("k").await?;
let n: i64 = cache.incr("counter", 1).await?;
cache.del("k").await?;
cache.delete_pattern("user:*").await?;
```

### 10.7 模板渲染

```rust
use alun_template::TemplateEngine;

// 渲染文件
let html = render_template("page.html", &json!({"title":"Home","items":[...]}))?;

// 从字符串渲染
// 需要手动创建引擎或使用 try_template() 获取全局引擎
let result = engine.render_str("Hello {{ name }}", &json!({"name":"World"}))?;

// 在 Handler 中
async fn home() -> Result<Res<String>, ApiError> {
    let html = render_template("index.html", &json!({"title":"Home"}))
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(html))
}
```

### 10.8 工具集速查

```rust
use alun_utils::*;
use alun::prelude::*;

// ── 字符串 ──
"helloWorld".to_snake(); // → hello_world
"hello_world".to_camel(); // → helloWorld
"".is_blank();            // → true

// ── 字符串清理 ──
sanitize_filename("file<name>.txt");    // → "file_name.txt"
clean_email("  User@Mail.COM  ");       // → "user@mail.com"
clean_string_param("  hello  ");        // → "hello"
clean_password("  pass 123  ");         // → "pass 123"

// ── 输入清理器（注册/登录） ──
let (email, pwd, nick) = InputCleaner::clean_register_input(" A@B.com ", " 123 ", " Tom ");
let (email, pwd) = InputCleaner::clean_login_input(" A@B.com ", " 123 ");

// ── 格式化 ──
format_file_size(0);          // → "0 B"
format_file_size(1_500_000);  // → "1.43 MB"
parse_json_value(r#"{"k":1}"#); // → Ok(Value)

// ── 随机生成 ──
generate_invite_code();         // → 12位邀请码
generate_random_digits(6);      // → 6位数字（不含0）
generate_random_alphanum(8);    // → 8位无混淆字符

// ── 日期 ──
let now = Date::now();
Date::fmt(&now, "%Y-%m-%d %H:%M:%S");
Date::relative(now.timestamp()); // → "3分钟前"

// ── 脱敏 ──
Mask::mobile("13812345678"); // → "138****5678"
Mask::email("a@b.com");     // → "a***@b.com"
Mask::id_card("320112199001011234"); // → "3201****1234"
Mask::name("张三丰");        // → "张**"
Mask::bank_card("6222021234567890"); // → "6222 **** 7890"
Mask::user_id("user_abc123"); // → "us****23"
Mask::password("secret");    // → "******"
Mask::address("北京市海淀区中关村"); // → "北京市海淀****"
Mask::license_plate("京A12345"); // → "京****5"
Mask::mask_by_type("mobile", "13812345678"); // 按类型自动选择

// ── User-Agent 解析 ──
use alun_utils::parse_user_agent;
let info = parse_user_agent("Mozilla/5.0 ... Chrome/120.0");
info.device_type;   // → "PC"
info.browser_type;  // → "Chrome"
info.os_type;       // → "Windows"

// ── ID ──
Sid::uuid();  // UUID v4
Sid::uuid7(); // UUID v7 (时间有序)
Sid::tsid();  // 时间戳+随机数
Sid::short(); // 16位hex
Sid::tiny();  // 8位hex

// ── 验证 ──
Valid::is_email("a@b.com");
Valid::is_mobile("13812345678");
Valid::is_phone("+8613812345678");
Valid::is_url("https://example.com");
Valid::is_ipv4("192.168.1.1");
Valid::is_strong_password("Abc@12345");
Valid::is_username("john_doe");
Valid::is_color("#FF00AA");
Valid::is_uuid("550e8400-e29b-41d4-a716-446655440000");
Valid::is_id_card("110101199003077758");
Valid::is_date("2024-01-01");
Valid::is_datetime("2024-01-01T00:00:00Z");
Valid::is_json(r#"{"key": 1}"#);
Valid::is_base64("SGVsbG8=");
Valid::is_digits("123456");
Valid::is_alphanumeric("abc123");
Valid::len_between("hello", 2, 10);
Valid::has_html("<div>hello</div>");
Valid::is_html_free("plain text");
Valid::is_file_extension("photo.jpg", &["jpg", "png"]);

// ── 加密 ──
Crypto::sha256("data");
Crypto::hash_password("pass123");       // Argon2 哈希
Crypto::verify_password("pass123", &hash)?;  // 自动检测 Argon2/BCrypt 算法
Crypto::random_key();
Crypto::random_token(32);

// ── 导出 ──
let csv = Export::to_csv(&["name","age"], &records)?;
let json = Export::to_json(&records)?;
```

### 10.9 插件

```rust
// 定时任务
use alun_plugin::scheduler::SchedulerPlugin;
let scheduler = SchedulerPlugin::new(4);
scheduler.register("task", "0 */2 * * *", "描述", || Box::pin(async { Ok(()) }));
scheduler.trigger("task").await?;

// 异步任务
use alun_plugin::async_task::AsyncTaskPlugin;
let pool = AsyncTaskPlugin::new(4);
pool.submit(async { heavy_work().await; });

// 异步任务（Kafka 驱动，需 features = ["task"]）
use alun::alun_task::*;

// 定义 Handler（编译期自动发现）
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
    async fn execute(&self, payload: Value) -> Result<Value, String> {
        // 执行业务逻辑 ...
        Ok(json!({"status": "done"}))
    }
}

// 实现 TaskStorage（内部通过 db() 全局函数操作数据库）
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

// 启动时创建插件（Arc::new(DbTaskStorage) 即可，无需传 db）
let task_cfg: TaskWorkerConfig = cfg().custom.get("task")
    .and_then(|v| serde_json::from_value(v.clone()).ok())
    .unwrap_or_default();
let task_plugin = TaskPlugin::new(
    task_cfg,
    Arc::new(DbTaskStorage),
    HandlerRegistry::new().from_discovered(),
)?;

App::new()?.plugin(task_plugin).scan().start().await?;

// 提交任务
let task_cfg: TaskWorkerConfig = cfg().custom.get("task")
    .and_then(|v| serde_json::from_value(v.clone()).ok())
    .unwrap_or_default();
let producer = TaskProducer::new(
    &task_cfg.brokers,
    Arc::new(DbTaskStorage),
    HandlerRegistry::new().from_discovered(),
)?;
let task_id = producer.submit(SubmitTaskParams {
    task_type: 1,
    payload: json!({"file_id": "f1"}),
    priority: None, user_id: None, resource_id: None, resource_type: None,
}).await?;

// 自定义插件
#[alun::plugin]
struct MyPlugin;
#[async_trait]
impl Plugin for MyPlugin {
    fn name(&self) -> &str { "my-plugin" }
    async fn start(&self) -> Result<()> { Ok(()) }
    async fn stop(&self) -> Result<()> { Ok(()) }
    fn depends_on(&self) -> &[&str] { &["db"] }
}
App::new()?.plugin(MyPlugin).serve("8080").await?;
```

### 10.10 配置

```rust
use alun_config::ConfigManager;
use std::sync::Arc;

let cm = Arc::new(ConfigManager::load(Some("config".into())));

// 静态配置
let port = &cm.get().server.listen;
let jwt_secret = &cm.get().middleware.auth.jwt_secret;

// 动态配置（运行时修改）
cm.set_dynamic("rate_limit.requests_per_window", 200);
let limit: Option<i32> = cm.get_dynamic("rate_limit.requests_per_window");

// 生成默认配置
ConfigManager::generate_default("config")?;
```

### 10.11 日志

```rust
// App 自动初始化，代码中使用 tracing
use tracing::{info, warn, error, debug};
info!("用户登录: user_id={}", user_id);
error!("支付回调异常: order_id={}", order_id);
info!(method="POST", path="/api/order", status=200, duration_ms=5, "请求完成");

// Span
#[tracing::instrument(skip(db))]
async fn create_order(db: &Db, req: CreateOrderReq) -> Result<Order, Error> { ... }
```

### 10.12 请求验证

```rust
use validator::Validate;
use alun::validate_uuid;

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

// 使用 ValidateExt + 自定义校验函数 —— 推荐方式
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
    req.validate_or_reject()?; // 一行完成所有字段校验，失败自动返回 422
    Ok(Res::ok("OK".into()))
}
```

### 10.13 多环境部署

```bash
# 项目文件结构
my-project/
├── config/
│   ├── config.toml           # 基础配置
│   ├── config-dev.toml       # 开发环境
│   └── config-prod.toml      # 生产环境

# 切换环境
ALUN_PROFILE=prod cargo run
cargo run -- profile=prod

# 环境变量覆盖
ALUN_SERVER_LISTEN=3000 \
ALUN_DATABASE_HOST=db-prod.internal \
ALUN_LOG_LEVEL=warn \
ALUN_DATABASE_MIGRATION_AUTO_MIGRATE=true \
cargo run
```

### 10.14 安全功能

```rust
// ── 安全响应头（标配，自动注入，全站生效） ──
// config.toml: [middleware.security_headers] enabled = true
// 无需代码，App 启动时自动注册 SecurityHeadersLayer
// 注入头: X-Content-Type-Options, X-Frame-Options, HSTS, CSP, Referrer-Policy

// ── Nonce 防重放（按需，写操作路由） ──
use alun_web::middleware::NonceLayer;
use std::time::Duration;

// 在写操作路由上单独包裹
#[alun::post("/api/transfer")]
async fn transfer() -> Result<Res<()>, ApiError> {
    // 请求需携带 x-nonce 头（UUID），相同 nonce 返回 409 Conflict
    transfer_funds().await?;
    Ok(Res::ok_empty())
}

// 通过 route_layer 挂载 NonceLayer
// router.route("/api/transfer", post(transfer)).route_layer(
//     NonceLayer::new(cache_arc, Duration::from_secs(300))
// );

// ── Idempotency-Key 幂等键（按需，订单/支付路由） ──
use alun_web::middleware:{id}empotencyLayer;

// 在关键写操作路由上单独包裹
#[alun::post("/api/order/create")]
async fn create_order() -> Result<Res<OrderModel>, ApiError> {
    // 请求需携带 x-idempotency-key 头
    // 相同 key 返回首次缓存的完整响应，保证不重复执行
    let order = place_order().await?;
    Ok(Res::ok(order))
}

// 通过 route_layer 挂载 IdempotencyLayer
// router.route("/api/order/create", post(create_order)).route_layer(
//     IdempotencyLayer::new(cache_arc, Duration::from_secs(86400))
// );

// ── XSS HTML 净化（按需，需启用 xss feature） ──
// Cargo.toml: alun = { features = ["xss"] }
use alun_utils::xss;

let user_html = "<script>alert(1)</script><p>Hello</p>";
let safe = xss::sanitize_html(&user_html);       // → "<p>Hello</p>"
let strict = xss::sanitize_html_strict(&user_html); // → "Hello"（纯文本）
if xss::has_potential_xss(&user_html) {
    tracing::warn!("检测到潜在 XSS 载荷，已净化");
}
```

### 10.15 Feature 按需引入

```toml
# 最小 Web
alun = { default-features = false, features = [] }

# Web + DB
alun = { default-features = false, features = ["db"] }

# Web + DB + Cache
alun = { default-features = false, features = ["db", "cache"] }

# Web + DB + Cache + Template
alun = { default-features = false, features = ["db", "cache", "template"] }

# Web + 文件存储（本地 / MinIO / S3）
alun = { default-features = false, features = ["fs"] }

# Web + 异步任务（Kafka 驱动）
alun = { default-features = false, features = ["task"] }

# Web + XSS 净化
alun = { default-features = false, features = ["xss"] }

# 全部
alun = { features = ["full"] }
```

### 10.16 文件上传/下载路径

```rust
// ── 获取上传目录路径 ──
// 启动时 App 自动根据 [upload].path 创建目录（默认 "uploads"）
use alun::{upload_path, try_upload_path};

#[alun::post("/api/file/upload")]
async fn upload() -> Res<String> {
    let dir = upload_path();          // 返回 "uploads"（或 config 配置的路径）
    // 可用 std::fs 或 alun_fs::LocalFs 进行文件保存
    let full_path = format!("{}/{}", dir, "myfile.pdf");
    Res::ok(full_path)
}

// ── 获取下载目录路径 ──
// 启动时 App 自动根据 [download].path 创建目录（默认 "downloads"）
use alun::{download_path, try_download_path};

#[alun::get("/api/file/download/:name")]
async fn download(Path(name): Path<String>) -> Res<String> {
    let dir = download_path();        // 返回 "downloads"（或 config 配置的路径）
    let full_path = format!("{}/{}", dir, name);
    // 读取文件并返回 ...
    Res::ok(full_path)
}

// ── 配合 alun_fs 使用 ──
// FsPlugin 统一管理存储后端，一行创建本地存储
use alun_fs::FsPlugin;

let plugin = FsPlugin::new_local(upload_path());
let meta = plugin.write("report.pdf", &data).await?;

// 多后端示例：按 backend_type 路由
let meta = plugin.write_to(Some("minio"), "report.pdf", &data).await?;
```

### 10.17 静态文件服务

```toml
# config.toml
[static_files]
enabled = true              # 启用静态文件服务
path = "static"             # 静态文件目录（默认 "static"）
```

启动后 App 自动创建 `static/` 目录，通过 `tower_http::services::ServeDir` 将目录挂载为 Router 的 fallback——所有未匹配 API 路由的 HTTP 请求会回退到 `static/` 目录下查找对应文件。

```rust
// 无需写任何代码，仅配置即可：
// App::new()?.scan().start().await  —— 启动时自动处理
```

目录结构示例：
```
my-project/
├── static/
│   ├── index.html          # 访问 http://host:8023/index.html
│   ├── css/
│   │   └── style.css       # 访问 http://host:8023/css/style.css
│   └── favicon.ico
└── config/
    └── config.toml
```


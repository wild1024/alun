# 03 — 依赖关系

## 1. 内部 Crate 依赖关系图

```
                          ┌──────────────┐
                          │  alun-core   │  (零 Web 依赖)
                          └──────┬───────┘
                                 │ 被所有 crate 依赖
      ┌──────────────┬───────────┼───────────┬──────────────┐
      │              │           │           │              │
      ▼              ▼           ▼           ▼              ▼
┌────────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐
│ alun-config │ │alun-log │ │alun-utils│ │alun-cache│ │alun-templ│
└──────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘ └────┬─────┘
       │             │           │           │            │
       ▼             ▼           ▼           │            │
┌──────────────────────────────────────┐     │            │
│           alun-web                    │     │            │
│ (App/Router/Middleware/State)        │◄────┘            │
└──────────────┬───────────────────────┘                  │
               │                                           │
               ▼                                           │
┌──────────────────────────────────────────────────────────┴──┐
│                         alun                                │
│        (facade: prelude + re-export + proc macros)          │
└──────┬──────────┬──────────┬──────────┬─────────────────────┘
       │          │          │          │
       ▼          ▼          ▼          ▼
┌──────────┐ ┌────────┐ ┌──────────┐ ┌────────┐ ┌────────┐
│ alun-db  │ │alun-   │ │alun-kafka│ │alun-fs │ │alun-   │
│          │ │plugin  │ │          │ │        │ │task    │
└──────────┘ └────────┘ └──────────┘ └────────┘ └────────┘
```

### 依赖详解

| Crate | 直接依赖（内部） | 说明 |
|-------|-----------------|------|
| **alun-core** | 无 | 最底层，无内部依赖 |
| **alun-config** | `alun-core` | 配置加载需核心错误类型 |
| **alun-log** | `alun-config` | 按 LogConfig 初始化 tracing |
| **alun-utils** | `alun-core` | 工具函数使用核心错误类型 |
| **alun-cache** | `alun-core`, `alun-config` | 缓存工厂需配置，Trait 用核心错误 |
| **alun-template** | `alun-core` | 模板错误 → 核心错误 |
| **alun-web** | `alun-core`, `alun-config`, `alun-log`, `alun-db`, `alun-cache`, `alun-utils`（部分） | Web 层集成所有基础设施 |
| **alun-db** | `alun-core`, `alun-config`, `alun-utils` | 数据库需配置、加密工具、错误类型 |
| **alun-plugin** | `alun-core`, `alun-config`, `alun-cache` | 插件基于核心 Plugin trait + 配置驱动 |
| **alun-kafka** | `alun-core`, `alun-config` | Kafka 集成核心框架 |
| **alun-task** | `alun-core`（Plugin trait） | 异步任务框架，零 SQL 依赖，持久化通过 TaskStorage trait 委托 |
| **alun-fs** | `alun-core`, `alun-config` | 文件系统核心框架 |
| **alun** | 所有以上 crate（按需 features） | 门面层统一导出 |
| **alun-macros** | `alun-web`（编译期引用） | Proc Macro 生成引用 `alun::ROUTES` 的代码 |

### alun-web 对 alun-db/alun-cache/alun-utils 的依赖

这些依赖通过 Rust feature 控制：

- `alun-web/template` feature → 引入 `alun-template`，通过 `render_template()` 全局函数使用
- `alun-db` 在 `App::init_resources()` 中 conditionally 使用
- `alun-cache` 在 `App::init_resources()` 中 conditionally 创建缓存
- `alun-utils::valid` 在 `ValidatedJson::validate()` 中使用

---

## 2. 外部核心依赖

### 2.1 HTTP / Web 框架

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **axum** | 0.8 | HTTP Web 框架 | `alun-web`（App、Router、Middleware、Extract） |
| **tower** | 0.5 | 中间件 Layer/Service trait 体系 | 所有中间件模块 |
| **tower-http** | 0.6 | CORS、压缩中间件、静态文件服务 | `alun-web::app`（CORS、Compression、ServeDir） |
| **http** | 1 | HTTP 类型（HeaderValue, Method, StatusCode） | 多个中间件 |
| **hyper** | (axum 依赖) | 底层 HTTP 实现 | axum 内部 |
| **bytes** | 1 | 字节缓冲区 | 中间件 |

### 2.2 数据库

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **sqlx** | 0.8 | 异步数据库驱动（PG/MySQL/SQLite） | `alun-db`（所有数据库操作） |
| **redis** | 0.29 | Redis 异步客户端 | `alun-cache`(RedisCache), `alun-db`（有条件使用） |

### 2.3 序列化与数据处理

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **serde** | 1 | 序列化框架 | 全项目（所有 struct 的 Serialize/Deserialize） |
| **serde_json** | 1 | JSON 处理 | `alun-core`(Res/ApiError), `alun-cache`, `alun-db`(Row) |
| **regex** | (自动) | SQL 参数索引调整、文件名清理 | `alun-db::db`, `alun-utils::str` |

### 2.4 认证与安全

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **jsonwebtoken** | 9 | JWT 编解码 | `alun-web`(AuthLayer, JWT struct) |
| **bcrypt** | 0.16 | Bcrypt 密码哈希 |（预留依赖） |
| **argon2** | (alun-utils 依赖) | Argon2 密码哈希 | `alun-utils::crypto` |
| **aes-gcm** | (alun-utils 依赖) | AES-256-GCM 加解密 | `alun-utils::crypto` |
| **sha2** | (alun-utils 依赖) | SHA-256 哈希 | `alun-utils::crypto` |
| **hmac** | (alun-utils 依赖) | HMAC | `alun-utils::crypto` |
| **rand** | (alun-utils 依赖) | 随机数 | `alun-utils::crypto` |

### 2.5 模板

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **minijinja** | 2 | Jinja2 模板引擎 | `alun-template` |
| **askama** | 0.12 | 编译期模板（预留） | — |

### 2.6 日志

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **tracing** | 0.1 | 结构化日志框架 | 全项目 |
| **tracing-subscriber** | 0.3 | 日志订阅器 | `alun-log` |
| **tracing-appender** | (alun-log 依赖) | 日志文件滚动 | `alun-log` |

### 2.7 异步运行时

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **tokio** | 1 (full) | 异步运行时 | 全项目 |

### 2.8 其他

| Crate | 版本 | 用途 | 使用位置 |
|-------|------|------|----------|
| **anyhow** | 1 | 灵活错误处理 | 示例项目 |
| **thiserror** | 2 | 派生 Error trait | `alun-db`(DbError) |
| **async-trait** | 0.1 | async trait 支持 | `alun-core`(Plugin), `alun-cache`(Cache), `alun-db`(Hook) |
| **linkme** | 0.3 | 编译期分布式切片 | `alun-web`(ROUTES), `alun-macros` |
| **inventory** | 0.3 | 编译期类型注册（备选） | — |
| **parking_lot** | 0.12 | 高性能同步原语 | 全项目（RwLock, Mutex） |
| **dashmap** | 6 | 并发 HashMap | — |
| **uuid** | 1 | UUID v4/v7 | `alun-web`(RequestId), `alun-web`(TokenClaims) |
| **chrono** | 0.4 | 日期时间 | `alun-utils::date`, `alun-db`(TimestampHook) |
| **validator** | 0.20 | 结构体字段校验 | `alun-web::extract`(ValidatedJson) |
| **ammonia** | 4 | HTML/XSS 净化（可选 feature） | `alun-utils::xss` |
| **rdkafka** | 0.36 | Kafka 客户端 | `alun-kafka`, `alun-task` |
| **flume** | (alun-kafka 依赖) | 高性能通道 | `alun-kafka` |
| **toml** | (alun-config 依赖) | TOML 解析/序列化 | `alun-config` |
| **paste** | (alun-db 依赖) | 宏拼接标识符 | `alun-db::db`（impl_db_ops 宏） |

---

## 3. Feature 开关传递链

```
alun (features = ["full"])
  ├── "full" = ["db", "template", "cache", "plugin", "kafka", "task", "fs"]
  │
  ├── "db" → alumn-db (直接链接)
  ├── "template" → alun-template + alun-web/template
  │     └── alun-web/template → 激活 alun-web 中的 template feature
  │           └── 通过 render_template() 全局函数使用
  ├── "cache" → alun-cache (直接链接)
  ├── "plugin" → alun-plugin (直接链接)
  ├── "kafka" → alun-kafka (直接链接)
  ├── "task" → alun-task (直接链接)
  ├── "fs" → alun-fs (直接链接)
  └── "xss" → alun-utils/xss feature (引入 ammonia)

alun-core (features = ["axum"])
  └── "axum" → 激活 Res<T> 和 ApiError 的 IntoResponse impl
        └── 引入 axum 依赖
```

---

## 4. 编译期依赖最小化

对于一个仅需 Web 服务的项目：

```toml
[dependencies]
alun = { version = "0.1", default-features = false, features = [] }
```

仅编译 `alun-core` + `alun-config` + `alun-log` + `alun-web` + `alun`，不需要 DB/Template/Cache/Kafka/FS 的代码。

对于需要数据库的项目：

```toml
[dependencies]
alun = { version = "0.1", default-features = false, features = ["db"] }
```

额外编译 `alun-db` + `sqlx`，其余不编译。

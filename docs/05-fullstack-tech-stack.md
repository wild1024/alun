# 05 — Alun 全栈开发推荐技术栈

本文档基于 Alun 框架的核心能力与设计理念，结合 AIFEI 框架的设计模式经验，给出全栈开发的技术选型推荐和项目架构方案。

***

## 1. 技术架构全景图

```
┌─────────────────────────────────────────────────────────────────┐
│                         前端层 (Frontend)                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │  Vue 3 / React│  │  TypeScript  │  │  Vite / Next.js/Nuxt │  │
│  │  (SPA/SSR)   │  │  (类型安全)    │  │  (构建工具)            │  │
│  └──────┬───────┘  └──────┬───────┘  └──────────┬───────────┘  │
│         │                 │                      │              │
│         └─────────────────┼──────────────────────┘              │
│                           │ HTTP/SSE/WebSocket                  │
├───────────────────────────┼─────────────────────────────────────┤
│                           ▼                      后端层 (Backend) │
│  ┌────────────────────────────────────────────────────────────┐ │
│  │                      Alun Framework                         │ │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────┐  │ │
│  │  │ alun-web │ │ alun-db  │ │alun-cache│ │ alun-template │  │ │
│  │  │ (路由/认证)│ │ (CRUD)   │ │ (缓存)    │ │ (模板渲染)     │  │ │
│  │  └──────────┘ └──────────┘ └──────────┘ └──────────────┘  │ │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────┐  │ │
│|  │  │alun-task │ │alun-kafka│ │ alun-fs  │ │ alun-plugin  │  │ │
|  │  │ (异步任务) │ │ (消息队列) │ │ (文件存储) │ │ (插件/定时/单号)│  │ │
│  │  └──────────┘ └──────────┘ └──────────┘ └──────────────┘  │ │
│  └────────────────────────────────────────────────────────────┘ │
│                           │                                      │
├───────────────────────────┼─────────────────────────────────────┤
│                           ▼                      基础设施层       │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────────┐   │
│  │PostgreSQL│ │  Redis   │ │  Kafka   │ │  MinIO / S3      │   │
│  │ (主数据库) │ │ (缓存/MQ) │ │ (消息队列) │ │  (对象存储)       │   │
│  └──────────┘ └──────────┘ └──────────┘ └──────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

***

## 2. 后端技术栈（核心）

### 2.1 框架层 — Alun

| 能力 | 技术选型 | 说明 |
|------|---------|------|
| **HTTP 框架** | axum 0.8 + tower 0.5 | 基于 hyper 的高性能异步 HTTP，tower 中间件生态 |
| **异步运行时** | tokio 1.x (full) | Rust 生态标准异步运行时 |
| **路由注册** | `#[alun::get]` / `#[controller]` / Builder 链式 | 编译期 linkme 注册，零运行时开销 |
| **统一响应** | `Res<T>` / `ApiError` | 标准 JSON 响应体 + 14 种 HTTP 错误工厂方法 |
| **请求校验** | `ValidatedJson<T>` + validator | 自动 JSON 解析 + 字段级校验 |
| **配置管理** | TOML + 多环境 Profile + `ALUN_*` 环境变量 | 配置驱动，修改无需重新编译 |
| **日志** | tracing + tracing-subscriber | 结构化日志，支持 text/json 双格式，日滚输出 |

### 2.2 数据库层

| 能力 | 技术选型 | 说明 |
|------|---------|------|
| **数据库** | PostgreSQL 16+ (首选) / MySQL 8.0+ / SQLite 3 | sqlx 0.8 异步驱动 |
| **数据访问** | `Row` 模式 + 原生 SQL | 非 ORM，HashMap 式操作 + 变更追踪 + 自动类型适配 |
| **事务管理** | RAII 事务 (`db().transaction()`) | Ok → Commit，Err/Drop → Rollback，编译器保证 |
| **迁移管理** | 内置 `Migrator` + `*.up.sql`/`*.down.sql` | 按时间戳排序扫描 `migrations/` 目录 |
| **连接池** | sqlx 内置连接池 | 可配置 max/min connections |
| **慢查询监控** | `slow_query_ms` 配置 | tracing 输出慢 SQL 及耗时 |
| **SQL 模板** | Jinja2 语法（内置 `SqlTemplate`） | 动态 SQL 拼接，防注入 |

### 2.3 缓存层

| 能力 | 技术选型 | 说明 |
|------|---------|------|
| **本地缓存** | `LocalCache`（HashMap + RwLock） | 进程内缓存，支持 TTL、容量限制、后台清理 |
| **分布式缓存** | Redis 6.0+ | 基于 redis-rs ConnectionManager |
| **统一接口** | `Cache` trait + `SharedCache` 枚举 | 消除 dyn trait 对象安全问题，切换零代码改动 |
| **API** | `cache()` 全局函数 | 无需 State 注入，一行调用 |

### 2.4 认证与安全

| 能力 | 技术选型 | 说明 |
|------|---------|------|
| **JWT** | jsonwebtoken 9 | Access + Refresh 双 Token 机制 |
| **Token 管理** | `JWT::from_config()` | 自动读取 `config.toml` 的 secret/过期时间 |
| **黑名单机制** | 内置（基于 `jti` + 内存/Redis） | Refresh 自动黑名单旧 Token |
| **权限控制** | 路径规则 + `#[alun::permission]` 注解 | 白名单模式，双机制 |
| **密码哈希** | Argon2（生成）+ BCrypt（兼容验证）<br/>`Crypto::hash_password` / `Crypto::verify_password` | 自动检测算法格式，支持混合迁移 |
| **加密** | AES-256-GCM（`Crypto::aes_encrypt`） | 对称加密，适合数据库密码字段 |
| **安全响应头** | 6 个安全头自动注入 | X-Content-Type-Options / X-Frame-Options / HSTS / CSP / Referrer-Policy / Permissions-Policy |
| **防重放** | `NonceLayer`（按需） | `x-nonce` 头去重 |
| **幂等键** | `IdempotencyLayer`（按需） | `x-idempotency-key` 保证写操作幂等 |
| **XSS 净化** | ammonia 4（按需 feature） | HTML 标签白名单过滤 |
| **限流** | `RateLimitLayer` | IP 滑动窗口限流 |

### 2.5 消息队列与异步任务

| 能力 | 技术选型 | 说明 |
|------|---------|------|
| **消息队列** | Apache Kafka（rdkafka 0.36） | 高吞吐、持久化、分区有序 |
| **任务框架** | `alun-task`（features = ["task"]） | Kafka 驱动的异步任务分发 |
| **任务注册** | `#[alun::task_handler]` 宏 | 编译期 linkme 自动发现 |
| **任务持久化** | `TaskStorage` trait（业务方实现） | 框架零 SQL 依赖，自由适配 PG/MySQL/MongoDB |
| **重试策略** | Fixed / Linear / Exponential | 可配置延迟、最大重试次数 |
| **死信队列** | 内置 DLQ 支持 | 超限任务推入死信 topic |

### 2.6 文件存储

| 能力 | 技术选型 | 说明 |
|------|---------|------|
| **统一接口** | `StorageBackend` trait | write / read / delete / exists / presign_url / health_check 六大契约 |
| **多后端注册** | `BackendRegistry` | 按 backend_type 管理实例，支持运行时注册 + linkme 编译期自动发现 |
| **插件门面** | `FsPlugin`（实现 `Plugin` trait） | 统一生命周期管理，`write_to(backend_type, ...)` 按后端类型路由 |
| **本地存储** | `LocalFs` | 按日期分目录 YYYY/MM/DD/uuid.ext，自动 MIME 推断 |
| **对象存储** | `MinioBackend`（feature = "minio"） | MinIO / AWS S3 兼容，支持预签名 URL |
| **自定义后端** | `#[storage_backend]` 宏 + `impl StorageBackend` | 编译期自动发现（linkme 分布式切片），零手动注册 |
| **配置管理** | `[fs]` section（config.toml） | default_backend_type / local_root_dir / max_file_size_bytes / presign_url_ttl_secs |
| **上传路径** | `upload_path()` 全局函数 | 配置 `[upload].path` 控制 |

### 2.7 模板引擎

| 能力 | 技术选型 | 说明 |
|------|---------|------|
| **服务端渲染** | minijinja 2（Jinja2 语法） | `render_template()` 全局函数 |
| **适用场景** | 管理后台 / 邮件模板 / 静态页面 | 不推荐作为主要前端方案 |

***

## 3. 前端技术栈推荐

### 3.1 推荐方案一：Vue 3 生态（中小型项目 / 快速开发）

| 技术 | 版本 | 说明 |
|------|------|------|
| **Vue** | 3.5+ | 渐进式框架，上手快，生态丰富 |
| **TypeScript** | 5.x | 类型安全，与 Rust 后端类型可对应 |
| **Vite** | 6.x | 极速 HMR，ESBuild 打包 |
| **Pinia** | 2.x | Vue 3 官方状态管理 |
| **Vue Router** | 4.x | SPA 路由 |
| **Element Plus** / **Naive UI** | — | 企业级 UI 组件库，后台管理首选 |
| **Axios** | 1.x | HTTP 客户端，请求/响应拦截 |
| **Tailwind CSS** | 4.x | 原子化 CSS，快速布局 |

**适用场景**：后台管理系统、ERP、CRUD 密集型应用、中小型 SaaS

### 3.2 推荐方案二：React 生态（大型项目 / 复杂交互）

| 技术 | 版本 | 说明 |
|------|------|------|
| **React** | 19+ | 生态最丰富的前端框架 |
| **TypeScript** | 5.x | 类型安全 |
| **Next.js** | 15+ (App Router) | SSR/SSG/ISR 全支持，SEO 友好 |
| **Vite** | 6.x | 替代方案（纯 SPA 场景） |
| **Zustand** / **Jotai** | — | 轻量级状态管理 |
| **Ant Design** / **shadcn/ui** | — | 企业级 UI 组件库 |
| **TanStack Query** | 5.x | 服务端状态管理，缓存/重试/乐观更新 |
| **Tailwind CSS** | 4.x | 原子化 CSS |

**适用场景**：大型 SaaS 平台、高交互 C 端应用、需要 SSR/SEO 的场景

### 3.3 推荐方案三：Nuxt 3（Vue SSR / 全栈）

| 技术 | 版本 | 说明 |
|------|------|------|
| **Nuxt** | 3.x | Vue 3 SSR 框架，文件即路由 |
| **TypeScript** | 5.x | 类型安全 |
| **Nuxt UI** / **Element Plus** | — | UI 组件库 |
| **Pinia** | 2.x | 状态管理 |
| **ofetch** | — | Nuxt 内置 HTTP 客户端 |

**适用场景**：官网、营销页、需要 SEO 的 Vue 项目

### 3.4 前端方案决策矩阵

| 维度 | Vue 3 + Vite | React + Next.js | Nuxt 3 |
|------|-------------|-----------------|--------|
| 学习曲线 | ★☆☆ 低 | ★★☆ 中 | ★★☆ 中 |
| TypeScript 支持 | ★★★ 优秀 | ★★★ 优秀 | ★★★ 优秀 |
| SSR/SEO | 需 Nuxt | Next.js 原生 | 原生支持 |
| 后台管理 UI | Element Plus/Naive UI | Ant Design/shadcn/ui | Nuxt UI/Element Plus |
| 打包体积 | 小（~50KB） | 中 | 中 |
| AI 辅助开发 | ★★★ 极友好 | ★★☆ 友好 | ★★★ 极友好 |
| **推荐场景** | 后台管理、中小型 SaaS | 大型 SaaS、C 端高交互 | Vue 技术栈的 SSR 项目 |

### 3.5 前后端协作规范

#### API 协议

```typescript
// 统一响应类型（与 alun 的 Res<T> 对齐）
interface ApiResponse<T = unknown> {
  code: number;    // 0 = 成功
  msg: string;
  data: T | null;
}

// 分页响应类型（与 alun 的 PageData<T> 对齐）
interface PageData<T> {
  list: T[];
  total: number;
  page: number;
  page_size: number;
}

// 分页请求参数（与 alun 的 PageQuery 对齐）
interface PageQuery {
  page: number;       // 默认 1
  page_size: number;  // 默认 20，最大 1000
}
```

#### 请求封装（Axios 示例）

```typescript
import axios from 'axios';

const api = axios.create({
  baseURL: '/api',
  timeout: 30000,
});

// 请求拦截器：自动注入 Token
api.interceptors.request.use((config) => {
  const token = localStorage.getItem('access_token');
  if (token) {
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

// 响应拦截器：统一错误处理 + Token 自动刷新
api.interceptors.response.use(
  (res) => {
    const { code, msg, data } = res.data;
    if (code !== 0) {
      // 业务错误统一提示
      return Promise.reject(new Error(msg));
    }
    return data;
  },
  async (error) => {
    if (error.response?.status === 401) {
      // Token 过期 → 自动刷新
      const refreshed = await refreshToken();
      if (refreshed) {
        return api(error.config); // 重试
      }
      // 刷新失败 → 跳转登录页
      window.location.href = '/login';
    }
    return Promise.reject(error);
  }
);
```

#### TypeScript 类型安全

推荐使用以下工具链实现前后端类型共享：

| 方案 | 说明 |
|------|------|
| **手写类型**（推荐起步） | 前端按 Rust DTO 手写 TS interface，简单直接 |
| **ts-rs** | 从 Rust struct 自动生成 TS 类型定义，适合复杂项目 |
| **OpenAPI/Swagger** | 为 alun 路由生成 OpenAPI 文档，自动生成 TS 客户端 |

---

## 4. 项目架构模式（参考 AIFEI 框架设计）

Alun 的设计理念源自 AIFEI（Java Web 框架）在 Rust 生态的表达。以下是推荐的架构模式：

### 4.1 AIFEI → Alun 设计映射

| AIFEI (Java) | Alun (Rust) | 设计优势 |
|-------------|------------|---------|
| "配置即回调"启动模式 | Builder 链式 `App::new().scan().start()` | Rust 编译期校验所有类型 |
| 多级拦截器分层 | tower Layer 栈 | 类型安全 + 零运行时开销 |
| 方法级参数注入 | axum `FromRequest` Extractor | 编译期校验，无反射 |
| **全局资源单例** | `OnceLock` + `db()`/`cfg()`/`cache()` | 无需 State 注入，代码极简 |
| Db + Record 模式 | `Db` 门面 + `Row` + sqlx | 异步原生，无 ORM 臃肿 |
| SQL 模板（Enjoy SQL） | `SqlTemplate`（Jinja2 语法） | 通用标准语法 |
| 隐式事务提交 | `Drop` + `?` 编译器保证 | 不可能忘记回滚 |
| Plugin 生命周期 | `Plugin` trait + 拓扑排序 | 异步 start/stop，编译期自动发现 |
| 类扫描 → 编译期注册 | `linkme` 分布式切片 | 零启动时间，毫秒级 |

### 4.2 推荐项目目录结构

参考 `Alun框架推荐项目架构.md` 的 "handler / dto / model" 三层极简方案：

```
my-project/
├── Cargo.toml                    # alun 依赖
├── config/
│   ├── config.toml               # 基础配置
│   ├── config-dev.toml           # 开发环境
│   └── config-prod.toml          # 生产环境
├── migrations/                   # 数据库迁移（*.up.sql / *.down.sql）
├── templates/                    # Jinja2 模板（管理后台/邮件）
├── uploads/                      # 文件上传目录
├── static/                       # 前端构建产物（或独立部署）
├── src/
│   ├── main.rs                   # 入口：App::new().scan().start()
│   ├── lib.rs                    # 库根（可选）
│   │
│   ├── middleware/               # 自定义中间件
│   │   ├── audit_log.rs          #   操作审计日志
│   │   ├── op_log.rs             #   操作日志
│   │   └── mod.rs
│   │
│   ├── task/                     # 后台任务处理器
│   │   ├── export_task.rs        #   数据导出任务
│   │   ├── email_task.rs         #   邮件发送任务
│   │   └── mod.rs
│   │
│   ├── plugins/                  # 自定义插件
│   │   ├── payment.rs            #   支付网关插件
│   │   ├── sms.rs                #   短信通道插件
│   │   └── mod.rs
│   │
│   ├── utils/                    # 额外工具函数
│   │   ├── gen_no.rs             #   业务单号生成
│   │   └── mod.rs
│   │
│   ├── shared/                   # 共享类型/常量
│   │   ├── config.rs             #   自定义配置扩展
│   │   ├── constant.rs           #   业务常量
│   │   └── mod.rs
│   │
│   └── modules/                  # 业务模块（垂直切割）
│       ├── user/                 #   用户模块
│       │   ├── mod.rs
│       │   ├── handler.rs        #     路由处理函数
│       │   ├── dto.rs            #     请求/响应 DTO
│       │   └── model.rs          #     数据库操作
│       │
│       ├── product/              #   产品模块
│       │   ├── mod.rs
│       │   ├── handler.rs
│       │   ├── dto.rs
│       │   └── model.rs
│       │
│       ├── order/                #   订单模块
│       │   └── ... (同上)
│       │
│       └── system/               #   系统模块
│           └── ...
│
├── web/                          # 前端项目（独立目录）
│   ├── package.json
│   ├── vite.config.ts
│   └── src/
│       ├── api/                  #   API 请求层
│       ├── views/                #   页面组件
│       ├── components/           #   通用组件
│       ├── stores/               #   状态管理
│       └── router/               #   路由配置
```

### 4.3 业务模块三层职责

| 文件 | 职责 | 依赖方向 |
|------|------|---------|
| `handler.rs` | 路由函数：解析请求 → 调用 model → 构造响应 | 依赖 `model`、`dto`、`shared`、`utils` |
| `dto.rs` | 请求/响应结构体（`*Req`/`*Res`），含 `serde` + `validator` 标注 | 只依赖 `shared` 的基础类型 |
| `model.rs` | 实体数据库操作（`db().insert/find_by_id/query/update`），返回 `Row`/D | 依赖 `shared`、`utils`、其他模块 model |

**极简原则**：没有独立的 `service`/`dao`/`mapper` 层——业务逻辑直接在 handler 中，数据库操作在 model 中。这特别适合 AI 辅助开发场景，因为 AI 理解 "模型 = 数据 + 行为" 比理解多层抽象更准确。

### 4.4 横切关注点归属

| 关注点 | 位置 | 说明 |
|--------|------|------|
| **自定义中间件** | `middleware/` | HTTP 请求前后处理的通用逻辑（审计日志、操作日志等），所有模块共用 |
| **后台任务** | `task/` | 异步/定时任务（导出报表、发送邮件），由事件或调度触发 |
| **插件** | `plugins/` | 第三方集成的可插拔能力（支付、短信、文件存储），统一接口 + 配置切换 |
| **工具函数** | `utils/` | 纯函数式辅助工具（单号生成、业务计算等），无状态、无副作用 |
| **共享类型** | `shared/` | 自定义配置、业务常量、通用类型，避免重复定义和循环引用 |

### 4.5 Handler 标准模式

```rust
// modules/user/handler.rs

use alun::prelude::*;

// ── 查询列表 ──
#[alun::get("/api/users")]
async fn list_users(
    Query(params): Query<HashMap<String, String>>,
) -> Result<Res<PageData<Vec<UserRes>>>, ApiError> {
    let page = params.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
    let page_size = params.get("page_size").and_then(|p| p.parse().ok()).unwrap_or(20);
    let pq = PageQuery::new(page, page_size);

    let (rows, total) = UserModel::list(&pq).await
        .map_err(|e| ApiError::internal(e))?;
    let list = rows.into_iter().map(UserRes::from).collect();

    Ok(Res::page(list, total, pq.page(), pq.page_size()))
}

// ── 查询详情 ──
#[alun::get("/api/users/{id}")]
async fn get_user(Path(id): Path<String>) -> Result<Res<UserRes>, ApiError> {
    let row = UserModel::find_by_id(&id).await
        .map_err(|e| ApiError::internal(e))?
        .ok_or_else(|| ApiError::not_found("用户不存在"))?;
    Ok(Res::ok(UserRes::from(row)))
}

// ── 创建 ──
#[alun::post("/api/users")]
async fn create_user(
    Extension(AuthClaims(claims)): Extension<AuthClaims>,
    ValidatedJson(req): ValidatedJson<CreateUserReq>,
) -> Result<Res<UserRes>, ApiError> {
    let row = UserModel::create(&req, &claims.sub).await
        .map_err(|e| ApiError::internal(e))?;
    Ok(Res::ok_with_msg(UserRes::from(row), "创建成功"))
}

// ── 更新 ──
#[alun::put("/api/users/{id}")]
async fn update_user(
    Path(id): Path<String>,
    ValidatedJson(req): ValidatedJson<UpdateUserReq>,
) -> Result<Res<UserRes>, ApiError> {
    let row = UserModel::update(&id, &req).await
        .map_err(|e| ApiError::internal(e))?
        .ok_or_else(|| ApiError::not_found("用户不存在"))?;
    Ok(Res::ok_with_msg(UserRes::from(row), "更新成功"))
}

// ── 删除 ──
#[alun::delete("/api/users/{id}")]
async fn delete_user(Path(id): Path<String>) -> Result<Res<()>, ApiError> {
    let deleted = UserModel::delete(&id).await
        .map_err(|e| ApiError::internal(e))?;
    if !deleted {
        return Err(ApiError::not_found("用户不存在"));
    }
    Ok(Res::ok_msg("删除成功"))
}
```

### 4.6 Model 标准模式

```rust
// modules/user/model.rs

use alun::prelude::*;

pub struct UserModel;

impl UserModel {
    /// 分页查询用户列表
    pub async fn list(pq: &PageQuery) -> DbResult<(Vec<Row>, u64)> {
        db().query_page(
            "SELECT * FROM sys_user WHERE is_deleted = false ORDER BY created_at DESC",
            &[],
            pq,
        ).await
    }

    /// 按主键查询用户
    pub async fn find_by_id(id: &str) -> DbResult<Option<Row>> {
        db().find_by_id("sys_user", id).await
    }

    /// 创建用户
    pub async fn create(req: &CreateUserReq, created_by: &str) -> DbResult<Row> {
        let mut row = Row::table("sys_user")
            .id(Sid::uuid())
            .set("username", &req.username)
            .set("real_name", &req.real_name)
            .set("email", &req.email)
            .set("created_by", created_by);
        db().insert(&row).await
    }

    /// 更新用户（只更新 changes 中的字段）
    pub async fn update(id: &str, req: &UpdateUserReq) -> DbResult<Option<Row>> {
        let mut row = db().find_by_id("sys_user", id).await?
            .ok_or(DbError::NotFound)?;
        if let Some(ref name) = req.real_name { row.set("real_name", name); }
        if let Some(ref email) = req.email { row.set("email", email); }
        db().update(&row).await
    }

    /// 删除用户
    pub async fn delete(id: &str) -> DbResult<bool> {
        db().delete_by_id("sys_user", id).await
    }
}
```

### 4.7 DTO 标准模式

```rust
// modules/user/dto.rs

use serde::{Deserialize, Serialize};
use validator::Validate;

/// 创建用户请求
#[derive(Debug, Deserialize, Validate)]
pub struct CreateUserReq {
    #[validate(length(min = 2, max = 50, message = "用户名长度必须为 2-50 个字符"))]
    pub username: String,
    #[validate(length(min = 1, max = 50, message = "姓名不能为空"))]
    pub real_name: String,
    #[validate(email(message = "邮箱格式不正确"))]
    pub email: String,
}

/// 更新用户请求
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateUserReq {
    pub real_name: Option<String>,
    #[validate(email(message = "邮箱格式不正确"))]
    pub email: Option<String>,
}

/// 用户响应
#[derive(Debug, Serialize)]
pub struct UserRes {
    pub id: String,
    pub username: String,
    pub real_name: String,
    pub email: String,
    pub created_at: String,
}

impl From<Row> for UserRes {
    fn from(r: Row) -> Self {
        Self {
            id: r.get_id().to_string(),
            username: r.get_as("username").unwrap_or_default(),
            real_name: r.get_as("real_name").unwrap_or_default(),
            email: r.get_as("email").unwrap_or_default(),
            created_at: r.get_as("created_at").unwrap_or_default(),
        }
    }
}
```

---

## 5. 开发工作流与工具链

### 5.1 开发环境

| 工具 | 用途 |
|------|------|
| **Rust** 1.95.0+ | 编译工具链 |
| **cargo** | 包管理 + 构建 + 测试 |
| **rust-analyzer** | VS Code / IDE 代码提示 |
| **clippy** | Lint 检查 (`cargo clippy`) |
| **rustfmt** | 代码格式化 (`cargo fmt`) |
| **pnpm** / **npm** | 前端包管理 |
| **Docker Compose** | 本地 PostgreSQL/Redis/Kafka 环境 |

### 5.2 推荐的 Cargo.toml

```toml
[package]
name = "my-project"
version = "0.1.0"
edition = "2021"

[dependencies]
alun = { version = "0.1", features = ["db", "cache", "task", "fs"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
validator = { version = "0.20", features = ["derive"] }
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
```

### 5.3 启动与调试

```bash
# 生成默认配置
cargo run -- gen-config

# 开发模式（热重载需要 cargo-watch）
cargo watch -x run

# 带环境变量
ALUN_PROFILE=dev ALUN_LOG_LEVEL=debug cargo run

# Release 构建（10-20MB 静态链接二进制）
cargo build --release

# Lint + 格式化
cargo clippy -- -D warnings
cargo fmt -- --check

# 测试
cargo test
```

### 5.4 Docker Compose 本地基础设施

```yaml
# docker-compose.yml
version: '3.8'
services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: mydb
      POSTGRES_USER: app
      POSTGRES_PASSWORD: secret
    ports:
      - "5432:5432"
    volumes:
      - pgdata:/var/lib/postgresql/data

  redis:
    image: redis:7-alpine
    ports:
      - "6379:6379"

  kafka:
    image: bitnami/kafka:latest
    environment:
      KAFKA_CFG_NODE_ID: 1
      KAFKA_CFG_PROCESS_ROLES: broker,controller
      KAFKA_CFG_CONTROLLER_QUORUM_VOTERS: 1@kafka:9093
      KAFKA_CFG_LISTENERS: PLAINTEXT://:9092,CONTROLLER://:9093
      KAFKA_CFG_ADVERTISED_LISTENERS: PLAINTEXT://localhost:9092
      KAFKA_CFG_CONTROLLER_LISTENER_NAMES: CONTROLLER
      KAFKA_CFG_LISTENER_SECURITY_PROTOCOL_MAP: CONTROLLER:PLAINTEXT,PLAINTEXT:PLAINTEXT
    ports:
      - "9092:9092"

volumes:
  pgdata:
```

---

## 6. 部署与运维

### 6.1 部署架构

```
                    ┌──────────────┐
                    │   Nginx /    │
                    │   Caddy      │  ← 反向代理 + SSL 终端
                    └──────┬───────┘
                           │
            ┌──────────────┼──────────────┐
            │              │              │
            ▼              ▼              ▼
    ┌──────────┐   ┌──────────┐   ┌──────────┐
    │  Alun    │   │  Alun    │   │  Static  │
    │  :8023   │   │  :8023   │   │  Files   │
    │(instance1)│  │(instance2)│  │  (前端)   │
    └────┬─────┘   └────┬─────┘   └──────────┘
         │              │
         └──────┬───────┘
                │
    ┌───────────┼───────────┐
    │           │           │
    ▼           ▼           ▼
┌──────┐  ┌──────┐  ┌──────────┐
│  PG  │  │Redis │  │  Kafka   │
└──────┘  └──────┘  └──────────┘
```

### 6.2 部署方式

| 方式 | 说明 | 适用场景 |
|------|------|---------|
| **二进制直接部署** | `cargo build --release` → 单个 ~10-20MB 二进制，`scp` + `systemd` | 中小规模，追求简单 |
| **Docker 容器** | `FROM rust:alpine` 多阶段构建 → ~50MB 镜像 | 标准化部署，K8s |
| **Kubernetes** | Deployment + Service + Ingress | 大规模微服务 |

### 6.3 Nginx 反向代理配置

```nginx
server {
    listen 443 ssl http2;
    server_name api.example.com;

    # SSL 证书
    ssl_certificate     /etc/ssl/certs/example.com.pem;
    ssl_certificate_key /etc/ssl/private/example.com.key;

    # 前端静态文件
    location / {
        root /var/www/frontend/dist;
        try_files $uri $uri/ /index.html;
    }

    # API 反向代理
    location /api/ {
        proxy_pass http://127.0.0.1:8023;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # SSE / WebSocket 支持
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_buffering off;
    }
}
```

### 6.4 监控与可观测性

| 能力 | 技术选型 | 说明 |
|------|---------|------|
| **结构化日志** | `tracing` JSON 输出 → ELK / Loki | `format = "json"` + `dir = "logs"` |
| **健康检查** | `#[alun::get("/health")]` | K8s liveness/readiness probe |
| **指标** | `TaskMetrics`（原子计数） | 内置任务总数/成功/失败计数 |
| **APM** | tracing span → Jaeger | `#[tracing::instrument]` 自动生成 span |

---

## 7. 技术选型决策总结

### 7.1 按项目类型推荐

| 项目类型 | 后端 | 前端 | 数据库 | 缓存 | 消息队列 |
|---------|------|------|--------|------|---------|
| **后台管理系统** | Alun + `features = ["db"]` | Vue 3 + Element Plus + Vite | PostgreSQL | LocalCache | 不需要 |
| **中小型 SaaS** | Alun + `features = ["db", "cache"]` | Vue 3/React + Vite | PostgreSQL | Redis | 不需要 |
| **大型 SaaS 平台** | Alun + `features = ["db", "cache", "task"]` | React + Next.js | PostgreSQL | Redis | Kafka |
| **API 服务** | Alun + `features = ["db", "cache"]` | 只提供 API，无前端 | PostgreSQL | Redis | 按需 |
| **数据处理平台** | Alun + `features = ["db", "task", "fs"]` | Vue 3 管理界面 | PostgreSQL | Redis | Kafka |
| **微服务** | Alun 多实例（每个服务独立启动） | 独立前端 / BFF | 按服务 | Redis | Kafka |

### 7.2 核心优势总结

| 维度 | 传统 Java/Spring 方案 | Alun 全栈方案 | 提升幅度 |
|------|----------------------|-------------|---------|
| 启动时间 | 1-3s (JVM) | <50ms | **20-60×** |
| 内存占用 | 200-500MB (JVM + 框架) | 30-80MB | **5-10×** |
| 部署体积 | JRE 200MB+ + jar | 静态链接 10-20MB | **10-20×** |
| 事务安全 | 代码约定 | 编译器强制 | 消除忘记关闭事务的风险 |
| 类型安全 | 运行时反射 | 编译期保证 | 杜绝运行时类型错误 |
| 并发能力 | 线程池阻塞 | tokio 异步 | DB 密集场景 **3-10×** |
| AI 开发友好度 | 注解多、抽象层多 | 三层极简、全局函数直调 | 大幅降低 AI 理解成本 |

---

## 8. 学习路径建议

### 8.1 后端开发者

1. **Day 1**：运行 `00-quick-start` 示例，理解 `App::new().scan().start()` 启动模式
2. **Day 2**：阅读 `01-architecture.md`，理解分层架构和全局资源单例设计
3. **Day 3**：运行 `03-db-crud` 示例，掌握 `Row` 模式 CRUD + 事务
4. **Day 4**：运行 `02-auth` 示例，理解 JWT 认证 + 权限校验
5. **Day 5**：按 `Alun框架推荐项目架构.md` 的三层结构，搭建业务模块

### 8.2 全栈开发者

1. **后端**：完成 8.1 的学习路径
2. **前端**：选择 Vue 3（推荐入门）或 React，搭建独立 `web/` 目录
3. **联调**：配置 Nginx 反代 / Vite proxy，API 路径统一 `/api/*`
4. **部署**：Docker Compose 一键启动 PG + Redis + Alun + Nginx

---

> **核心理念**：Alun 遵循 **"约定优于配置、编译器优于约定"** 的设计哲学——能用 Rust 编译器保证的绝不靠运行时，能用配置控制的绝不靠代码。全栈开发时，前端选型遵循 **"AI 友好 + 生态成熟"** 原则，Vue 3 + Element Plus 是后台管理场景的最优解。
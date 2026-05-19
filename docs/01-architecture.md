# 01 — 整体架构与设计理念

## 1. 设计哲学

Alun 不是 aifei（Java Web 框架）的代码翻译，而是 aifei **设计理念的 Rust 表达**：

| aifei 设计精华 | alun 实现方式 | 优势 |
|--------------|-------------|------|
| "配置即回调"启动模式 | Builder 链式调用 | Rust 编译期校验所有类型，回调变同步构建 |
| 多级拦截器分层 | tower Layer 栈 | 类型安全 + 零运行时开销 + 标准生态 |
| 方法级参数注入 | axum `FromRequest` | 类型安全 Extractor，编译期校验 |
| **全局资源单例** | `std::sync::OnceLock` | 无需 State 注入，一行 `db()`/`cfg()` 即可访问 |
| Db + Row 数据访问 | `Db` 门面 + `Row` struct + sqlx | 异步原生 + 编译期 SQL 校验（可选） |
| SQL 模板（Enjoy SQL） | Jinja2 模板语法 | 通用标准语法，零学习成本 |
| 隐式事务提交 | `Drop` + `?` 保证 | 编译器强制执行，不可能"忘记回滚" |
| Plugin 生命周期 | Plugin trait + 拓扑排序 | 异步 start/stop，编译期自动发现 |
| 类扫描 → 编译期注册 | `linkme` 编译期注册 | 零启动时间，毫秒级 |

## 2. 分层架构

```
用户应用代码
     │
     ▼
┌──────────────────────┐
│   alun (facade)      │  ← 用户唯一引入的 crate
│   prelude + re-export │
└──┬───┬───┬───┬───┬──┘
   │   │   │   │   │
   ▼   ▼   ▼   ▼   ▼
┌────┐┌────┐┌───┐┌───┐┌─────┐
│web ││ db ││tpl││json││cache│  ← 功能 crate（按需 features）
└──┬─┘└────┘└───┘└───┘└─────┘
   │
   ▼
┌──────────────┐
│  alun-core   │  ← 核心抽象：Error、Plugin trait、API 类型
└──────────────┘
```

### 分层原则

- **`alun-core`** 尽量轻，零 Web 框架依赖，仅定义 Error、Plugin、Res 等基础类型
- **功能 crate** 可选引入（通过 `features = ["db", "cache", "kafka", "fs"]` 控制）
- **`alun`** 是唯一用户界面，API 保持稳定，所有元素通过 `prelude` 统一导出

### 辅助 Crate

| Crate | 职责 | 层级 |
|-------|------|------|
| `alun-config` | TOML 配置加载、多环境 Profile、动态配置 | 基础设施 |
| `alun-log` | tracing 日志初始化（text/json/文件） | 基础设施 |
| `alun-utils` | 字符串、日期、脱敏、ID、验证、加密、导出、清理、格式化、随机生成等工具 | 基础设施 |
| `alun-macros` | Proc Macro（路由注解 `#[get]`/`#[post]`、`#[controller]`、`#[plugin]`） | 基础设施 |
| `alun-web` | App 构建器、路由注册器、中间件体系、JWT 管理、全局资源 | 核心功能 |
| `alun-db` | 数据库连接池、Row CRUD、事务、Hook、迁移、SQL 模板 | 核心功能 |
| `alun-cache` | 缓存抽象 Trait + LocalCache + RedisCache 实现 | 核心功能 |
| `alun-template` | minijinja 模板引擎封装 | 核心功能 |
| `alun-plugin` | 内置插件实现（通知/缓存/异步任务/定时任务/SID） | 扩展功能 |
| `alun-kafka` | Kafka 生产者/消费者（基于 rdkafka） | 扩展功能 |
| `alun-task` | 异步任务框架（Kafka 驱动、宏注册、泛型存储、DLQ） | 扩展功能 |
| `alun-fs` | 文件系统抽象（多后端存储：本地/MinIO/S3） | 扩展功能 |

## 3. 请求处理流程

```
HTTP Request
     │
     ▼
┌─────────────────────────────────────────────────┐
│ tower Layer Stack（中间件链，按配置顺序执行）       │
│ ┌───────────┐ ┌──────────────┐ ┌───────────┐   │
│ │Security   │→│ RequestId    │→│ RequestLog│→  │
│ │Headers    │ │ Layer        │ │ Layer     │   │
│ │Layer(标配)│ └──────────────┘ └───────────┘   │
│ └───────────┘                                  │
│ ┌───────────┐ ┌──────────────┐ ┌───────────┐  │
│ │CORS/      │→│ RateLimit    │→│ Permission│  │
│ │Compression│ │ Layer        │ │ Check     │  │
│ └───────────┘ └──────────────┘ └───────────┘  │
│ ┌───────────┐                                  │
│ │AuthLayer  │                                  │
│ │(JWT认证)  │                                  │
│ └───────────┘                                  │
└─────────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────┐
│ axum Router           │
│ ┌──────────────────┐ │
│ │ Route Matching    │ │  ← 路径匹配、方法匹配
│ │ ↓                │ │
│ │ MethodRouter      │ │  ← 可带方法级 Layer（权限/角色校验）
│ │ ↓                │ │
│ │ Handler 调用       │ │  ← 无 State 注入，直接调用全局资源函数
│ └──────────────────┘ │
└──────────────────────┘
     │
     ▼
Handler 返回 Res<T> / Result<Res<T>, ApiError>
     │
     ▼
IntoResponse 序列化为 JSON HTTP Response
```

### 全局资源访问方式

业务代码中直接调用全局函数访问资源：
- `db()` → 获取数据库实例
- `cache()` → 获取缓存实例
- `cfg()` → 获取配置
- `render_template()` → 渲染模板
- `upload_path()` → 获取上传文件存储目录
- `download_path()` → 获取下载文件存储目录

所有资源通过标准库 `OnceLock` 保证线程安全，只能在框架启动后访问。

## 4. 启动流程

```
App::new()  / App::from_config()
     │
     ├── ConfigManager::load()         ← 加载 TOML 配置
     ├── alun_log::init()              ← 初始化日志
     ├── parse_cli()                   ← 解析 gen-config/print-config/profile
     │
     ▼
.start() / .serve(":8080")
     │
     ├── 1. init_global_resources()     ← 初始化 Db/Cache/Config/Template/Upload/Download
     ├── 2. startup_hook()              ← 执行用户自定义启动回调（可选）
     ├── 3. plugin.check_duplicate_names()  ← 检查插件名重复
     ├── 4. plugin.start_all()              ← 拓扑排序启动插件
     ├── 5. router.into_axum()              ← 构建 axum Router（无需 with_state）
     ├── 6. build_middleware_chain()        ← 构建中间件链
     ├── 7. static_file_serve()             ← 当 static_files.enabled 时挂载 ServeDir
     ├── 8. handle_not_found()              ← 当 router.not_found.enabled 时注册 JSON 404 fallback
     ├── 9. custom_middleware_hook()        ← 用户自定义中间件嵌入点（可选）
     ├── 10. axum::serve(listener, router)   ← 启动 HTTP 服务
     │
     ▼
Ctrl-C → graceful shutdown
     │
     ▼
plugin.stop_all() ← 逆序关闭插件
```

## 5. 设计原则

1. **配置驱动**：行为由 `config.toml` 决定，修改配置无需重新编译
2. **零成本抽象**：纯 Rust trait + 泛型，编译期展开，零反射 / 零动态分发
3. **错误不泄露**：5xx 错误前端模糊化，详细信息仅进入日志
4. **渐进增强**：从最简单的 `App::new().get("/", h).serve("8080")` 起步，按需叠加配置
5. **类型安全**：Rust 编译期保证类型正确，杜绝运行时类型错误
6. **编译器强制安全**：事务 Rollback、错误处理由 Rust `Drop` + `?` 天然保证
7. **按需引入**：功能 crate 通过 features 可选引入，不编译不需要的代码
8. **安全默认**：安全响应头标配自动注入（X-Content-Type-Options、X-Frame-Options、HSTS、CSP），防重放/幂等中间件按需挂载到写操作路由

## 6. 安全体系

| 安全能力 | 类型 | 说明 |
|----------|------|------|
| **安全响应头** | 标配（自动） | 6 个安全头全站注入，通过配置按需开关 |
| **Nonce 防重放** | 按需 | 检查 `x-nonce` 请求头，缓存去重，建议在写操作路由上使用 |
| **幂等键** | 按需 | `x-idempotency-key` 保证同一请求只执行一次，建议在订单/支付路由上使用 |
| **XSS 净化** | 按需（feature） | 基于 ammonia 的 HTML 净化工具，需启用 `xss` feature |
| **JWT 认证** | 标配（可选配） | Bearer Token 天然免疫 CSRF，支持 Access/Refresh Token + 黑名单 |
| **SQL 注入防护** | 标配（自动） | sqlx 参数化查询（`$1`/`$2`），杜绝 SQL 拼接注入 |
| **IP 限流** | 标配（可选配） | 滑动窗口限流，防暴力破解和 DDoS |
| **权限校验** | 标配（可选配） | 路径规则 + 注解双重权限控制 |

## 7. 与 Java 框架对比

| 维度 | aifei (Java) | alun (Rust) | 提升幅度 |
|------|-------------|------------|---------|
| HTTP 服务器 | Undertow (~4MB) | axum + hyper (~1.5MB) | 内存 -60% |
| 启动时间 | 1-3s (JVM 预热) | <50ms | **20-60×** |
| 路由扫描 | classpath 遍历 | linkme 编译期注册 | 零运行时开销 |
| AOP | CGLib 字节码代理 | tower Layer | 类型安全 + 无运行时开销 |
| JSON | Fastjson2 反射 | serde 编译期 | **2-3×** |
| SQL 校验 | 运行时 | sqlx 编译期（可选） | 杜绝 SQL 语法错误上线 |
| 事务安全 | 代码约定 | Drop + ? | 编译器保证 |
| 并发模型 | 线程池阻塞 | tokio 异步 | DB 密集场景 **3-10×** |
| 部署体积 | JRE 200MB+ | 静态链接 ~10-20MB | **10-20×** |
| 运行时依赖 | JDK 17+ | 无 | 绿色部署 |

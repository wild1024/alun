# Alun 框架 Code Wiki

> 轻量级 Rust Web 框架 —— 配置驱动 · 插件扩展 · 开箱即用

## 文档导航

| 文档                                               | 说明                            |
| ------------------------------------------------ | ----------------------------- |
| [01-architecture.md](01-architecture.md)         | 整体架构、设计理念、分层架构、与 Java 框架对比    |
| [02-crates-reference.md](02-crates-reference.md) | 各 Crate 详细参考：关键结构体、Trait、函数说明 |
| [03-dependencies.md](03-dependencies.md)         | 内部模块依赖关系图、外部核心依赖说明            |
| [04-getting-started.md](04-getting-started.md)   | 项目运行方式、配置说明、示例代码、环境变量         |

## 项目概览

**Alun** 是一个借鉴 aifei 核心设计思想、结合 Rust 语言优势（零成本抽象、类型安全、异步运行时）打造的轻量级 Web 开发框架。

### 核心特性

- **配置驱动**：行为由 `config.toml` 控制，修改配置无需重新编译
- **一行启动**：`App::new().get("/", handler).serve("8080").await`
- **插件系统**：支持拓扑排序的插件生命周期管理
- **多层中间件**：安全响应头、JWT 认证、请求日志、IP 限流、防重放、幂等、CORS、压缩、权限校验
- **文件管理**：上传/下载目录自动创建 + 静态文件 Serving（配置驱动，零代码）
- **数据库抽象**：统一 PostgreSQL/MySQL/SQLite 的 Row 模式 CRUD，编译期事务安全
- **编译期路由**：Proc Macro + `linkme` 实现零运行时反射的路由注册
- **统一响应**：`Res<T>` 标准 JSON 响应体，自动序列化

### 技术栈

| 类别      | 技术选型                         |
| ------- | ---------------------------- |
| 语言      | Rust (Edition 2021)          |
| HTTP 框架 | axum 0.8 + tower 0.5 + hyper |
| 异步运行时   | tokio                        |
| 数据库     | sqlx 0.8（PG/MySQL/SQLite）    |
| 模板引擎    | minijinja 2（Jinja2 语法）       |
| 缓存      | 内置 LocalCache + Redis        |
| 日志      | tracing + tracing-subscriber |
| 配置      | TOML（serde 解析）               |
| 序列化     | serde + serde\_json          |
| JWT     | jsonwebtoken 9               |
| 密码哈希    | argon2 + bcrypt              |
| 消息队列    | rdkafka 0.36                 |

### Cargo Workspace 结构

```
alun/                              # 工作空间根目录
├── alun/                          # 门面 crate（prelude、re-export）
├── alun-core/                     # 核心抽象层：Error、Plugin、Res、ApiError
├── alun-macros/                   # 过程宏：get/post/put/delete/controller/plugin
├── alun-config/                   # 配置系统：TOML 加载、多环境、动态配置
├── alun-log/                      # 日志：tracing 初始化
├── alun-web/                      # Web 层：App 构建器、路由、中间件、状态
├── alun-db/                       # 数据库：Row 模式、事务、Hook、迁移、SQL 模板
├── alun-cache/                    # 缓存：本地内存缓存 + Redis 缓存
├── alun-template/                 # 模板：Jinja2 渲染
├── alun-utils/                    # 工具集：字符串、日期、脱敏、验证、加密
├── alun-plugin/                   # 内置插件：缓存、通知、异步任务、定时任务
├── alun-kafka/                    # Kafka 集成：生产者/消费者
├── alun-task/                     # 异步任务框架：Kafka 驱动、宏注册、泛型存储、DLQ
├── alun-fs/                       # 文件系统抽象：本地文件存储
└── examples/                      # 示例项目
    ├── 00-quick-start/            # 快速入门：全功能演示
    ├── 01-basic/                  # 基础启动：配置驱动
    ├── 02-auth/                   # 认证示例：JWT 登录/刷新/登出
    └── 03-db-crud/                # 数据库 CRUD + 导出
```

### 版本信息

- **版本**: 0.1.0
- **许可**: Apache-2.0
- **仓库**: <https://github.com/wild1024/alun>
- **Rust 版本**: 1.95.0+


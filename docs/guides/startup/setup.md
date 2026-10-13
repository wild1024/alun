---
id: guides.startup.setup
title: 准备环境与引入依赖
summary: Rust 版本要求、项目初始化、把 alun 加进 Cargo.toml 并选择 feature 组合。
tags: [getting-started, setup, cargo, features]
status: active
owner: alun-core-team
source_of_truth: [code, examples, config/config.toml]
depends_on: [guides.getting-started]
last_reviewed: 2026-10-06
---

# 准备环境与引入依赖

> 本文件是 [`getting-started.md`](../getting-started.md) 的 `setup` 分册；总目录与全流程顺序见 [从零到跑起来](../getting-started.md)。


| 项目 | 要求 |
| ---- | ---- |
| Rust | 1.94.1+（工作区 `rust-version`，MSRV） |
| Edition | 2024（`Cargo.toml` 里写 `edition = "2024"`） |
| 数据库（可选） | PostgreSQL / MySQL / SQLite |
| Redis（可选） | 6.0+（只有缓存要跨进程共享时才需要） |

**实战提问：我现在只想跑个 Web 服务，必须装数据库吗？**
不必。`[database] enabled = false`（默认就是 false）时框架完全不碰数据库，启动流程照常。

**实战提问：学习阶段推荐哪种数据库？**
SQLite。`name = "app.db"` 就是一个文件，不用装服务、不用建库建用户，做完删文件即清空。

> 门面 crate `alun` 当前版本 `0.1.x`；`Cargo.toml` 必须用 `edition = "2024"`。

---

# 2. 第 2 步：把 alun 加进项目

## 2.1 最省事的写法

```toml
[dependencies]
alun = { version = "0.3" }   # 默认 = core：路由 + 中间件 + 配置 + 日志，零外部重量依赖
tokio = { version = "1", features = ["full"] }
```

**0.3 起默认不是 `full` 了** —— 默认是最小可用的 `core`（HTTP 服务），
能力一层层按需叠加：数据库、缓存、模板、插件、Kafka、异步任务、文件存储都要显式开。
好处是编译快、依赖面小；想「什么都有」就写预设：

```toml
alun = { version = "0.3", features = ["mini"] }   # core + SQLite + 缓存，零外部服务（原型 / 小工具 / 测试）
alun = { version = "0.3", features = ["lite"] }   # core + PostgreSQL + 缓存 + 插件 + 本地存储（单库生产推荐）
alun = { version = "0.3", features = ["full"] }   # 以上 + MySQL + Kafka + 任务(Kafka) + MinIO + SSE + 可观测 + XLSX + 模板
```

> 模板（`template`）不属于任何预设：它是纯可选单点，用得到就写 `features = ["lite", "template"]`。

## 2.2 想快一点：只开你要的 feature

依赖越少编译越快。按需求单点叠加，**不用写 `default-features = false`**：

```toml
# 只要 HTTP 服务（什么都不加）
alun = { version = "0.3" }

# HTTP + 数据库（注意：驱动必须单独挑一个，见下）
alun = { version = "0.3", features = ["postgres"] }

# 数据库 + 缓存 + 模板
alun = { version = "0.3", features = ["postgres", "cache", "template"] }

# 实时推送 + 指标导出 + Excel
alun = { version = "0.3", features = ["postgres", "cache", "sse", "metrics-prometheus", "xlsx"] }
```

> **`default-features = false` 已经不需要了**（0.1/0.2 时代默认 `full` 时的遗留写法）。
> 0.3 默认就是最小可用的 `core`，而 `core` → `config` → `alun-web/config` 这条链**当前是空标记**，
> 全仓库没有任何 `#[cfg(feature = "config")]`；真正占编译时间的 `axum` / `tokio` / `alun-web`
> 都是**非可选依赖**，关不关 default 都会被编进来 —— 实测两种写法的依赖树**完全一致**。
> 反而关掉 default 会让 `core` 这个语义标记丢失，将来 `core` 下若新增内容会静默缺失。
> 想省编译时间，正确的做法是用 `mini` / `lite` 或直接单点 feature，而不是关 default。

| feature | 打开什么 | 什么时候需要 |
| ---- | ---- | ---- |
| `core`（`default`） | 路由 + 中间件 + 配置 + 日志，**不含任何数据库驱动** | 起步 / 只做 HTTP 接口 |
| `mini` / `lite` / `full` | 预设，能力严格递增（`mini` ⊂ `lite` ⊂ `full`） | 不想一个个点 |
| `db` | `alun-db` 门面（**能力，不含驱动**） | 要读写数据库；**必须再选一个驱动** |
| `sqlite` / `postgres` / `mysql` | 对应驱动（各自隐含 `db`） | 连 SQLite / PostgreSQL / MySQL，配置见 5.1 |
| `chrono` / `time` / `jiff` / `rust_decimal` / `bigdecimal` / `uuid` / `ulid` / `json` / `ipnetwork` | 模型的生态类型映射（自动隐含 `db`） | 模型字段用到 `DateTime` / `Decimal` / `Uuid` / `#[json]` 等 |
| `xlsx` | 真 Excel 读写（`alun-utils/xlsx`：`rust_xlsxwriter` 写 / `calamine` 读） | 要导出/导入 `.xlsx`；**不开则这些 API 不存在** |
| `xss` | `alun-utils` 的 `xss` 模块（唯一新增依赖是 `ammonia`） | 要净化用户提交的 HTML |
| `sse` | `alun-web/sse`：`Broadcaster` / `sse_json` | 站内信、进度推送、监控大屏 |
| `metrics` / `metrics-prometheus` | 埋点 / + Prometheus recorder 与 `/metrics` 端点 | 要指标、要喂 Prometheus |
| `cache` | `alun-cache`（本地 / Redis） | 要缓存、要 JWT 黑名单、要幂等键/防重放 |
| `template` | `alun-template`（minijinja / Jinja2 语法） | 服务端渲染页面 |
| `plugin` | `alun-plugin`（调度器 / 通知 / 单号） | 要用内置插件 |
| `fs` | `alun-fs`（本地 / MinIO / S3 多后端） | 文件上传下载要多后端 |
| `kafka` / `task` / `task-kafka` | Kafka 收发 / 异步任务模型 / 含 Kafka worker（后者需 CMake + C 工具链） | 消息、可靠异步任务 |
| `config` | 配置文件加载（`core` 已隐含） | 基本都需要 |




**实战提问：我只写了一个 hello world，编译怎么那么久？**
因为用了 `full` —— 它把三套数据库驱动、Kafka（还要 CMake + C 工具链）、模板引擎全编进来了。
hello world 直接写 `alun = { version = "0.3" }`（默认 `core`，只含 HTTP + 配置 + 日志），
并把「用到哪个再加哪个」当默认习惯。

**实战提问：加 feature 之后一直报找不到类型（比如 `sql!`）。**
feature 与 API 面都只在 `alun` 上：`db` 打开数据库**能力**（**不含任何驱动**，
还要再选 `sqlite` / `postgres` / `mysql` 之一），Model derive、`CrudExt`、
`sql!`、`Hook`、`Value` / `Order` / `Cursor` 由 `alun::prelude::*` 直接提供——**不需要额外声明 `alun-db`**：

```toml
[dependencies]
# 驱动、生态类型都在 alun 上选
alun = { version = "0.3", features = ["postgres", "rust_decimal"] }
```

`db()` / `Row` / `Page` / `PageQuery` / `PageResponseExt` 也由 `alun::prelude::*` 提供。
路由宏（`#[alun::get]` 等）与任务宏 `#[alun::task_handler]` 的编译期注册项由门面转发 `inventory`
（生成代码写 `::alun::inventory::submit! { … }`），**不需要**自己声明 `inventory`。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [返回总目录](../getting-started.md) | [② 配置与启动](config-basics.md) → |

<sub>本分册属于「从零到跑起来（8 步分册）」，共 8 篇。首篇 / 末篇可点「返回总目录」。</sub>

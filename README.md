# Alun Framework

> **面向 AI Coding 的 Rust Web 框架** —— 配置驱动 · 插件架构，让「一个接口 = 一个 `async fn` + 一个宏注解」。
>
> **极小化 Token 消耗，极大化 Attention 浓度。**

文档：[docs/](./docs/) · 示例：[examples/](./examples/) · AI 速查：[skill-alun/SKILL.md](./skill-alun/SKILL.md)

> 🔗 **致敬 [Aifei](https://github.com/jfinal/aifei)**（[官网 aifei.cn](https://aifei.cn)）——世界首个 AI Coding 框架，JFinal 作者专注极简 15 年。
> Alun 承接其 **Just Service** 设计理念，在 Rust 生态用编译期零成本抽象实现同一目标（Rust 侧称 **Just Config**）。
> [设计理念的 Rust 表达：完整映射与对比 →](./docs/architecture/aifei-tribute.md)

## 简介

Alun 开创 Rust 下的 **Just Config** 范式：一行 `use alun::prelude::*` 覆盖路由、配置、数据库、响应、错误的全部 API，一个业务模块只留 `handler / dto / model` 三层——没有 Service / Repository / Mapper，没有散落各处的注册样板。

零成本抽象（纯 trait + 泛型，编译期展开），内核 `alun-core` 零 HTTP、零数据库依赖。从宏路由 `scan()` 到 Inventory 自动注册，专注「让 AI 写业务像写脚本一样直接」。

**Just Config. Just Code. Only Alun can do.**

## 为什么是 Alun

AI 写代码正从辅助走向主导，开发者角色转向决策与审查。但从框架层面为 AI Coding 设计的 Rust 服务端 framework 仍然缺位——传统框架面向人类手写代码时代，Alun 面向 AI Coding 时代。

框架需要优先满足大模型对代码结构的需求：**更少的 Token 消耗、更高的 Attention 浓度、更低的上下文噪音、更稳定的生成模式**。框架不再只是人类组织代码的工具，也会成为大模型理解与维护代码的上下文结构。

## 快速上手

```toml
alun = { version = "0.3", features = ["lite"] }   # lite = PostgreSQL + 缓存 + 插件 + 本地存储
tokio = { version = "1", features = ["full"] }
```

`alun` 是**唯一的框架依赖**——路由、配置、Model 派生宏都从 `alun::prelude::*` 来，`Cargo.toml` 里不需要出现 `alun-db` / `alun-web`。

```rust
use alun::prelude::*;

#[alun::get("/")]
async fn hello() -> Res<String> {
    Res::ok("Hello, alun!".into())
}

#[tokio::main]
async fn main() {
    App::new()
        .expect("初始化失败")
        .parse_cli()   // 支持 cargo run -- gen-config / print-config / profile=prod
        .scan()        // ← 自动发现上面所有 handler，不用写注册代码
        .start()
        .await
        .unwrap();
}
```

`scan()` 收走整个 crate 里所有 `#[alun::get/post/put/patch/delete]` 标注的 handler，路由分布几个文件都无所谓。响应统一包成 `{ code: 0, msg: "ok", data: … }`；5xx 对前端只显示「服务器内部错误」，细节只进日志。

📖 **深入**：[从零到跑起来（8 步分册）](./docs/guides/getting-started.md) ｜ [路由四种写法与双通道](./skill-alun/references/routing.md) ｜ [第一个接口完整示例](./docs/guides/startup/first-endpoint.md) ｜ [配置全字段参考](./docs/guides/startup/config-reference.md)

## 数据库

选方言即可，API 完全一样。**驱动要单独选**（`db` 只是「数据库能力」，不含驱动）：

```toml
alun = { version = "0.3", features = ["postgres"] }   # 或 sqlite / mysql
```

```rust,ignore
#[derive(Model)]
#[table("invoices")]
pub struct Invoice {
    #[key] pub id: i64,
    #[tenant] pub tenant_id: i64,
    #[unique] pub no: String,
    pub amount: Decimal,                // 金额用 Decimal，别用 f64
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

async fn demo() -> DbResult<()> {
    let inv = Invoice { 
                     id: 0, 
                     tenant_id: 1, 
                     no: "INV-1".into(),
                    amount: Decimal::new(19999, 2),
                    created_at: Utc::now(), 
                    updated_at: Utc::now() 
    };
    // 新增
    inv.insert(db()).await?;                              // Hook 生效、自增 id 回填
    
    // 分页查询
    let page: Page<Invoice> = Invoice::query().paginate(db(), 1, 20).await?;
    // 更新
    Invoice::of(inv.id).set(Invoice::amount(), Decimal::new(29999, 2))
        .update(db()).await?;                             // 只改被 set 的列
    
    Ok(())
}
```

表名、主键、时间戳、软删除都有默认约定。错误直通：handler 恒为 `ResResult<T>`，数据层的 `DbError` 直接 `?` 上来（`From<DbError> for ApiError`），不用写一行 `map_err`。

**多租户（SaaS）**：开 `[tenant].enabled = true` 后 `App` 自动挂载租户层——从表自动注入租户列、上下文缺失直接报错、原生 SQL 需显式承诺完整性。**不要手写 `AND tenant_id = ?`**。

📖 **深入**：[数据库实操（7 分册）](./docs/guides/database.md) ｜ [模型属性速查](./docs/guides/db/modeling.md) ｜ [增删改查照抄](./docs/guides/db/crud.md) ｜ [联表/批量/动态表名](./docs/guides/db/recipes.md) ｜ [迁移引擎与踩坑](./docs/guides/db/ops.md) ｜ [多租户对照与设计要点](./docs/guides/db/multi-tenant.md) ｜ [API 速查](./docs/guides/db/appendix.md)

## 能力分层（按需 feature）

默认只是一个 HTTP 服务，能力一层层叠，不为没用的代码买单。

| 能力 | feature | 一句话 |
| --- | --- | --- |
| 认证与权限 | `core` | JWT Access/Refresh、黑名单、角色 / 权限判定、限流 |
| 缓存 | `cache` | Local / Redis / Shared，代码写法一致 |
| 模板 | `template` | minijinja（Jinja2 语法） |
| 文件存储 | `fs` / `fs-minio` | 多后端，上传下载目录自动创建 |
| 实时推送 | `sse` | 单向推送走普通 HTTP，天然复用鉴权 |
| 可观测 | `metrics` / `metrics-prometheus` | HTTP/DB/缓存埋点 + 两个租户安全指标 |
| 导出导入 | `xlsx` | 真 OOXML（多 sheet / 列宽 / 冻结首行） |
| 插件 | `plugin` | scheduler / 邮件 / 单号 / 短 ID 内置插件 |
| 异步任务 | `task` / `task-kafka` | `TaskStorage` 抽象 + 退避重试 + 死信队列 |
| 数据库 | `sqlite` / `postgres` / `mysql` | 驱动（必选其一，各自隐含 `db`） |

预设严格递增 `mini ⊂ lite ⊂ full`，也可完全按点拼。

📖 **深入**：[按能力速查（12 份 modules）](./skill-alun/SKILL.md) ｜ [认证与鉴权](./skill-alun/references/auth.md) ｜ [插件与扩展点](./skill-alun/references/plugins.md) ｜ [异步任务](./skill-alun/references/task.md) ｜ [可观测](./skill-alun/references/observability.md) ｜ [Crate 级 API 参考](./docs/reference/crates.md)

## 核心理念

**Just Config** 是 Alun 成为 AI Coding 框架的核心：消除 Service / Repository / Mapper 等非业务结构与路由注册样板，把代码收敛为「一个接口 = 一个 `async fn` + 一个宏注解」。这不只是更少代码，而是直接作用于大模型的 Token 预算与注意力分布。

**HIO 等价物**：handler（`async fn`）掌控流程，axum 提取器管输入（`Path` / `Query` / `Json`），输出统一 `Res<T>`。三者都由你定义，结构稳定可预测，AI 生成时形成一致模式。

**加业务 = 加一个目录**：`modules/user/{handler,dto,model}.rs`，`scan()` 自动收路由，不用改任何注册代码。

📖 **深入**：[推荐项目结构与代码组织规范](./docs/guides/recommended-architecture.md) ｜ [架构与分层理念](./docs/architecture/overview.md) ｜ [决策记录（ADR）](./docs/architecture/adr/)

## AI Coding

给 AI 的检索与引用规范见 [`AGENTS.md`](./AGENTS.md)（用稳定 id 引用文档、变更走 ADR + CHANGELOG + migration），机器可读索引见 [`docs/index.json`](./docs/index.json)（按 tag / summary 筛选，**按需拉取、不要整包读入上下文**）。

让模型参考 [`examples/`](./examples/) 可直接生成业务逻辑；在 Just Config 之下，已能获得稳定且高质量的生成结果。

## 构建与测试

```bash
cargo check --workspace             # 类型 / 编译检查（迭代日常用）
cargo test -p alun-docs-tests --doc # 文档门禁：66 个 Markdown 模块的 rust 代码块被真实编译
cargo test --workspace              # 完整门禁（CI 用）
./ci/check_feature_discipline.sh    # feature 分层纪律守卫生效
```

文档里的 ` ```rust ``` 代码块会被 rustdoc **真实编译**，防止 API 与代码漂移：可编译示例用 ` ```rust,no_run `，示意片段用 ` ```rust,ignore `。

## 文档

| 入口 | 内容 |
| --- | --- |
| [docs/index.md](./docs/index.md) | 文档总索引（按场景找文档）｜ 机器可读版 [index.json](./docs/index.json) |
| [guides/getting-started.md](./docs/guides/getting-started.md) | 从零到跑起来（8 分册） |
| [guides/database.md](./docs/guides/database.md) | 数据库实操（7 分册） |
| [reference/crates.md](./docs/reference/crates.md) | 21 个 crate 的 API 参考（16 分册） |
| [skill-alun/SKILL.md](./skill-alun/SKILL.md) | 编码速查手册（人与 AI 通用，12 份按能力拆的 references） |
| [CHANGELOG.md](./CHANGELOG.md) | 版本变更日志 |
| [migrations/v0.3.0-breaking.md](./docs/migrations/v0.3.0-breaking.md) | 0.3.0 破坏性变更与升级指南 |
| [architecture/aifei-tribute.md](./docs/architecture/aifei-tribute.md) | 致敬 [Aifei](https://github.com/jfinal/aifei)——设计理念的 Rust 表达（完整映射与对比） |
| [glossary.md](./docs/glossary.md) | 术语表 |

## 技术栈

Rust 1.94.1（Edition 2024）· Tokio · Axum · SQLx 风格自研 ORM · `inventory` 编译期注册 · minijinja · `tracing`

## 联系方式

技术交流、问题反馈或商务合作，欢迎扫码添加微信：

<img src="docs/images/wechat-qrcode.jpg" alt="微信二维码" width="180" />

## License

MIT

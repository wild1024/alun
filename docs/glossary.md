---
id: glossary
title: 术语表（Glossary）
summary: alun 文档中反复出现的核心术语与同义词统一表，供人类与 AI 检索时对齐语义。
tags: [glossary, terms, conventions]
status: active
owner: alun-core-team
source_of_truth: docs
depends_on: [architecture.overview]
last_reviewed: 2026-10-06
---

# 术语表（Glossary）

alun 文档里反复出现的核心概念，以及**应当统一使用的同义词**。AI 检索时若命中这些词，优先跳到对应文档的锚点。

## 框架与架构

- **Alun**：世界首个面向 AI Coding 的 Rust Web 框架，配置驱动 + 插件架构。
- **alun-core**：内核 crate，零 HTTP / 零数据库依赖，仅含类型与抽象；是「Just Config」范式得以成立的底座。
- **feature 预设（mini / lite / full）**：能力分层。`mini ⊂ lite ⊂ full`，`lite` 不含 `template`，`full` 才含数据库驱动与全部插件。
- **Inventory / linkme**：路由注册机制。0.3.0 起用 `inventory` 取代 `linkme`，配合 `#[alun::routes]` 与 `App::module()` 双通道（见 `ADR-0001`）。

## 请求与响应

- **handler / 接口 / 路由**：同义。一个 `async fn` + 一个宏注解（`#[alun::get]` 等）即一个接口。
- **ResResult\<T\>**：handler 的**唯一**出口类型；`DbError` / `WebError`(已删) / IO 错误经 `From` 汇入 `ApiError`。
- **Res\<T\>**：统一响应信封（`ok` / `err` / `page` 等构造器）。
- **ApiError**：框架与业务唯一可抛错误类型，含状态码与脱敏消息；`DbError::NotFound` → 404、`UniqueViolation` → 409 等映射见 `reference/crates.md`。
- **ChangeSet（变更集）**：`Model::of(id)` 返回的可变变更单元，免 SELECT 直接发 `UPDATE` / `DELETE`，软删模型自动改写。

## 数据访问

- **Model 模式 / Row 模式**：类型化模型（`#[derive(Model)]`）vs 动态行（`Row`），无类型表 / 报表 / 迁移用 Row。
- **ModelQuery**：结构化查询构造器（链式）。多租户场景下，联查的从表会**自动注入租户条件**。
- **DbError**：数据访问层错误类型（`alun-db-core`），签名用 `DbResult`；在 handler 层经 `?` 直通为 `ApiError`。
- **tenant / 租户 ≈ Schema（schema_per_tenant 场景）**：当开启 `schema_per_tenant` 时，一个租户对应一个数据库 schema，是物理隔离单元；`#[model(schema)]` 与 `schema_per_tenant` 优先级见 `guides/db/multi-tenant.md` §6.7。
- **多租户三通道**：A. `ModelQuery::join::<U>()`（结构化，推荐）/ B. 原生 SQL `/*tenant:别名*/` + `raw_tenant_annotated()`（需显式承诺）/ C. `tenant_scope(TenantContext::Cross, ..)`（跨租户，需明确意图）。
- **fail-closed（默认拒绝）**：安全模型——缺少租户上下文直接报错，而非静默放行；原生 SQL 必须显式承诺租户，鉴权作用域不匹配直接拒绝。
- **TenantContext（枚举，非 Option）**：用 `enum` 表达「当前租户 / 跨租户 / 未定」等状态，避免 `Option<TenantId>` 把「无租户」和「跨租户」混为一谈。
- **Migrator**：v2 迁移引擎单一入口；方言感知、记录表 `(kind, name)`、`reset` / `fresh` 安全门。

## 插件与扩展

- **scan()**：自动发现当前编译单元内所有注解 handler / 路由，无需手写注册。
- **App::module() / #[alun::routes]**：显式模块路由通道（与 `scan()` 的 Inventory 自动通道并列）。
- **WebPlugin / 中间件 / layer**：扩展点三层；`on_startup`、请求钩子、行级异步 Hook 见 `skill-alun/references/extension-points.md`。

## 文档与流程

- **ADR（Architecture Decision Record）**：架构决策记录，见 `docs/architecture/adr/`；不修改历史，只追加。
- **migration 指南**：破坏性变更的去向，见 `docs/migrations/`；对应 `CHANGELOG.md` 条目。
- **front matter / index.json**：每篇文档头部的 YAML 元信息，与机器可读索引 `docs/index.json` 一一对应，供 AI 检索。

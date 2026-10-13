---
id: docs.index
title: 文档总索引（Documentation Index）
summary: alun 文档人类可读目录，按 guides/reference/architecture/migrations 组织，能力模块指向 skill-alun/references，并给出按场景找文档表。
tags: [index, toc]
status: active
owner: alun-core-team
source_of_truth: docs
depends_on: []
last_reviewed: 2026-10-06
---

# Alun 文档总索引

> 配置驱动 · 插件架构的 Rust Web 框架文档体系。版本：`alun 0.3`。
> 机器可读索引见 [index.json](./index.json)；AI 检索规范见 [AGENTS.md](../AGENTS.md)。

## 文档怎么组织（四层 + 模块）

- **guides/**：任务导向教程，按顺序读即可上手（`getting-started` / `database` / `fullstack` / `recommended-architecture`）。其中 `getting-started` 与 `database` 为总目录，实体内容分别按流程步骤拆在 `guides/startup/`（8 个分册）与 `guides/db/`（7 个分册）。
- **reference/**：API 与依赖事实（`crates` 为总索引，其下 `crates/` 按 crate 拆成独立模块文件；`dependencies`）。
- **architecture/**：架构总览 + [致敬 Aifei](architecture/aifei-tribute.md) + 决策记录 `adr/`（不修改历史，只追加）。
- **migrations/**：破坏性变更的迁移指南，与 `CHANGELOG.md` 一一对应。
- **模块（按能力查询）= `skill-alun/references/`**：路由 / 配置 / 数据库 / 进阶 DB / 认证 / 插件 / 任务 / 工具 / 扩展点 / SSE / 可观测 / 导出导入。**想要查某项功能，直接看对应模块，不必通读大文档。**
- **glossary.md**：术语表（`ResResult` / `ApiError` / `ChangeSet` / `ModelQuery` / `TenantContext` / `fail-closed` 等）。
- **CHANGELOG.md**：全局版本变更日志。

## 第一次接触（按顺序，约 1 小时）

| 顺序 | 文档 | 你会得到什么 |
| ---- | ---- | ---- |
| 0 | [根 README.md](../README.md) | 一分钟跑起来：依赖怎么选、第一个接口怎么写、能力分七层 |
| 1 | [guides/getting-started.md](guides/getting-started.md) | 入门总目录：下挂 8 个分册（环境依赖 / 配置启动 / 第一个接口 / 中间件 / 钩子插件 / 自定义配置 + 配置参考 + 安全模型）|
| 2 | [guides/database.md](guides/database.md) | 数据库总目录：三步跑通（配库 → 建模 → 增删改查），下挂 7 个按主题拆分的分册 |
| 3 | [examples/](../examples/) | 可 `cargo run` 的示例：`00-quick-start` … `04-test-suite` |
| 4 | [skill-alun/SKILL.md](../skill-alun/SKILL.md) | 编码速查手册（人和 AI 通用）+ 12 份 references |

## 进阶（把框架用熟）

| 顺序 | 文档 | 你会得到什么 |
| ---- | ---- | ---- |
| 5 | [architecture/overview.md](architecture/overview.md) | 整体架构、设计理念、分层、与 Java 系框架差异、feature 预设 |
| 6 | [guides/startup/security-model.md](guides/startup/security-model.md) | 认证 / 鉴权 / 多租户隔离安全模型与自检清单 |
| 7 | [reference/crates.md](reference/crates.md) | Crate 参考总索引（分层全景 + 模块映射）；每个 crate 的详细 API 在 `reference/crates/` 下有独立文件，查哪个读哪个 |
| 8 | [reference/dependencies.md](reference/dependencies.md) | 内部依赖关系、外部核心依赖、feature 传递链 |
| 9 | [guides/fullstack.md](guides/fullstack.md) | 全栈技术选型与前后端约定 |
| 10 | [skill-alun/references/](../skill-alun/references/) | 按主题拆的深水区：路由 / 数据库 / 进阶 DB / 认证 / 配置 / 插件 / 任务 / 工具 / SSE / 可观测 / 扩展点 / 导出导入 |

## 设计与演进（想知道「为什么」再读）

| 文档 | 内容 |
| ---- | ---- |
| [architecture/aifei-tribute.md](architecture/aifei-tribute.md) | **致敬 [Aifei](https://github.com/jfinal/aifei)**（世界首个 AI Coding 框架）——Just Service → Just Config 的逐项映射、Rust 侧增益与性能对比 |
| [migrations/v0.3.0-breaking.md](migrations/v0.3.0-breaking.md) | 0.3.0 破坏性变更总表、迁移对照、F1–F20 落地状态、关键安全修复、feature 分层、验证矩阵 |
| [architecture/adr/ADR-0001-inventory-over-linkme.md](architecture/adr/ADR-0001-inventory-over-linkme.md) | 路由注册由 linkme 切 inventory + 双通道的决策与后果 |
| [architecture/adr/ADR-0002-fail-closed-tenant-isolation.md](architecture/adr/ADR-0002-fail-closed-tenant-isolation.md) | 多租户 fail-closed 隔离模型的决策与后果 |
| [guides/recommended-architecture.md](guides/recommended-architecture.md) | 推荐项目目录结构与代码组织规范 |

## 按需求找文档（想要查什么就查什么）

| 我想… | 看这里（模块） |
| ---- | ---- |
| 写第一个接口 / 路由四种写法怎么选 | `skill-alun/references/routing.md` |
| 统一响应 / 分页 / `ResResult` / `ApiError` | `skill-alun/references/routing.md` + [guides/startup/first-endpoint.md](guides/startup/first-endpoint.md) |
| 建表 / `Model` / 增删改查 / 事务 / 迁移 | [guides/db/modeling.md](guides/db/modeling.md) + [guides/db/crud.md](guides/db/crud.md) + `skill-alun/references/database.md` |
| 联查 / 排序白名单 / 原生 SQL 租户安全 / 多数据源 | `skill-alun/references/advanced-db.md` + [guides/db/recipes.md](guides/db/recipes.md) |
| 登录 / JWT / 角色权限 / 中间件顺序 | [guides/startup/security-model.md](guides/startup/security-model.md) + `skill-alun/references/auth.md` |
| 多租户（SaaS） | [guides/db/multi-tenant.md](guides/db/multi-tenant.md)（逐项对照 + 三个安全默认值 + 隔离设计要点）+ `skill-alun/references/advanced-db.md` |
| 表结构演进 / 接存量库 / 迁移引擎 | [guides/db/ops.md](guides/db/ops.md) §迁移（`Migrator`、`import_legacy`/`baseline`） |
| 数据库报错怎么变成 HTTP 响应 | [guides/db/ops.md](guides/db/ops.md) §错误映射 |
| 出错了、结果不对 | [guides/db/ops.md](guides/db/ops.md) §新手最常踩的坑 |
| 缓存 / 模板 / 内置插件 | `skill-alun/references/plugins.md` |
| 启动钩子 / 自定义中间件 / `WebPlugin` / 异步 Hook | `skill-alun/references/extension-points.md` |
| 指标埋点 / Prometheus / 安全指标 | `skill-alun/references/observability.md` |
| SSE 实时推送 | `skill-alun/references/realtime-sse.md` |
| 导出 / 导入 Excel | `skill-alun/references/export-and-import.md` |
| 定时任务 / 分布式异步任务 / Kafka | `skill-alun/references/task.md` |
| 工具函数（ID / 脱敏 / 校验 / 加密 / 单号） | `skill-alun/references/utils.md` |
| 配置驱动 / 自定义配置 | `skill-alun/references/config.md` |
| 某个 crate 的完整公开 API / feature 标志 | [reference/crates/](reference/crates/) 下同名模块（如 `web.md` / `db.md` / `task.md` / `core.md`） |

## 文档校验

文档里的 Rust 代码块会被 rustdoc 真实编译，防止文档与代码漂移：

```bash
cargo test -p alun-docs-tests --doc
```

约定：可编译示例用 ` ```rust,no_run `，纯示意片段用 ` ```rust,ignore `，其他语言必须显式打标记。
新增文档时在 `docs-tests/src/lib.rs` 里加一个 `#[doc = include_str!("../../<路径>")]` 模块即可纳入校验；
并同步在 [index.json](./index.json) 增加一条 entry（含 front matter 的 `id` / 摘要 / 标签 / `source_of_truth`）。

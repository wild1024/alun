---
id: reference.crates
title: Crate 级 API 参考（索引）
summary: 21 个 workspace crate 的分层全景与索引表；每个 crate 的详细 API 已拆成独立模块文件，按需查阅。
tags: [reference, crates, api, features, index]
status: active
owner: alun-core-team
source_of_truth: code
depends_on: [reference.dependencies]
last_reviewed: 2026-10-06
---

# Crate 详细参考（索引）

本文档是 **Crate 级 API 参考的总入口**。工作区共 21 个 crate，每个 crate 的详细 API（关键结构体、
Trait、函数、枚举、feature 标志）都已拆分为独立模块文件——**查哪个 crate 就只读哪个文件**，
不必通读全量参考。

> 想知道怎么写业务代码，看 [`guides/database.md`](../guides/database.md)、
 [`guides/getting-started.md`](../guides/getting-started.md)；
 想知道依赖版本与 feature 关系，看 [`dependencies.md`](dependencies.md)。

***

## 2.0 Crate 全景

工作区共 **21 个 crate**（`alun` 门面 + 20 个实现 crate；另有 `examples/` 下 5 个示例工程）。
下表按依赖分层排列，同一层内互不依赖。

```text
                    ┌───────────────────────────────────────────┐
        第 4 层     │                 alun（门面）                │
                    └───────────────────────────────────────────┘
                                        │
                    ┌───────────────────┼───────────────────┬──────────────┐
        第 3 层     │        alun-web   │  alun-plugin      │  alun-task   │  alun-kafka / alun-fs
                    └───────────────────┴───────────────────┴──────────────┘
                                        │
                    ┌───────────────────┼──────────────┬───────────────┬───────────────┐
        第 2 层     │  alun-db          │ alun-cache   │ alun-log      │ alun-template │ alun-config
                    │   + 7 个 db 子crate│              │               │               │
                    └───────────────────┴──────────────┴───────────────┴───────────────┘
                                        │
                    ┌───────────────────┴───────────────────┐
        第 1 层     │          alun-core / alun-utils         │  alun-macros（过程宏）
                    └─────────────────────────────────────────┘

    db 子层：alun-db ──▶ alun-db-core（纯类型，零异步/零驱动）
                       ├─▶ alun-db-sql（SQL 渲染 + 方言）──▶ alun-db-core
                       ├─▶ alun-db-macro（Model/Row/TypeMapper derive + sql!）──▶ alun-db-core
                       ├─▶ alun-db-build（build.rs 助手，schema 快照）
                       └─▶ alun-db-driver-{sqlite,postgres,mysql} ──▶ alun-db-core + alun-db-sql
```

| Crate                | 职责                                                       | 主要依赖                                                            | 章节   |
| -------------------- | -------------------------------------------------------- | --------------------------------------------------------------- | ---- |
| `alun`               | 门面：统一 re-export、prelude、feature 开关                        | 全部实现 crate                                                      | [facade](crates/facade.md) |
| `alun-core`          | 错误、插件协议、API 响应体（`Res`/`ApiError`）、分页公共类型（`PageQuery`/`PageData`） | —                                                               | [core](crates/core.md) |
| `alun-utils`         | 字符串/日期/脱敏/短 ID/校验/加密/单号生成/导出                                | `alun-core`（validator 可选）                                       | [utils](crates/utils.md) |
| `alun-macros`        | 过程宏：路由、`#[controller]`、`#[plugin]`、`#[permission]`、`#[no_auth]`、`#[layer]`、`#[task_handler]` | `syn`/`quote`                                                   | [macros](crates/macros.md) |
| `alun-config`        | TOML + profile + 环境变量配置体系                                  | —                                                               | [config](crates/config.md) |
| `alun-log`           | 基于 `tracing` 的日志初始化                                      | `alun-config`                                                     | [log](crates/log.md) |
| `alun-cache`         | `Cache` trait + Local/Redis/Shared 实现                      | `alun-core`、`alun-config`                                         | [cache](crates/cache.md) |
| `alun-template`      | minijinja 模板引擎                                          | `alun-core`                                                       | [template](crates/template.md) |
| `alun-db`            | 数据库门面：SQL 级 API、Row 模式、事务、Hook、`ModelQuery`/`ChangeSet`、Builder、迁移 | `alun-db-core`/`-sql`/`-macro`/三个驱动                                | [db](crates/db.md) |
| `alun-db-core`       | 核心类型与 trait（Row/Model/Hook/Dialect/Expr/Value/Page…）+ `hook` 模块（TimestampHook 等），零异步零驱动 | `thiserror`（生态类型可选）                                             | [db-family](crates/db-family.md) |
| `alun-db-sql`        | `Expr`/`Statement` → SQL 渲染与三种方言                           | `alun-db-core`                                                    | [db-family](crates/db-family.md) |
| `alun-db-macro`      | `Model`/`Row`/`TypeMapper` derive 与 `sql!`                 | `alun-db-core`（展开期）                                              | [db-family](crates/db-family.md) |
| `alun-db-build`      | `build.rs` 助手：解析迁移 SQL 生成 schema 快照                        | 无（仅 `std`）                                                        | [db-family](crates/db-family.md) |
| `alun-db-driver-sqlite` | SQLite 驱动（rusqlite，同步核心 + 池化）                              | `alun-db-core`、`alun-db-sql`、`rusqlite`                          | [db-family](crates/db-family.md) |
| `alun-db-driver-postgres` | PostgreSQL 驱动（tokio-postgres）                             | `alun-db-core`、`alun-db-sql`、`tokio-postgres`                     | [db-family](crates/db-family.md) |
| `alun-db-driver-mysql` | MySQL 驱动（mysql_async）                                    | `alun-db-core`、`alun-db-sql`、`mysql_async`                        | [db-family](crates/db-family.md) |
| `alun-web`           | App 构建器、路由、中间件、提取器、JWT                                  | `alun-core`、`alun-macros`、`alun-config`、`alun-log`、`alun-utils`（`db`/`cache`/`template` 可选） | [web](crates/web.md) |
| `alun-plugin`        | 内置插件：cache / notification / async-task / scheduler / serial / sid | `alun-core`、`alun-config`、`alun-cache`、`alun-utils`                 | [plugin](crates/plugin.md) |
| `alun-kafka`         | Kafka 生产者/消费者/插件                                          | `alun-core`、`alun-config`、`rdkafka`                               | [kafka](crates/kafka.md) |
| `alun-task`          | Kafka 驱动的异步任务框架（重试 + DLQ）                                 | `alun-core`、`rdkafka`                                             | [task](crates/task.md) |
| `alun-fs`            | 多后端文件存储（local / MinIO）                                    | `alun-core`、`alun-config`（`minio` 特性含 aws-sdk-s3）                  | [fs](crates/fs.md) |

> 编译基线：`edition = "2024"`、`resolver = "3"`、`rust-version = "1.94.1"`（详见 [`dependencies.md`](dependencies.md)）。

***


***

## 2.16 模块索引

下表把全景表中的每个 crate 映射到其独立模块文件。**按需查阅，不要整包读入上下文。**

| 模块文件 | Crate / 主题 | 一句话职责 |
| --- | --- | --- |
| [`core.md`](crates/core.md) | `alun-core` — 核心抽象层 | 错误、插件协议、响应体与分页公共类型 |
| [`facade.md`](crates/facade.md) | `alun` — 门面 Crate（prelude / feature / re-export） | 统一 re-export、prelude、feature 开关 |
| [`web.md`](crates/web.md) | `alun-web` — Web 核心层 | App 构建器、路由、中间件、提取器、JWT |
| [`db.md`](crates/db.md) | `alun-db` — 数据库门面 | SQL 级 API、Row 模式、事务、Hook、Model/Query/ChangeSet、迁移 |
| [`config.md`](crates/config.md) | `alun-config` — 配置系统 | TOML + profile + 环境变量配置体系 |
| [`cache.md`](crates/cache.md) | `alun-cache` — 缓存系统 | Cache trait + Local/Redis/Shared 实现 |
| [`template.md`](crates/template.md) | `alun-template` — 模板引擎 | minijinja 模板引擎 |
| [`utils.md`](crates/utils.md) | `alun-utils` — 工具集 | 字符串/日期/脱敏/短 ID/校验/加密/单号/导出 |
| [`macros.md`](crates/macros.md) | `alun-macros` — Proc Macro | 路由、controller、plugin、permission、task_handler 过程宏 |
| [`plugin.md`](crates/plugin.md) | `alun-plugin` — 内置插件 | cache/notification/async-task/scheduler/serial/sid 内置插件 |
| [`log.md`](crates/log.md) | `alun-log` — 日志初始化 | 基于 tracing 的日志初始化 |
| [`kafka.md`](crates/kafka.md) | `alun-kafka` — Kafka 生产者/消费者 | Kafka 生产者 / 消费者 / 插件 |
| [`fs.md`](crates/fs.md) | `alun-fs` — 多后端文件存储 | 多后端文件存储（local / MinIO） |
| [`task.md`](crates/task.md) | `alun-task` — 异步任务框架 | Kafka 驱动的异步任务（重试 + DLQ） |
| [`db-family.md`](crates/db-family.md) | `alun-db` 家族其余 Crate（core/sql/macro/build/驱动） | db 家族其余 crate：core/sql/macro/build/三驱动 |
| [`observability.md`](crates/observability.md) | 可观测 · 实时推送 · 导出导入 · 扩展点 | 可观测、实时推送(SSE)、导出导入、扩展点 |

***

## 2.17 相关文档

- 数据库实操与多租户：[`guides/database.md`](../guides/database.md)
- 入门与启动方式：[`guides/getting-started.md`](../guides/getting-started.md)
- 架构与分层理念：[`architecture/overview.md`](../architecture/overview.md)
- 依赖与 feature 全表：[`dependencies.md`](dependencies.md)
- 按能力的速查手册：[`skill-alun/SKILL.md`](../../skill-alun/SKILL.md)

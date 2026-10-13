---
id: guides.database
title: 数据库指南（总目录）
summary: alun-db 数据库指南总入口：按主题拆成 7 个分册（入门/建模/CRUD/查询技巧/运维/多租户/附录），按需查阅不必通读。
tags: [database, index, toc]
status: active
owner: alun-core-team
source_of_truth: [code, alun-db/src, alun-db-core/src]
depends_on: []
last_reviewed: 2026-10-06
---

# 数据库指南（总目录）

> `alun-db`：接库 → 定义模型 → 查一条 / 查一批 / 增删改 / 分页 → 按需求查技巧 → 多租户隔离。
> 版本：`alun 0.3`。机器可读索引见 [index.json](../index.json)。

本文档已按主题拆成 7 个分册——**查什么就只读对应分册**，不必通读全文。

## 快速上手（三步跑通）

```text
① 配库      →  db/getting-started.md   （加依赖、配置驱动或代码初始化）
② 定义模型  →  db/modeling.md          （#[derive(Model)]，约定替你写掉大部分）
③ 增删改查  →  db/crud.md              （六件事照着抄，覆盖 90% 需求）
```

## 按需求找分册

| 分册 | 你会得到什么 |
| --- | --- |
| [`db/getting-started.md`](db/getting-started.md) | 加依赖 → 配库 → 跑通第一个接口（两种初始化方式、db() 语义） |
| [`db/modeling.md`](db/modeling.md) | `#[derive(Model)]` 最小模型与属性速查 |
| [`db/crud.md`](db/crud.md) | 查一条/查一批/新增/改/删/分页/计数——照着抄即可 |
| [`db/recipes.md`](db/recipes.md) | 搜索条件、联表聚合、多表操作、批量导入/upsert、动态表名 |
| [`db/ops.md`](db/ops.md) | 只读视图、审计与 Hook、迁移引擎、多数据库、错误映射、踩坑清单 |
| [`db/multi-tenant.md`](db/multi-tenant.md) | 多租户逐项对照、三个安全默认值、schema-per-tenant、隔离设计要点 |
| [`db/appendix.md`](db/appendix.md) | API 签名速查、反模式清单、已删除/改名 API |

## 三个最常被问到的场景

| 我想… | 看这里 |
| --- | --- |
| 做 SaaS 多租户隔离、担心数据泄漏 | [db/multi-tenant.md](db/multi-tenant.md)（含三个 fail-closed 安全默认值、隔离设计要点） |
| 写表结构演进、接存量库 | [db/ops.md](db/ops.md) §迁移（`Migrator` 方言感知、`import_legacy`/`baseline`） |
| 联表 / 聚合 / 批量导入 / 动态表名 | [db/recipes.md](db/recipes.md) |
| 查某个 API 的准确签名 | [db/appendix.md](db/appendix.md) |
| 数据库报错怎么变成 HTTP 响应 | [db/ops.md](db/ops.md) §错误映射 |
| 出错了、结果不对 | [db/ops.md](db/ops.md) §新手最常踩的坑 |

## 相关文档

- 框架入门与启动方式：[`guides/getting-started.md`](getting-started.md)
- Crate 级 API（`Db` / `Transaction` / `ModelQuery` 逐项签名）：[`reference/crates/db.md`](../reference/crates/db.md)
- 迁移与破坏性变更：[`migrations/v0.3.0-breaking.md`](../migrations/v0.3.0-breaking.md)
- 编码速查手册：[`skill-alun/references/database.md`](../../skill-alun/references/database.md)
- `alun-db` 设计动机：[`alun-db/README.md`](../../alun-db/README.md)

## 文档校验

分册里的 Rust 代码块会被 rustdoc 真实编译（防止文档与代码漂移）：

```bash
cargo test -p alun-docs-tests --doc
```

约定：可编译示例用 ` ```rust,no_run `，纯示意片段用 ` ```rust,ignore `。
新增分册时在 `docs-tests/src/lib.rs` 加 `#[doc = include_str!("../../docs/guides/db/<名>.md")]` 纳入校验。

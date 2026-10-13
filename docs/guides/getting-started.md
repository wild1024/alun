---
id: guides.getting-started
title: 从零到跑起来（总目录）
summary: alun 入门总入口：按 8 步流程拆成 8 个分册（环境/配置/接口/中间件/钩子插件/自定义配置 + 配置参考 + 安全模型），按需查阅不必通读。
tags: [getting-started, index, toc]
status: active
owner: alun-core-team
source_of_truth: [code, examples, config/config.toml]
depends_on: []
last_reviewed: 2026-10-06
---

# 从零到跑起来（总目录）

> 配置驱动 · 插件架构的 Rust Web 框架。版本：`alun 0.3`（MSRV 1.94.1，Edition 2024）。
> 机器可读索引见 [index.json](../index.json)；按能力查 API 见 [`skill-alun/SKILL.md`](../../skill-alun/SKILL.md)。

本文档已按流程步骤拆成 8 个分册——**查什么就只读对应分册**，不必通读全文。

## 全流程一览

```text
① 环境与依赖   →  startup/setup.md            （Rust 版本 + Cargo.toml + feature 选择）
② 配置与启动   →  startup/config-basics.md    （config.toml + 三种启动方式）
③ 数据库与接口 →  startup/first-endpoint.md   （接库 + 第一个路由 + Res<T>）
④ 中间件       →  startup/middleware.md       （能配就别写代码）
⑤ 钩子与插件   →  startup/hooks-plugins.md    （on_startup / 何时值得写插件）
⑥ 自定义配置   →  startup/custom-config.md    （[custom] 段 + 类型安全封装）
```

## 按需求找分册

| 分册 | 你会得到什么 |
| --- | --- |
| [`startup/setup.md`](startup/setup.md) | Rust 版本、项目初始化、Cargo.toml 依赖与 feature 组合 |
| [`startup/config-basics.md`](startup/config-basics.md) | config.toml 怎么写、三种启动方式、启动时到底发生了什么 |
| [`startup/first-endpoint.md`](startup/first-endpoint.md) | 接入数据库的最小配置 + 第一个路由与统一响应 |
| [`startup/middleware.md`](startup/middleware.md) | 配置能解决多少中间件、还需要什么、自定义中间件怎么挂 |
| [`startup/hooks-plugins.md`](startup/hooks-plugins.md) | `on_startup` 什么时候用、插件何时值得写、三者关系 |
| [`startup/custom-config.md`](startup/custom-config.md) | `[custom]` 配置段写法与类型安全封装、FAQ、下一步 |
| [`startup/config-reference.md`](startup/config-reference.md) | config.toml 全字段参考（默认值 / 取值 / 作用） |
| [`startup/security-model.md`](startup/security-model.md) | 认证鉴权与多租户隔离安全模型、三条 fail-closed 底线、自检清单 |

## 三个最常被问到的场景

| 我想… | 看这里 |
| --- | --- |
| 零配置先跑起来看看 | [startup/config-basics.md](startup/config-basics.md)（`App::with_config(AppConfig::default())`） |
| 出问题了、启动就报错 | [startup/config-basics.md](startup/config-basics.md) §启动时到底做了什么（按顺序看日志） |
| 查某个配置项叫什么、默认值多少 | [startup/config-reference.md](startup/config-reference.md) |
| 加登录鉴权 / 多租户隔离 | [startup/security-model.md](startup/security-model.md) + [`guides/db/multi-tenant.md`](db/multi-tenant.md) |
| 定制自己的配置段 | [startup/custom-config.md](startup/custom-config.md) |
| 写数据库表 | [`guides/db/modeling.md`](db/modeling.md) + [`guides/db/crud.md`](db/crud.md) |

## 相关文档

- 数据库实操：[`guides/database.md`](database.md)（下挂 7 个分册）
- 架构与分层理念：[`architecture/overview.md`](../architecture/overview.md)
- Crate 级 API 参考：[`reference/crates.md`](../reference/crates.md)
- 全栈技术选型：[`guides/fullstack.md`](fullstack.md)
- 破坏性变更与升级：[`migrations/v0.3.0-breaking.md`](../migrations/v0.3.0-breaking.md)

## 文档校验

分册里的 Rust 代码块会被 rustdoc 真实编译（防止文档与代码漂移）：

```bash
cargo test -p alun-docs-tests --doc
```

约定：可编译示例用 ` ```rust,no_run `，纯示意片段用 ` ```rust,ignore `。
新增分册时在 `docs-tests/src/lib.rs` 加 `#[doc = include_str!("../../docs/guides/startup/<名>.md")]` 纳入校验。

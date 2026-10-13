---
id: reference.crates.macros
title: `alun-macros` — Proc Macro
summary: `alun-macros` — Proc Macro——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [macros, reference, proc-macro, route]
status: active
owner: alun-core-team
source_of_truth: [alun-macros/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-macros` — Proc Macro

> 本文件是 [`crates.md`](../crates.md) 的 `macros` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun-macros/src/`

## `route.rs` — 路由宏

| 宏                                               | 行为                                                                             |
| ----------------------------------------------- | ------------------------------------------------------------------------------ |
| `#[alun::get("/path")]`                         | 为 async fn 生成 `inventory::submit!(RouteEntry)` 注册项，调用 `AlunRouter::add_get` |
| `#[alun::post("/path")]`                        | 同上，`add_post`                                                                  |
| `#[alun::put("/path")]`                         | 同上，`add_put`                                                                   |
| `#[alun::patch("/path")]`                       | 同上，`add_patch`（部分更新；与 `put` 语义一致，仅 HTTP 方法不同）                                |
| `#[alun::delete("/path")]`                      | 同上，`add_delete`                                                                |
| `#[alun::controller("/base")]`                  | 扫描 impl 块中标注了 `#[get]`/`#[post]` 的方法，自动生成路由注册                                  |
| `#[alun::permission(path = "…", method = "…", permission = "…")]` | 生成 `PERMISSION_ROUTES` 切片注册项（**必须具名参数**）              |
| `#[alun::no_auth("/path")]`                     | 生成 `NO_AUTH_ROUTES` 切片注册项，标记无需认证的路径                                            |
| `#[alun::layer(L)]`                             | **路由级 layer**：写在路由注解**上方**，摘掉该路由的编译期自注册、改生成 `add_*_with_layer`（因此不会重复挂载）。`L` 可为 `axum::middleware::from_fn(..)` 等 |

## `plugin.rs` — 插件标记宏

| 宏                 | 行为                                      |
| ----------------- | --------------------------------------- |
| `#[alun::plugin]` | 为结构体添加 `alun_plugin_name()` 静态方法，返回结构体名 |

## `task_handler.rs` — 任务处理器宏（需 `task` feature）

| 宏                                           | 行为                                                                   |
| ------------------------------------------- | -------------------------------------------------------------------- |
| `#[alun::task_handler(task_type = N, ...)]` | 生成 `inventory::submit!(TaskHandlerEntry)` 注册项，编译期自动发现并注册 handler |

***

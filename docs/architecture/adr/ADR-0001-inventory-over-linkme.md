---
id: architecture.adr.inventory
title: ADR-0001 路由注册改用 inventory 取代 linkme
summary: 0.3.0 将路由自动注册由 linkme 切换为 inventory，并提供 #[alun::routes] / App::module() 双通道。
tags: [adr, routing, inventory, linkme, decision]
status: accepted
owner: alun-core-team
source_of_truth: decision
depends_on: [architecture.overview]
last_reviewed: 2026-10-06
---

# ADR-0001 路由注册改用 inventory 取代 linkme

- 状态：Accepted（已采纳，0.3.0 生效）
- 日期：2026-10-06
-  supersedes：2.0 的 linkme 方案

## 背景

alun 的「Just Config」范式要求 handler 写完即生效，不手写注册代码。`scan()` 早期依赖 `linkme` 做跨 crate 的分布式注册。实践中遇到三类问题：

1. `linkme` 在 wasm / 部分静态链接目标上行为不一致，且对 `used` 属性与链接顺序敏感，偶发「路由丢失」。
2. 自动注册无法表达「显式分组 / 版本前缀 / 按域聚合」这类有意为之的结构，大型项目被迫把路由打散到零散 `#[alun::get]`。
3. 调试困难：注册的来源不可见，CI 难以静态断言「某前缀下有哪些路由」。

## 决策

路由注册改为 **`inventory`**（基于链接器节的分布式注册，语义更稳定），并引入**双通道**：

- **通道 A（Inventory 自动）**：`App::new().scan()` 收集整个编译单元内所有 `#[alun::get/post/...]` 注解的 handler，行为与旧 `scan()` 一致。
- **通道 B（显式模块）**：用 `#[alun::routes]` 标注一个路由聚合体，再以 `App::module(Routes)` 显式挂载，支持前缀、分组与可测试性。

旧 `linkme` 依赖从 `alun-web` / `alun-macros` 移除。

## 后果

正向：

- 注册行为在更多目标三元组上稳定，CI 可静态断言路由集合。
- 显式通道让「按域聚合成 Controller」成为一等公民，大型项目结构更清晰。
- 两条通道可混用：自动 `scan()` 兜底散落 handler，显式 `module()` 组织核心域。

负向 / 成本：

- 既有代码若依赖 `linkme` 的内部符号需迁移到 `#[alun::routes]` / `App::module()`。
- 宏实现复杂度上移，文档需同时讲清两条通道（见 `skill-alun/references/routing.md` 路由四种写法、原 README「路由双通道」）。

## 备选方案

- 保留 linkme：稳定性与可调试性不达标，否决。
- 纯手写注册（axum Router 风格）：与「Just Config」范式冲突，否决。
- 完全自动、取消显式通道：大型项目分组诉求无法满足，否决。

## 参考

- `docs/migrations/v0.3.0-breaking.md` §5 注册机制双通道
- `skill-alun/references/routing.md`
- `skill-alun/references/routing.md`（路由四种写法）

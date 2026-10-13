---
id: architecture.adr.fail-closed
title: ADR-0002 多租户隔离采用 fail-closed 安全模型
summary: 多租户隔离默认拒绝：无租户上下文即报错、原生 SQL 必须显式承诺租户、鉴权作用域不匹配即拒绝。
tags: [adr, multi-tenant, security, fail-closed, decision]
status: accepted
owner: alun-core-team
source_of_truth: decision
depends_on: [architecture.overview, guides.database]
last_reviewed: 2026-10-06
---

# ADR-0002 多租户隔离采用 fail-closed 安全模型

- 状态：Accepted（已采纳，0.3.0 生效）
- 日期：2026-10-06
- 动机：封住 2.0 时期的 F2 租户泄漏通道

## 背景

多租户 SaaS 下，租户泄漏 = 数据越权。2.0 的实现把「租户条件」当成**可选装饰**：缺少上下文时静默放行、原生 SQL 不强制注入租户、鉴权作用域不匹配时降级放行。这导致 F2 类泄漏——某租户读到了另一租户的数据。

核心设计原则（来自 `guides/db/multi-tenant.md` §6.7 设计要点）：

- 复刻 `SOFT_DELETE` 的「框架层统一兜底」范式，而非把租户过滤推给每个开发者。
- 查询条件用**结构化的 `Statement` 字段**（非 `Expr` 字符串），租户是其中一等字段。
- 租户上下文用 `task_local`（非 `thread_local`），契合 async 运行时的执行流。
- 租户状态用 `enum TenantContext`（非 `Option<TenantId>`），显式区分「当前租户 / 跨租户 / 未定」。

## 决策

隔离默认**拒绝（fail-closed）**，落地三条底线：

1. **无租户上下文即报错**：需要租户作用域的查询在 `TenantContext` 缺失时直接返回错误，而不是「查全部」。
2. **原生 SQL 必须显式承诺租户**：手写 SQL 用 `/*tenant:别名*/` 注记 + `raw_tenant_annotated()` 声明已自行处理租户；未注记的原生 SQL 在需要租户的作用域下被拒绝执行。
3. **鉴权作用域不匹配即拒绝**：JWT 声明的作用域（如 `cache_tenant()` 命名空间、`TokenClaims` 的 `extra`）与当前请求租户不匹配时，直接 403，不降级。

并通过安全可观测性暴露两类计数器：`alun_tenant_scope_missing_total`（底线 1 触发次数）、`alun_raw_sql_tenant_unbound_total`（底线 2 触发次数）。

## 后果

正向：

- 从机制上封死 F2 泄漏：结构化联查（通道 A `ModelQuery::join::<U>()`）的从表自动注入租户，开发者无法「忘记写 where tenant_id」。
- 任何泄漏尝试都可观测、可报警，而非静默发生。
- 跨租户操作（通道 C `tenant_scope(TenantContext::Cross, ..)`）成为**显式、可审计**的少数路径。

负向 / 成本：

- 老代码里「静默查全部」的写法在 0.3.0 会直接报错，需按 `docs/migrations/v0.3.0-breaking.md` 迁移。
- 原生 SQL 必须加租户注记，书写成本略增；但避免了越权事故成本。

## 备选方案

- fail-open（默认放行，缺上下文就查全部）：正是 F2 泄漏根因，否决。
- 仅在 ORM 层兜底、原生 SQL 不管：覆盖不全，否决。
- 租户用 `Option<TenantId>` 表达：无法区分「无」与「跨」，否决（改用 `enum`）。

## 参考

- `docs/migrations/v0.3.0-breaking.md` §4 关键安全修复
- `guides/db/multi-tenant.md` 多租户逐项对照、§6.7 设计要点
- `docs/glossary.md`（fail-closed / TenantContext / 多租户三通道）

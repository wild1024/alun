---
id: guides.startup.security-model
title: 附录：认证、鉴权与多租户隔离安全模型
summary: 一句话模型、三条 fail-closed 底线、ClaimsBuilder 签发与 extra 访问、鉴权中间件、TokenClaims 缓存命名空间、安全自检清单。
tags: [getting-started, auth, jwt, multi-tenant, fail-closed, security]
status: active
owner: alun-core-team
source_of_truth: [code, examples, config/config.toml]
depends_on: [guides.getting-started]
last_reviewed: 2026-10-06
---

# 附录：认证、鉴权与多租户隔离安全模型

> 本文件是 [`getting-started.md`](../getting-started.md) 的 `security-model` 分册；总目录与全流程顺序见 [从零到跑起来](../getting-started.md)。

# 附录：认证、鉴权与多租户隔离安全模型

> 本节能帮你建立「请求进来后，谁在哪一刻决定了能不能看哪些数据」的整体心智。多租户的日常写法见 [db/multi-tenant.md](../db/multi-tenant.md)；0.3 破坏性变更与升级细节见 [migrations/v0.3.0-breaking.md](../../migrations/v0.3.0-breaking.md)。

## 一句话模型

```text
请求 → AuthLayer(JWT 校验/黑名单) → TokenClaims → TenantLayer(租户上下文)
     → RequirePermission(鉴权) → handler → ModelQuery(结构化隔离)
                                         └→ 原生 SQL(必须显式承诺)
```

## 三条 fail-closed 底线（默认安全，宁可报错不可泄露）

1. 租户模型在**无租户上下文**时查询 → 报错，绝不静默放行全量；
2. 租户模型的**原生 SQL** 在 `Scoped` 上下文下 → 报错，除非显式 `raw_tenant_annotated()` 承诺完整性；
3. 鉴权规则未匹配到放行项 → 拒绝。

## 认证：用 `ClaimsBuilder` 签发带扩展声明的 token

`TokenClaims` 在 8 个固定字段之外有 `#[serde(flatten)] extra` 扩展位（多租户的 `tenant_id`、会话版本等都放这里）。**不要自己另签 token**——`jti`/黑名单/过期语义会漂移。

```rust
let jwt = JWT::from_config();
let token = jwt.issue(
    ClaimsBuilder::new("u-1")
        .username("alice")
        .roles(&roles)
        .permissions(&perms)
        .claim("tenant_id", serde_json::json!(42)),   // 自定义声明
)?;
// 刷新必须走 issue_refresh 并带上 tenant_id，否则新 Access Token 会丢租户
```

- `jwt.validate(&token)?` 取值的业务代码**无需改动**；仅手工构造 `TokenClaims {..}` 字面量需补 `extra: Default::default()`。
- 读取：`claims.extra_claim("tenant_id")` / `extra_str` / `extra_i64`。`extra` 是无类型 map，键名拼错只在运行期暴露，自定义 claim 键名勿与固定字段（`sub`/`roles`…）冲突。

## 鉴权：权限与角色

`AuthLayer` 校验 JWT 注入 `AuthClaims`；`RequireRoleLayer`（`#[alun::role]`）/ `RequirePermissionLayer`（`#[alun::permission]`）/ `PermissionCheckLayer`（`PERMISSION_ROUTES` 切片或显式注册）负责鉴权。持有 `*`/`*:*:*` 视为超级管理员（`TokenClaims::is_super_admin`）。

## 隔离：三条通道（按优先级选）

- **通道 A（推荐）：`ModelQuery::join::<U>()` 结构化联查** —— 从表 `#[tenant]` 由框架自动注入，不存在"漏标一张表"；
- **通道 B：原生 SQL + `/*tenant:别名*/` 占位 + `raw_tenant_annotated()`** —— 结构化表达不了时（UNION/CTE/窗口函数）用，每张租户表都要标；
- **通道 C：显式跨租户** —— `tenant_scope(TenantContext::Cross, async { .. })`，管理后台/迁移/巡检用。

全局表用 `#[model(cross_tenant)]` 显式豁免，不参与隔离（不要包 `Cross`）。

## 缓存与文件：上下文感知命名空间

`db()`/`cache()` 是全局单例，但资源访问必须上下文感知，避免多租户串数据：

```rust
use alun::prelude::cache_tenant;       // = cache().scoped(format!("tenant:{key}"))
let c = cache_tenant()?;               // 无租户上下文 → Err（fail-closed）
c.set("dict:type", &v).await?;
```

底层 `CacheNamespace` 自动加前缀；命名空间前缀与实例自身 `key_prefix` 处于**同一层级**（非嵌套），`c.scoped("t:7").get("k")` 与 `c.get("t:7:k")` 命中同一物理 key。

## 安全自检清单

- [ ] 多租户写入是否依赖框架自动补 `tenant_id`（而非手工传）？
- [ ] 联查是否用 `ModelQuery::join::<U>`（而非手写 `JOIN` + 字符串占位）？
- [ ] 原生 SQL：每张租户表都有 `/*tenant:别名*/` **且**调用了 `raw_tenant_annotated()`？
- [ ] 全局表是否用 `#[model(cross_tenant)]` 显式豁免（而非包 `Cross`）？
- [ ] 缓存/文件路径是否经 `cache_tenant()` / 租户命名空间？
- [ ] 自定义 claim 键名是否与固定字段（`sub`/`roles`…）冲突？

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [附录：配置全字段参考](config-reference.md) | [返回总目录](../getting-started.md) → |

<sub>本分册属于「从零到跑起来（8 步分册）」，共 8 篇。首篇 / 末篇可点「返回总目录」。</sub>

---
id: guides.db.multi-tenant
title: 多租户：与普通操作逐项对照
summary: 绝大多数操作一行都不用改（附实际生成的 SQL 对比）；原生 SQL/定时任务/超管三种必须动手的情况各附错误与正确写法对比；fail-closed 真实报错文案、两个安全指标、行级 vs schema-per-tenant 选型对比、§6.7 设计要点。
tags: [database, multi-tenant, isolation, fail-closed, saas]
status: active
owner: alun-core-team
source_of_truth: [code, alun-db/src, alun-db-core/src]
depends_on: [guides.database]
last_reviewed: 2026-10-06
---

# 多租户：与普通操作逐项对照

> 本文件是 [`database.md`](../database.md) 的 `multi-tenant` 分册；总目录与阅读顺序见 [数据库指南](../database.md)。


> 这一章只讲一件事：**开了多租户之后，你的代码跟普通（无租户）写法到底差在哪。**
> 设计动机、注入点清单、性能分析见本章末的「§6.7 设计要点」，以及 [migrations/v0.3.0-breaking.md](../../migrations/v0.3.0-breaking.md)。

## 6.1 先看清：绝大多数操作一行都不用改

多租户做的是**行级隔离**：框架在 Model 层的读写路径上自动补 `tenant_id = ?`。
所以只要你走 Model 模式，业务代码跟单租户**完全一样**：

```rust,no_run
use alun::prelude::*;

#[derive(Model, Debug, Clone, serde::Serialize)]
pub struct Invoice {
    #[key]
    pub id: i64,
    pub title: String,
    /// 租户列：普通业务字段，类型无关（推荐 i64）
    #[tenant]
    pub tenant_id: i64,
}

// 下面这段代码在「有租户」和「无租户」两种配置下**一模一样**：
async fn list_mine() -> ResResult<Vec<Invoice>> {
    let list = Invoice::query().limit(20).find(db()).await?;
    Ok(Res::ok(list))
}

async fn load(id: i64) -> ResResult<Invoice> {
    let inv = Invoice::find(db(), id).await?;
    Ok(Res::ok(inv))
}

async fn create(title: String) -> ResResult<Invoice> {
    let mut inv = Invoice { id: 0, title, tenant_id: 0 };
    inv.insert(db()).await?;          // 写路径自动补齐 tenant_id
    Ok(Res::ok(inv))
}
```

配置里打开开关即可：

```toml
# config/config.toml
[tenant]
enabled = true
claim = "tenant_id"   # JWT 里承载租户 ID 的字段名
write_enabled = true  # 写路径租户补齐 / 校验（迁移观察期可先设 false）
# 以下两项仅 PostgreSQL 生效，额外做 schema 级（库级）隔离
# schema_per_tenant = true
# schema_prefix = "tenant_"   # 最终 schema 名 = 前缀 + 租户ID，如 tenant_42
```

## 6.2 对照表：同一个操作，有租户 vs 无租户

| 操作 | 普通（无租户） | 多租户（`enabled = true`） | 代码要不要改 |
| ---- | ---- | ---- | ---- |
| `Invoice::query().find(db())` | `SELECT .. FROM invoice` | 自动加 `WHERE tenant_id = ?` | **不用改** |
| `Invoice::find(db(), id)` | 按主键查 | 按主键 **+ 租户**查；查到别租户的行视为不存在 | **不用改** |
| `Invoice::of(id).set(..).update(db())` | 按主键更新 | 按主键 **+ 租户**更新 | **不用改** |
| `inv.insert(db())` | 原样插入 | 自动用上下文租户**补齐** `tenant_id` | **不用改**（`tenant_id` 填 0 即可） |
| `Invoice::delete(db(), id)` | 软删 | 软删 + 租户条件 | **不用改** |
| 分页 `paginate` | 全表分页 | 租户内分页，`total` 是租户内的 | **不用改** |
| **`Row` + 原生 SQL**（`db().find(..)`） | 原样执行 | **不注入也不改写**，且带 `#[tenant]` 的模型**直接报错**（fail-closed） | 必须显式用 `Cross` 或自己写 `tenant_id = ?` |
| `Invoice::sql(sql!(..))` | 原样执行 | 同上，**不能**自动补租户 | 需显式 `Cross`，否则报错 |
| 业务上要跨租户（超管 / 对账） | — | `tenant_scope(TenantContext::Cross, async { .. }).await` | 加一层作用域 |
| 后台任务 / 定时任务（无 HTTP 请求） | — | `tenant_scope(TenantContext::Scoped(Value::I64(tid)), async { .. }).await` | 加一层作用域 |
| 全局表（字典、地区）不隔离 | 普通模型 | 结构体加 `#[model(cross_tenant)]` | 加一个属性 |
| 忘了带租户上下文 | — | **直接报错**，绝不静默返回全量数据 | — |

### 看看 SQL 到底长什么样

同样一句 `Invoice::query().find(db())`，框架生成的 SQL 是这样的：

```sql
-- enabled = false（普通模式）
SELECT * FROM invoice LIMIT 20;

-- enabled = true（多租户）—— tenant_id = 42 由框架按上下文注入
SELECT * FROM invoice WHERE tenant_id = 42 LIMIT 20;
```

**业务代码一个字都没多写。** 这就是 `#[tenant]` 属性的全部作用——把「隔离」变成声明式的事。

## 6.3 只有这三种情况需要你动手

1. **原生 SQL / `Row` 模式**：拿不到自动注入。走 `Cross` 显式提权，或自己在 SQL 里写 `tenant_id = ?`。
2. **没有 HTTP 请求的场景**（定时任务、消息消费、CLI）：没有 JWT 就没有租户上下文，
   用 `tenant_scope(TenantContext::Scoped(..))` 包一层。
3. **确实要跨租户**（超管后台、全局统计）：用 `tenant_scope(TenantContext::Cross, ..)` 显式提权，
   让「我要跨租户」这件事在代码里**看得见**。

### 情况 1 · 原生 SQL：框架不碰你的 SQL

框架**不会**去解析、改写你的原生 SQL——那太危险了。所以它选择另一条路：**要么你承诺，要么报错**。

```rust,ignore
use alun::prelude::*;

// ❌ 错误：带 #[tenant] 的模型走原生 SQL，却没有任何声明 → 直接报错（fail-closed）
let rows = db().find("SELECT * FROM invoice WHERE status = 1").await?;

// ✅ 正确 A：SQL 里自己带租户条件 + 占位标记 + 显式承诺完整性
let sql = sql!(
    "SELECT id, title, amount FROM invoice",
    " WHERE status = ", { status },
    " AND /*tenant:invoice*/"
);
let rows = Invoice::sql(sql)
    .raw_tenant_annotated()      // 声明「我保证每张租户表都标了占位」
    .find(db())
    .await?;

// ✅ 正确 B：这本来就是跨租户查询（运维对账、数据修复）→ 显式提权
let all = tenant_scope(TenantContext::Cross, async {
    db().find("SELECT * FROM invoice").await
}).await?;
```

**为什么不自动给原生 SQL 加 `tenant_id = ?`**：框架无法确定你的 WHERE 子句结构、
别名怎么对应、要不要加括号——猜错就是**静默的数据泄漏**。所以规则是：
**要么用 Model 模式（自动安全），要么用 `/*tenant:别名*/` 占位 + `raw_tenant_annotated()` 显式承诺，要么显式 `Cross`。**
没有第四条路。

### 情况 2 · 定时任务：没有请求就没有租户

定时任务、MQ 消费、CLI、集成测试——这些场景**没有 HTTP 请求**，也就没有 JWT，
拿不到租户上下文。此时若直接查带 `#[tenant]` 的模型，会触发 fail-closed 报错：

```rust,no_run
use alun::prelude::*;

// ✅ 正确：用 tenant_scope 包一层，指定「我现在代表哪个租户」
async fn nightly_report(tid: i64) -> ResResult<Res<Vec<Invoice>>> {
    let rows = tenant_scope(TenantContext::Scoped(Value::I64(tid)), async {
        Invoice::query().find(db()).await
    })
    .await?;
    Ok(Res::ok(rows))
}
```

注意 `Value::I64(tid)`——租户值是一个 `Value`（框架的动态类型），
所以租户 ID 可以是数字、也可能是字符串 UUID。

### 情况 3 · 超管后台：显式提权，让意图可见

```rust,no_run
use alun::prelude::*;

// 超管看全平台数据：tenant_scope(Cross) 让「我要跨租户」在代码里一目了然
async fn admin_count() -> ResResult<Res<i64>> {
    let n = tenant_scope(TenantContext::Cross, async {
        Invoice::count(db()).await
    })
    .await?;
    Ok(Res::ok(n))
}

// 批注某个租户的账（运营后台常见）：把外部来的租户 ID 显式收窄
async fn settle_tenant(tenant_id: i64) -> ResResult<Res<u64>> {
    let affected = tenant_scope(TenantContext::Scoped(Value::I64(tenant_id)), async {
        Invoice::of(1i64).set(Invoice::title(), "已结算").update(db()).await
    })
    .await?;
    Ok(Res::ok(affected))
}
```

**为什么用 `enum TenantContext { Scoped, Cross }` 而不是 `Option<Value>`**：
`None` 有歧义——它可能表示「没设上下文」（该报错），也可能被误读成「无租户 = 不过滤」（该报错才对，但语义拧巴）。
拆成两个变体后，**「限定到某租户」和「显式越权」在类型层面就是分开的**，
编译器能帮你堵住「不小心跨租户」的洞。

## 6.4 三个安全默认值（都是「宁可报错，不可泄露」）

| 机制 | 行为 |
| ---- | ---- |
| **fail-closed** | 模型声明了 `#[tenant]` 却没有租户上下文（忘了登录、定时任务忘了包 `tenant_scope`）→ **查询直接报错**，绝不静默返回全量数据 |
| **关闭时零回归** | `enabled = false`（默认）时既不注入条件也不做守卫，行为与单租户完全一致 |
| **迁移观察期** | `write_enabled = false`：读照常隔离，写退化为「不注入、不校验」，方便灰度 |

### fail-closed 长什么样

「忘了设租户上下文」在传统做法里往往**不会报错**——SQL 少一个条件，于是返回了全表数据，
问题要到线上数据串了才会被发现。Alun 把它变成**当场报错**：

```rust,ignore
// 场景：一个忘记包 tenant_scope 的定时任务
async fn broken_job() -> ResResult<Res<Vec<Invoice>>> {
    // ❌ 这里会返回一个 DbError，任务失败并告警
    //    而不是「安静地把所有租户的数据导出来」
    let rows = Invoice::query().find(db()).await?;
    Ok(Res::ok(rows))
}
```

报错信息会明确指出**哪个模型**缺了租户上下文（来自 `alun-db/src/tenant.rs`）：

```text
model `invoice` requires a tenant scope but no tenant context is set;
wrap the call in `tenant_scope(..)` or declare `#[model(cross_tenant)]`
```

**这个设计是有意的**：宁可让任务失败一次，也不要静默地把 A 租户和 B 租户的数据混在一起。
排查时按提示补上 `tenant_scope` 即可。

### 配套的安全指标

两个 Prometheus 指标可以第一时间发现隔离被触发：

| 指标 | 含义 | 非 0 怎么办 |
| ---- | ---- | ---- |
| `alun_tenant_scope_missing_total` | 有代码需要租户上下文却没有 | 当告警处理——说明有路径漏设了上下文 |
| `alun_raw_sql_tenant_unbound_total` | 原生 SQL 未绑定租户 | 查是否有裸 SQL 绕过了隔离 |

> 开启 `[metrics].enabled` 后自动产出，无需埋点。

## 6.5 老系统怎么迁进来

按顺序做，每一步都可以独立上线、随时回退：

1. **只加 `#[tenant]` 属性**（此时 `enabled = false`，代码行为完全不变）→ 上线；
2. 打开 `enabled = true`、`write_enabled = false`，**只观察读隔离** → 观察；
3. 确认无误后打开 `write_enabled = true`；
4. 最后清理服务层手写的 `AND tenant_id = ?`。

## 6.6 PostgreSQL schema-per-tenant（可选，库级隔离）

除行级 `tenant_id = ?` 外，可再叠加库级隔离：每个租户一个 schema，连接从池中借出后、
业务语句（或 `BEGIN`）之前执行 `SET search_path TO "前缀+租户ID"`，未限定表名即落在
租户自己的 schema 里。schema 名由租户 ID 派生并做**字符集白名单**校验（`[A-Za-z0-9_-]`、
长度 ≤ 63），拒绝注入字符；连接上缓存已生效的 schema，同租户连续请求零额外往返。
SQLite / MySQL 无此概念，开关惰性。

### 两种隔离方式的差别

假设有租户 42 和租户 77，SQLite 上**根本不支持** schema（惰性忽略），
而 PostgreSQL 上两种都可以开：

| | 行级隔离（默认） | schema-per-tenant（叠加） |
|---|---|---|
| 物理形态 | 一张 `invoice` 表，所有租户的行混在一起 | 租户 42 有 `tenant_42.invoice`、租户 77 有 `tenant_77.invoice` |
| 一条写错 WHERE 的后果 | **可能看到别租户的行** | **看不到别租户的表**（表本身不存在） |
| 迁移 | 一次全表 `ALTER` | 每个 schema 各改一次 |
| 备份 / 恢复 | 按行过滤导出 | 按 schema 整体导出，天然可单租户恢复 |
| 跨租户统计 | 一条 `GROUP BY tenant_id` 即可 | 需跨 schema union，复杂得多 |
| 适用 | **绝大多数 SaaS 够用** | 有「单租户整体交付 / 数据驻留合规」要求时 |

**推荐路径**：先用默认的行级隔离（简单、迁移轻、跨租户统计方便）。
等到真的遇到「必须把某个租户的数据整体搬走/交付/销毁」时，再叠加 schema 隔离。
两者可以共存——`#[tenant]` 负责行级过滤，`search_path` 负责库级隔离，各管一层。

```toml
# config/config.toml —— 在行级隔离之上叠加库级隔离
[tenant]
enabled = true
schema_per_tenant = true      # 仅 PostgreSQL 生效
schema_prefix = "tenant_"     # 租户 42 → schema "tenant_42"
```

---

## 6.7 设计要点：隔离是怎么落地的（精华）

> 下面这几条是 0.3 多租户实现的"为什么"，对应原设计文档的四条核心范式。日常写法见 [multi-tenant.md](multi-tenant.md) §6.1–§6.6，本节能帮你避开"看起来能用、实则漏隔离"的写法。

**精华 1 · 复制 `SOFT_DELETE` 的"声明 → 注入 → 豁免"范式**
`#[tenant]` → `Model::TENANT` → `scope_filter::<T>()` 在查询/写入口注入 → `#[cross_tenant]` / `TenantContext::Cross` 显式豁免。与软删除同一条链路，学习成本为零。

**精华 2 · 租户条件走 `Statement` 的结构化字段，而不是 `Expr`**
`SelectStmt`/`UpdateStmt`/`DeleteStmt` 各加一个内联（无 `Box`）的 `scope: ScopeMode` 字段，`Join` 加 `scope: Option<ScopeFilter>`。因为租户值每个请求都不同，用 `Expr::Binary` 表达会引入每查询约 40ns 的堆分配；内联字段让隔离注入的渲染分配降到最低。**`ScopeMode::Unset` + 执行前 fail-closed 守卫**是本方案的安全底线：把"忘了加租户条件"从静默数据泄露变成一次明确报错。

**精华 3 · 上下文用 `tokio::task_local!`，不是 `thread_local!`**
async 任务在 `await` 点会跨线程迁移，`thread_local!` 会**串租户**（A 请求的租户值被 B 请求读到）。`task_local` 下业务函数签名完全不用改，是"简单好用"的关键。

**精华 4 · 零签名改动 + 显式提权**
用 `enum TenantContext { Scoped(Value), Cross }` 而非 `Option<Value>`：语义上 `None`（无上下文）与 `Cross`（显式越权）必须可区分，否则 `None` 会被误读成"无租户 = 不过滤"。

**三条通道（按优先级选）**
- **通道 A（推荐）：`ModelQuery::join::<U>()` 结构化联查** —— 从表模型由类型参数 `U` 枚举，框架自动注入从表租户条件，**不存在"漏标一张表"**，从机制上消灭 F2 的泄漏类缺陷；租户谓词追加在 `ON` 之后保证 `LEFT JOIN` 语义正确。
- **通道 B：原生 SQL + 占位 + 显式承诺** —— `/*tenant:别名*/` 占位（独立谓词，需自行 `AND` 连接；每张租户表都要标），且必须 `raw_tenant_annotated()` 承诺完整性；未调用一律拒绝。
- **通道 C：显式跨租户** —— `tenant_scope(TenantContext::Cross, async { .. })`，与"忘记设上下文"区分开。

**性能预算（实测，SQLite 10 万行表）**
隔离注入绝对开销约 **0.5 µs/次**（不含 SQL 执行）；分操作宽松回归上限：点查 ≤ 4×、分页 ≤ 2×、批量插入 ≤ 1.5×。分页务必用 keyset 游标（`page_after`），`OFFSET` 一旦带 `WHERE` 会退化为 O(n²/页大小)，与租户无关。

**迁移灰度（关键）**
`[tenant].enabled` 是总开关（关闭时 `#[tenant]` 完全惰性），`write_enabled` 单独控制写路径。老系统迁移建议三步：① 只加 `#[tenant]` 不开关；② 开 `enabled` + `write_enabled=false` 只隔离读、观察；③ 确认后开 `write_enabled` 放开写路径并清理服务层手写的 `AND tenant_id = ?`。回退只需改配置。

**schema 双路径优先级**
`#[model(schema = "..")]`（静态）与 `[tenant].schema_per_tenant`（动态）并存时：**动态 `search_path`（连接级）优先**，`#[model(schema)]` 作显式跨 schema 限定（按 `cross_tenant` 语义处理）；同时生效时启动日志打 `warn!`，不静默。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [⑤ 视图 / Hook / 迁移 / 多库 / 错误映射](ops.md) | [附录：API 速查 / 反模式 / 已删除 API](appendix.md) → |

<sub>本分册属于「数据库实操（7 分册）」，共 7 篇。首篇 / 末篇可点「返回总目录」。</sub>

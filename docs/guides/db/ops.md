---
id: guides.db.ops
title: 按需求查技巧：视图、Hook、迁移、多库与错误映射
summary: 只读视图、审计与时间戳 Hook、表结构演进（迁移）、一套代码跑多种数据库、数据库错误映射为 HTTP 响应、新手踩坑清单。
tags: [database, migration, hook, view, error-mapping, pitfalls]
status: active
owner: alun-core-team
source_of_truth: [code, alun-db/src, alun-db-core/src]
depends_on: [guides.database]
last_reviewed: 2026-10-06
---

# 按需求查技巧：视图、Hook、迁移、多库与错误映射

> 本文件是 [`database.md`](../database.md) 的 `ops` 分册；总目录与阅读顺序见 [数据库指南](../database.md)。
> 查询与数据操作技巧（条件 / 联表 / 多表 / 批量 / 动态表名）见 [`recipes.md`](recipes.md)。

这一章共 5 个需求 + 踩坑清单，每个都是「什么情况下需要 + 最小示例 + 一句话建议」。
**新手可以先跳过，遇到问题时按标题回来查。**

## 5.6 只读视图

映射到数据库视图而不是物理表时，用 `#[view("..")]`——**隐式只读**，不需要额外声明：

```rust,ignore
use alun::prelude::*;

/// 视图名 = v_active_user（PostgreSQL / SQLite 通用写法）
#[derive(Model, Debug, Clone, serde::Serialize)]
#[view("v_active_user")]
pub struct ActiveUser {
    #[key]
    pub id: i64,
    pub name: String,
    /// 视图读取同样走租户隔离
    #[tenant]
    pub tenant_id: i64,
}
```

- 读取用法与普通 Model 完全一样：`ActiveUser::query().find(db()).await?`
- 视图上**不能** `insert` / `update` / `delete`（对视图做写操作多数数据库直接拒绝）
- 复杂聚合不适合用 Model 表达时（窗口函数、多列 `GROUP BY`），走
  [§5.2 联表聚合](recipes.md) 或原生 SQL + [`multi-tenant.md`](multi-tenant.md) 的显式承诺

## 5.7 审计、时间戳、软删除

三类横切逻辑挂在 **Hook** 上：同步 `Hook` 拿到 `&mut Row` 可改字段，异步 `AsyncHook` 在**事务提交后**触发。

```rust,ignore
use alun::prelude::*;
use alun_db_core::traits::hook::{Hook, AsyncHook};

/// 同步 Hook：写入前补字段。六个钩子按需实现，不实现就是默认空实现。
struct TimestampHook;

impl Hook for TimestampHook {
    // 插入前补 created_at / updated_at
    fn before_insert(&self, row: &mut Row) -> DbResult<()> {
        let now = Utc::now();
        row.set("created_at", now)?;
        row.set("updated_at", now)?;
        Ok(())
    }
    // 更新前刷新 updated_at
    fn before_update(&self, row: &mut Row) -> DbResult<()> {
        row.set("updated_at", Utc::now())?;
        Ok(())
    }
}

/// 异步 Hook：**只在事务提交后**触发，适合写审计、发通知、失效缓存。
struct AuditHook;

#[async_trait::async_trait]
impl AsyncHook for AuditHook {
    async fn after_insert(&self, row: &Row) -> DbResult<()> {
        // 此时事务已提交，写审计失败不会让业务回滚
        audit_log::append(row).await
    }
}
```

> ⚠️ **同步 `Hook` 的 `after_*` 在事务内执行**——所以**不要在 `Hook` 里写库**，
> 否则会出现「审计已写、业务回滚」的不一致。需要 IO 的副作用一律用 `AsyncHook`。

软删除则是**声明式**的：模型存在 `Option` 类型的 `deleted_at` 字段即自动生效，
删除自动改写成 `UPDATE ... SET deleted_at = now()`，查询自动追加 `deleted_at IS NULL`。
要真正物理删除就给模型加 `#[model(hard_delete)]`。

## 5.8 表结构演进（迁移）

迁移由 `Migrator` 统一驱动，**方言感知 + 记录表防重跑**：

```rust,no_run
use alun::prelude::*;

// 从目录跑全部未应用的迁移，返回本次应用的条数
let applied = Migrator::from_dir(std::path::Path::new("migrations")).run(db()).await?;
println!("applied {applied} migrations");
```

约定：

| 项 | 规则 |
| --- | --- |
| 文件协议 | 一个文件一条 SQL（`statement_split = true` 时支持多语句，默认开） |
| 命名 | 自动识别 `V` 前缀或无前缀；可重复迁移用 `R` 前缀、种子用 `S`、基线用 `B` |
| 记录表 | `(kind, name)` 唯一键 + `applied_seq` + `applied_via`，重复跑不会重复执行 |
| 校验 | `validate_checksum = true` 会校验已应用迁移的内容指纹，改动已发布文件会按 `divergent_policy` 报警或中止 |
| 接入存量库 | `import_legacy = true` 读旧框架的 `_sqlx_migrations` 还原已应用记录；或用 baseline 标记 |

也可以在启动钩子里自动跑（开发环境省事）：

```rust,ignore
App::new()?
    .on_startup(|| async {
        Migrator::from_dir(std::path::Path::new("migrations")).run(db()).await?;
        Ok(())
    })
    .scan()
    .start()
    .await
```

> 生产环境建议把迁移单独作为一次性的运维步骤执行，而不是每个实例启动都跑。
> 完整配置项见 [`startup/config-reference.md`](../startup/config-reference.md)，API 签名见
> [`reference/crates/db.md`](../../reference/crates/db.md)。

## 5.9 一套代码跑多种数据库

三种方言的差异都由**方言渲染器**吸收，SQL 写法只有三类需要留意：

```rust,ignore
// ① 占位符统一写 ?，PostgreSQL 执行前自动改写为 $1 / $2
let rows = db().find_with_params(
    "SELECT * FROM sys_user WHERE status = ? AND name LIKE ?",
    vec![1i64.into(), format!("%{kw}%").into()],
).await?;

// ② 不支持的语法走方言能力协商，报错文案会告诉你该方言缺什么
//    （MySQL 没有 ON CONFLICT，upsert 要退回「先查后写」或 ON DUPLICATE KEY UPDATE）

// ③ 方言特有能力（RETURNING、窗口函数、JSON 运算符）先查方言支持矩阵再拼
```

**能写可移植 SQL 就写可移植的**；确实要用方言特性时，先查
[`reference/crates/db-family.md`](../../reference/crates/db-family.md) 里的方言支持矩阵，
不要假设某种语法在三种库上都成立。

## 5.10 把数据库错误变成 HTTP 响应

这一节是**「错误直通」机制的完整说明**——绝大多数情况下你**不需要写任何 `map_err`**。

`DbError` 住在 `alun-db-core`，`ApiError` 住在 `alun-web`。按 Rust 的孤儿规则，
`alun-web` 可以为「外部类型 → 本地类型」实现 `From`，于是：

```rust,no_run
use alun::prelude::*;

// handler 恒为 ResResult<T>，数据层的 DbError 直接 ? 上来
#[alun::get("/invoices/{id}")]
async fn detail(Path(id): Path<i64>) -> ResResult<Res<Invoice>> {
    let inv = Invoice::find(db(), id).await?;   // ← 不用写一行 map_err
    Ok(Res::ok(inv))
}
```

自动映射规则（`alun-web/src/api.rs` 的 `db_error_to_api_error`）：

| `DbError` 变体 | 自动变成 | 语义 |
| --- | --- | --- |
| `NotFound` | 404 | 记录不存在 |
| `UniqueViolation` | 409 | 数据重复：唯一约束冲突 |
| `ForeignKeyViolation` / `CheckViolation` | 409 | 数据约束冲突 |
| `Deadlock` / `SerializationFailure` / `LockTimeout` | 500 | 服务繁忙，请重试 |
| 其余（连接失败、查询失败、类型不匹配…） | 500 | 模糊文案，细节只进日志 |

**无论走哪条分支，`DbError` 原文（含驱动原因与 SQL）都写进 `internal_detail`，
只进日志、绝不出现在响应体**——对外脱敏与对内可排查同时成立。

### 什么时候才需要手写 `map_err`

只想换一句更贴合业务的文案时才写，且**必须保留兜底分支**：

```rust,ignore
// 唯一冲突想说得更具体
user.insert(db()).await.map_err(|e| match e {
    DbError::UniqueViolation { .. } => ApiError::conflict("该邮箱已注册，请直接登录"),
    // other 分支绝不能省：省了就会把内部细节直接暴露给用户
    other => ApiError::internal_masked("创建失败", other.to_string()),
})?;
```

**默认用 `?`，确实需要定制文案时才 `map_err`。** 更多说明见 [`crud.md`](crud.md) §4.8。

## 新手最常踩的坑

| 现象 | 原因 | 正确做法 |
| ---- | ---- | -------- |
| 启动直接失败，日志是 `数据库连接失败: ...` | 配置字段填错 / 库没起 / 认证失败 | 对照 [`getting-started.md`](getting-started.md) 的配置表逐项核对；先看日志里的脱敏 URL |
| 改了 `config/config.toml` 却「没生效」 | **TOML 解析失败会静默回退到全默认配置** | 用 `cargo run -- print-config` 打印生效配置 |
| handler 里 `db()` panic | 数据库没初始化（`[database] enabled = false`，或走了 `with_config(default)`） | 关键路径用 `db()`，可选路径用 `try_db()` 显式降级 |
| 事务里回滚没生效 | 事务内调了 `db()` 而不是 `tx` | 事务内统一传 `&tx`（Model / `ChangeSet` / `sql!` 首参都接受） |
| 软删除后列表里还看得到已删数据 | 走了动态 `Row` 或原生 SQL，绕过了软删过滤 | 回收站以外的查询走 Model 层；原生 SQL 自己带 `deleted_at IS NULL` |
| 给非 `Option` 的 `deleted_at` 加了 `#[soft_delete]` | 非 `Option` 的 `deleted_at` 本身就会编译报错 | 用 `Option<...>`，或 `#[model(hard_delete)]` 关掉软删 |
| `insert_many` 后拿到的主键是 `0`、时间戳是空 | 批量插入**不跑 Hook、不回填主键** | 构造行时自己填时间戳；需要回填就逐条 `insert` |
| 审计写进去了，业务却回滚了 | 同步 `Hook` 的 `after_*` 在**事务内**执行 | 有 IO 的副作用改用 `AsyncHook`（提交后触发） |
| 分页返回的字段名对不上 | `Page` 是 `size`，响应是 `page_size` | 用 `into_res()` / `into_page_data()`，别手工搬字段 |
| 深分页越来越慢 | `OFFSET` 越翻越贵 | 换 `page_after` + `Cursor`；keyset 模式 `total` 恒为 0，用 `has_next()` |
| 用 `unwrap()` 处理 `db()` / `get_typed` | 未初始化 panic、类型不符 panic | 用 `try_db()`、`?`、`unwrap_or_default()` |
| 迁移跑不起来 | 文件里写了多条语句 / 改了已发布的迁移 | 每个文件一条语句；改动追加新版本 |
| `upsert` 在 MySQL 上报错 | MySQL 无 `ON CONFLICT` | 退回「先查后写」，或写 `ON DUPLICATE KEY UPDATE` 原生 SQL |
| 错误统一返回 500，明明是「不存在」 | 手写了 `map_err(...internal_masked...)` 抹平了自动映射 | 去掉 `map_err` 用 `?`；只在要换文案时才手写 |

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [④ 查询与数据操作技巧](recipes.md) | [⑥ 多租户隔离](multi-tenant.md) → |

<sub>本分册属于「数据库实操（7 分册）」，共 7 篇。首篇 / 末篇可点「返回总目录」。</sub>

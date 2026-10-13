---
id: guides.db.modeling
title: 定义模型（Model derive 与属性速查）
summary: 最小模型、属性速查（除 #[view] 外全部可选）、新手常见提问。
tags: [database, model, derive, schema]
status: active
owner: alun-core-team
source_of_truth: [code, alun-db/src, alun-db-core/src]
depends_on: [guides.database]
last_reviewed: 2026-10-06
---

# 定义模型（Model derive 与属性速查）

> 本文件是 [`database.md`](../database.md) 的 `modeling` 分册；总目录与阅读顺序见 [数据库指南](../database.md)。


## 3.1 最小模型

多数业务表就用 `#[derive(Model)]`，并把默认约定当成免费赠品：

```rust,no_run
use alun::prelude::*;

/// `sys_user` 表对应的强类型模型。
#[derive(Model, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SysUser {
    pub id: i64,                     // 名为 id 的整型主键：默认就是 #[key] #[auto]
    #[unique]
    pub email: String,
    #[column("user_name")]           // Rust 字段名与列名不一致时显式映射
    pub name: String,
    pub status: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,   // 自动填充
    pub updated_at: chrono::DateTime<chrono::Utc>,   // 自动填充
}
```

上面这一小段，derive 已经帮你做了这些事：

| 你写的 | 自动获得的行为 |
| ---- | ---- |
| 结构体名 `SysUser` | 表名 `sys_user`（驼峰自动转蛇形） |
| 字段 `id: i64` | 主键 + 自增（插入时值填 `0` 即可，数据库生成后回填） |
| 字段 `email` | 唯一列约束声明 |
| `#[column("user_name")]` | 字段 `name` ↔ 列 `user_name` |
| `created_at` / `updated_at` | 插入/更新时**自动覆盖**为当前时间（`TimestampHook`） |
| 每个字段 | 一个编译期校验的访问器，如 `SysUser::name() -> FieldExpr<String>` |

## 3.2 属性速查（除 `#[view]` 外全部可选）

| 属性 | 位置 | 作用 |
| ---- | ---- | ---- |
| `#[table("name")]` | 结构体 | 显式指定表名；缺省由结构体名蛇形推导（`SysUser` → `sys_user`、`HTTPLog` → `http_log`） |
| `#[view("name")]` | 结构体 | 只读视图（见 [5.6](ops.md) §5.6），**必须显式写出** |
| `#[hook(Type)]` | 结构体 | 注册同步 Hook，可写多条（见 [5.7](ops.md) §5.7） |
| `#[hook_async(Type)]` | 结构体 | 注册异步 Hook（提交后触发），可写多条（见 [异步 Hook](#异步-hook-需要-io-的副作用)） |
| `#[model(readonly)]` | 结构体 | 只读模型：写路径一律拒绝（`#[view]` 隐式只读） |
| `#[model(strict)]` | 结构体 | 编译期校验列名与迁移快照一致（见 [5.8](ops.md) §5.8） |
| `#[model(hard_delete)]` | 结构体 | 关闭软删除语义，`delete` 变成物理删除 |
| `#[model(no_timestamps)]` | 结构体 | 关闭 `created_at` / `updated_at` 自动覆盖（例如 `created_at` 承载业务时间） |
| `#[model(cross_tenant)]` | 结构体 | 该模型不参与多租户隔离（全局表，如字典）；见 [第 6 章](multi-tenant.md) |
| `#[key]` | 字段 | 主键（至多一个）；缺省时叫 `id` 的字段即主键 |
| `#[auto]` | 字段 | 自增；缺省时「名为 `id` 的整型主键」默认自增 |
| `#[unique]` | 字段 | 唯一列 |
| `#[column("name")]` | 字段 | 列名映射 |
| `#[json]` / `#[json(nullable)]` | 字段 | JSON 列；嵌套结构再 `#[derive(Row)]` |
| `#[soft_delete]` | 字段 | 软删除标记列（至多一个）；`Option<...>` 的 `deleted_at` 缺省即生效 |
| `#[tenant]` | 字段 | 租户隔离列（至多一个，类型无关）；见 [第 6 章](multi-tenant.md) |

## 3.3 新手常见提问

**Q：表名/列名跟默认推导不一致怎么办？**
`#[table("sys_user_info")]` 改表名，`#[column("user_name")]` 改列名 —— 都是显式覆盖，写清楚就不容易踩坑。

**Q：主键不叫 `id`，或者不是自增的？**
非 `id` 命名的主键**不会**默认自增，请显式标注：`#[key] pub code: String`。
自增但列名不是 `id`：`#[key] #[auto] pub user_id: i64`。

**Q：`created_at` 是业务时间（开票时刻、事件发生时刻），不想被自动覆盖？**
在结构体上写 `#[model(no_timestamps)]`，时间戳完全由你自己维护。

**Q：想物理删除，不要软删除？**
`#[model(hard_delete)]`。
另外注意：**声明成非 `Option` 的 `deleted_at` 字段会编译报错**（提示你写成 `Option<...>` 或关闭软删）。

**Q：要存 JSON？**
字段加 `#[json]`；如果嵌套结构也想强类型，让嵌套结构也 `#[derive(Row)]`。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [① 配库与跑通](getting-started.md) | [③ 增删改查](crud.md) → |

<sub>本分册属于「数据库实操（7 分册）」，共 7 篇。首篇 / 末篇可点「返回总目录」。</sub>

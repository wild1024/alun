---
id: guides.db.crud
title: 六件最常用的事（照着抄）
summary: 查一条 / 查一批 / 新增 / 改 / 删 / 分页 / 计数，六个可直接照抄的示例，覆盖 90% 日常需求。
tags: [database, crud, query, pagination]
status: active
owner: alun-core-team
source_of_truth: [code, alun-db/src, alun-db-core/src]
depends_on: [guides.database]
last_reviewed: 2026-10-06
---

# 六件最常用的事（照着抄）

> 本文件是 [`database.md`](../database.md) 的 `crud` 分册；总目录与阅读顺序见 [数据库指南](../database.md)。


先记住一句话，它解释了别处学不到的「为什么有两种写法」：

> **查询是关联函数（`SysUser::find`），持久化是实例方法（`user.insert`）。**

这两组方法都由 `alun-db` 对 `T: Model` 的 blanket impl 提供（`CrudExt` / `PersistExt`），
derive 本身不生成它们。它们第一个参数是 `&impl SqlExecutor`：
传 `&Db` 走连接池，传 `&Transaction` 就进了事务（见 [5.3](recipes.md) §5.3）。

下面每一小节都是「实战提问 → 推荐写法 → 简化技巧」。

## 4.1 查一条

> **提问**：按 ID 查用户，查不到要返回 404，怎么写最省事？

```rust,no_run
use alun::prelude::*;
use std::result::Result;

#[derive(Model, Debug, Clone)]
pub struct SysUser {
    pub id: i64,
    pub email: String,
    pub status: i32,
}

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
// 「不存在就是错误」→ 用 find，查不到直接给 DbError::NotFound
let user: SysUser = SysUser::find(db(), 1i64).await?;

// 「不存在是正常情况」→ find_or_none，返回 Option
let maybe: Option<SysUser> = SysUser::find_or_none(db(), 1i64).await?;

// 按唯一列查（登录场景：email 唯一）
let by_email: Option<SysUser> = SysUser::find_first_by(db(), "email", ("a@b.com",)).await?;

// 按主键批量查（比循环单查少 N-1 次往返）
let some: Vec<SysUser> = SysUser::find_in_ids(db(), [1i64, 2, 3]).await?;
# let _ = (user, maybe, by_email, some);
# Ok(())
# }
```

**选哪个**：业务上「不存在就是异常」用 `find`（省一次 `Option` 判断）；存在性探测、幂等写入前检查用 `find_or_none`。

## 4.2 查一批 + 条件

> **提问**：查询启用的用户，按创建时间倒序取前 20 条。

```rust,no_run
use alun::prelude::*;
use std::result::Result;

#[derive(Model, Debug, Clone)]
pub struct SysUser {
    pub id: i64,
    pub name: String,
    pub status: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
// ① 条件固定 → 链式查询，类型安全（推荐）
let active: Vec<SysUser> = SysUser::query()
    .filter(SysUser::status().eq(1))
    .order_by(SysUser::created_at().desc())
    .limit(20)
    .find(db())
    .await?;

// ② 条件简单但懒得写表达式 → WHERE 片段（注意：片段是 SQL，参数要占位符）
let admins: Vec<SysUser> = SysUser::find_by(db(), "status = ? ORDER BY id DESC", (1,)).await?;

// ③ 单字段等值 → 双态签名会自动变成「字段 = ?」
let named: Vec<SysUser> = SysUser::find_by(db(), "name", ("张三",)).await?;

// ④ 单列 IN（列名编译期校验）
let by_status: Vec<SysUser> = SysUser::find_in(db(), SysUser::status(), [1, 2]).await?;

// ⑤ 全表（有软删字段时自动排除已删行）
let all: Vec<SysUser> = SysUser::query().find(db()).await?;
# let _ = (active, admins, named, by_status, all);
# Ok(())
# }
```

**简化技巧**：
- `filter` 之间**只有 AND**；需要 OR 时看 [5.1](recipes.md) §5.1 的 `sql!` 或 `Expr::or_all`。
- 只要前 N 条、不要总数 → `limit`，别用分页（少一次 `COUNT`）。
- 需要「恰好一行」的语义（唯一键校验）→ 用 `find_one`：0 行报 `NotFound`、多于一行报 `InvalidValue`，
  比「查一批再判断长度」更早暴露数据问题。

**两个极易踩的坑**：
1. `find_by` 的字符串片段是 **WHERE 之后的 SQL**（可带 `ORDER BY`，**不要带 `LIMIT`**）。
2. **不要**写 `find_by(db(), "age > ?", (18,))` 这种「两参数」形式：单参数只有是合法标识符时才走「字段 = ?」分支，
   其余按片段处理，容易占位符对不上。复杂条件请写完整 WHERE 片段，或改用 `query().filter(...)`。

## 4.3 新增

> **提问**：新增一个用户，插入后要拿到数据库生成的自增 ID。

```rust,no_run
use alun::prelude::*;
use std::result::Result;

#[derive(Model, Debug, Clone)]
pub struct SysUser {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub status: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
let mut user = SysUser {
    id: 0,                              // 自增占位值：0（或 Null），由数据库生成并**回填**
    email: "a@b.com".into(),
    name: "张三".into(),
    status: 1,
    created_at: Default::default(),      // 会被 TimestampHook 覆盖成当前时间
    updated_at: Default::default(),
};

user.insert(db()).await?;                // 纯 INSERT
# let _ = user.id;
# Ok(())
# }
```

**`insert` 与 `insert_or_update` 怎么选**：

| 方法 | 行为 | 什么时候用 |
| ---- | ---- | ---- |
| `insert` | 纯 INSERT；占位主键交给数据库生成并回填 | 明确是新增（省掉一次主键判断） |
| `insert_or_update` | 主键为占位（`Null` / `0`）→ INSERT；否则 UPDATE | 同一个入口既接受前端「新增」也接受「编辑」 |

## 4.4 改：整行写回 vs 只改几列

> **提问**：只要把用户状态改成 2，不想先 SELECT 再整行 UPDATE，怎么办？

```rust,no_run
use alun::prelude::*;
use std::result::Result;

#[derive(Model, Debug, Clone)]
pub struct SysUser {
    pub id: i64,
    pub name: String,
    pub status: i32,
}

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
// ① 只改几列 → 变更集：只需要主键，不 SELECT，只 UPDATE 被 set 的列
SysUser::of(1i64)
    .set(SysUser::status(), 2)
    .set(SysUser::name(), "李四")
    .update(db())
    .await?;

// ② 整表单提交（字段全都要写回）→ 先查出来改完再 update
let mut user: SysUser = SysUser::find(db(), 1i64).await?;
user.name = "王五".into();
user.update(db()).await?;              // 写回全部非主键字段；受影响 0 行 → NotFound
# let _ = user.status;
# Ok(())
# }
```

**为什么推荐变更集**：后台最典型的操作就是「改状态 / 改备注 / 改归属」这种单列改动。
`of(pk).set(...).update()` 省掉一次 SELECT，也避免把并发期间别人改过的列覆盖回去（写回整行才有这个问题）。

**编程期安全**：`set` 的键是 `FieldExpr`，字段名与值类型都编译期校验 —— `set(SysUser::status(), "2")` 直接编译不过。

**规则**：同一列多次 `set` 保留最后一次；`set` 主键、一列都不 `set`、字段不属于该模型 → `DbError::InvalidValue`。

## 4.5 删：软删除几乎是默认行为

> **提问**：删除用户后，希望数据还能找回（审计/回收站），怎么写？

```rust,no_run
use alun::prelude::*;
use std::result::Result;

#[derive(Model, Debug, Clone)]
pub struct Article {
    pub id: i64,
    pub title: String,
    pub status: i32,
    // Option<...> 的 deleted_at 缺省即「软删除标记列」
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
let article: Article = Article::find(db(), 1i64).await?;

article.delete(db()).await?;                       // → UPDATE articles SET deleted_at = now WHERE id = ?
Article::query().find(db()).await?;                // → SELECT ... WHERE deleted_at IS NULL（自动过滤）
Article::delete_by(db(), "status", (0,)).await?;   // 条件删除同样被改写为标记删除

// 免 SELECT 的按主键删除
Article::delete_by_pk(db(), 1i64).await?;
# let _ = article.title;
# Ok(())
# }
```

**简化技巧**：加了 `deleted_at: Option<...>` 之后，「查列表」和「删除」都不用你写任何额外代码，
整个查询族自动追加 `deleted_at IS NULL`，删除族自动改写为 `UPDATE`。

**两个注意点**：
- 要查回收站（**绕过**软删过滤）→ 用原生 SQL 显式写条件。
- 软删后唯一列仍占位：`email` 这类唯一列删过之后不能再次使用，通常要做「部分唯一索引」或把标记列并入唯一键。

真需要物理删除：结构体上写 `#[model(hard_delete)]`。

## 4.6 列表分页

> **提问**：后台用户列表要显示「共 N 条 / 第 x 页」，用最少的代码写完。

全框架分页参数**只有一个类型** `PageQuery`，响应结构**也只有一个**。
两步搞定：`PageQuery` 收参数 → `paginate` 查 → `into_res()` 转响应。

```rust,no_run
use alun::prelude::*;
use std::result::Result;

#[derive(Model, Debug, Clone, serde::Serialize)]
#[table("sys_user")]
pub struct SysUser {
    pub id: i64,
    pub name: String,
    pub status: i32,
}

#[alun::get("/api/users")]
async fn list_users(Query(pq): Query<PageQuery>) -> std::result::Result<Res<PageData<Vec<SysUser>>>, ApiError> {
    let page = SysUser::query()
        .filter(SysUser::status().eq(1))
        .paginate(db(), pq.page, pq.page_size)
        .await?;

    Ok(page.into_res())   // → { code:0, msg:"ok", data:{ list, total, page, page_size } }
}
```

`PageQuery` 反序列化时**已经做了参数规整**，所以缺参数、`page=0`、`page_size=0`、
`page_size` 超过 `MAX_PAGE_SIZE`（1000）都不会 400、也不会下溢：`page` 缺省 → 1，
`page_size` 缺省 → 10（上限 1000）。

分页容器字段名是新手最容易踩的坑，记住这张表：

| 容器 | 从哪来 | 字段 |
| ---- | ---- | ---- |
| `alun_db::Page<T>` | `paginate` / `PageBuilder::exec` | `list` / `total` / `page` / **`size`** / `pages` |
| `alun_core::PageData<T>` | 响应体 `data` | `list` / `total` / `page` / **`page_size`** |

`into_res()` / `into_page_data()`（`PageResponseExt`）是两者唯一的桥，**不要手工搬字段**。

只写原生 SQL 也想分页？`db().paginate("SELECT ...", pq.page, pq.page_size)` 返回 `Page<Row>`，
再 `.map(|r| Dto {...})` 映射后 `into_res()`（`Row` 不实现 `Serialize`，必须映射）。

**进阶**：移动端无限下拉、日志流这类「深分页」场景，用游标分页（`page_after` + `Cursor`），
写法见 [appendix.md](appendix.md) 的「`Page<T>` / `Cursor`」小节。
一句话记住：`OFFSET` 翻到几千页会越来越慢，keyset 游标是恒定代价；keyset 模式下 `total` / `pages` / `page` 恒为 0，前端用 `has_next()`。

## 4.7 计数与存在判断

```rust,no_run
use alun::prelude::*;
use std::result::Result;

#[derive(Model, Debug, Clone)]
pub struct SysUser {
    pub id: i64,
    pub email: String,
    pub status: i32,
}

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
// 计数：全表 / 条件
let total: i64 = SysUser::count(db()).await?;
let actives: i64 = SysUser::count_by(db(), "status", (1,)).await?;

// 存在判断：只查「有没有」，比查整行再判断更省
let exists: bool = SysUser::query().filter(SysUser::email().eq("a@b.com")).exists(db()).await?;

// 单个聚合值：取首行首列
let sum: i64 = db().scalar("SELECT COUNT(*) FROM sys_user").await?;
# let _ = (total, actives, exists, sum);
# Ok(())
# }
```

## 4.8 把它们串起来：一个用户管理模块

真实模块里六件事是混在一起用的。看一个完整（但很短）的例子，体会「同一系统、多需求共存」时的写法选择：

```rust,no_run
use alun::prelude::*;

#[derive(Model, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[table("sys_user")]
pub struct SysUser {
    pub id: i64,
    #[unique]
    pub email: String,
    pub name: String,
    pub status: i32,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(serde::Deserialize)]
struct CreateUserReq {
    email: String,
    name: String,
}

/// 列表：分页 + 可选关键字（条件可选见 5.1，这里用最简片段写法）
/// 注意全程没有一行 map_err —— DbError 由 From<DbError> for ApiError 自动转换
#[alun::get("/api/users")]
async fn list_users(Query(pq): Query<PageQuery>) -> ResResult<Res<PageData<Vec<SysUser>>>> {
    let page = SysUser::query()
        .paginate(db(), pq.page, pq.page_size)
        .await?;
    Ok(page.into_res())
}

/// 详情：记录不存在 → 404（`find` 找不到时返回 DbError::NotFound，自动转 404）
#[alun::get("/api/users/{id}")]
async fn get_user(Path(id): Path<i64>) -> ResResult<Res<SysUser>> {
    let user = SysUser::find(db(), id).await?;
    Ok(Res::ok(user))
}

/// 新增：唯一键冲突 → 409（自动映射，无需手写 match）
#[alun::post("/api/users")]
async fn create_user(AxumJson(req): AxumJson<CreateUserReq>) -> ResResult<Res<SysUser>> {
    let mut user = SysUser {
        id: 0,
        email: req.email,
        name: req.name,
        status: 1,
        deleted_at: None,
        created_at: Default::default(),
        updated_at: Default::default(),
    };
    user.insert(db()).await?;
    Ok(Res::ok(user))
}

/// 改状态：只改一列，不 SELECT
#[alun::patch("/api/users/{id}/status")]
async fn update_status(Path(id): Path<i64>, Query(q): Query<std::collections::HashMap<String, i32>>)
    -> ResResult<Res<()>>
{
    let status = q.get("status").copied().unwrap_or(1);
    SysUser::of(id)
        .set(SysUser::status(), status)
        .update(db())
        .await?;
    Ok(Res::ok_empty())
}

/// 删除：软删，无需额外代码
#[alun::delete("/api/users/{id}")]
async fn delete_user(Path(id): Path<i64>) -> ResResult<Res<()>> {
    SysUser::delete_by_pk(db(), id).await?;
    Ok(Res::ok_empty())
}

#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?.scan().start().await
}
```

**这个例子里每个需求的「简化选择」**：

| 需求 | 为什么这么写 |
| ---- | ---- |
| 列表分页 | `PageQuery` + `paginate` + `into_res()`，三行结束，字段名不用记 |
| 详情 | 直接 `find` + `?`——找不到自动 404，不需要 `ok_or_else` |
| 新增 | 直接 `insert` + `?`——唯一冲突自动 409，不需要手写 `match DbError` |
| 改状态 | 变更集 `of().set().update()`，省一次 SELECT |
| 删除 | 只写了「删除」，软删由 `deleted_at` 字段自动生效 |
| **错误处理** | **全程零 `map_err`**——`From<DbError> for ApiError` 自动完成 |

### `?` 到底做了什么

`DbError` 住在 `alun-db-core`，`ApiError` 住在 `alun-web`。按 Rust 的孤儿规则，
本 crate 可以为「外部类型 → 本地类型」实现 `From`，于是框架直接实现了：

```text
DbError::NotFound              → 404 记录不存在
DbError::UniqueViolation      → 409 数据重复：唯一约束冲突
DbError::ForeignKey/Check     → 409 数据约束冲突
DbError::Deadlock/序列化/锁超时 → 500 服务繁忙，请重试
其他（连接失败、查询失败…）    → 500 模糊文案
```

无论走哪条分支，`DbError` 原文（含驱动原因与 SQL）都写进 `internal_detail`，
**只进日志、绝不出现在响应体**——所以对外脱敏与对内可排查同时成立。

### 什么时候才需要手写 `map_err`

自动映射给的是**通用文案**。只有当你要给某个端点换一句更贴合业务的说法时才手写，
且**只写差异部分**：

```rust,ignore
// 唯一冲突想说得更具体：把 DbError 翻成自定义 409 文案
user.insert(db()).await.map_err(|e| match e {
    DbError::UniqueViolation { .. } => ApiError::conflict("该邮箱已注册，请直接登录"),
    other => ApiError::internal_masked("创建失败", other.to_string()),
})?;
```

注意这里的 `other` 分支仍然走 `internal_masked`——**漏一个分支就会把内部细节暴露给用户**。
所以：**默认用 `?`，确实需要定制文案时才 `map_err`。**

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [② 定义模型](modeling.md) | [④ 查询与数据操作技巧](recipes.md) → |

<sub>本分册属于「数据库实操（7 分册）」，共 7 篇。首篇 / 末篇可点「返回总目录」。</sub>

---
id: guides.db.recipes
title: 按需求查技巧：查询与数据操作
summary: 搜索条件可有可无、联表聚合与窗口函数、一个操作动多张表、批量导入 / 存在就更新、表名运行时才知道。
tags: [database, recipes, join, batch, upsert]
status: active
owner: alun-core-team
source_of_truth: [code, alun-db/src, alun-db-core/src]
depends_on: [guides.database]
last_reviewed: 2026-10-06
---

# 按需求查技巧：查询与数据操作

> 本文件是 [`database.md`](../database.md) 的 `recipes` 分册；总目录与阅读顺序见 [数据库指南](../database.md)。


这一章共 10 个需求，每个都是「什么情况下需要 + 最小示例 + 一句话建议」。
**新手可以先跳过，遇到问题时按标题回来查。**

## 5.1 搜索条件可有可无

**什么时候需要**：后台列表搜索 —— 前端可能只传 2 个条件，也可能传 5 个。
典型错误做法是 `WHERE 1=1` 加一堆 `if` 拼字符串，既啰嗦又容易漏掉参数化。

用 `sql!` 宏：插值一律转成占位符并绑定参数，`None` 的整段条件自动省略。

```rust,no_run
use alun::prelude::*;
use std::result::Result;

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
let status: Option<i32> = Some(1);
let keyword: Option<String> = None;

let rows: Vec<Row> = sql!(
    "SELECT * FROM sys_user WHERE 1=1 #and(status, '=', status) #and(name, 'LIKE', keyword)",
    " #orderBy(created_at desc) #limit(20)",
)
.find(db())
.await?;
# let _ = rows;
# Ok(())
# }
```

**常用指令**：

| 指令 | 语义 | 参数为 `None` 时 |
| ---- | ---- | ---- |
| `#where` / `#and` / `#or(field, op, param)` | 条件 | 整段省略 |
| `#orderBy(f1 desc, f2 asc)` | 排序 | 始终生效 |
| `#groupBy(f1, f2)` | 分组 | 始终生效 |
| `#limit(n)` | 限制行数 | 始终生效 |
| `#if(cond) ... #else ... #end` | 条件块（可嵌套，`cond` 是 Rust 布尔表达式） | `#else` 可选 |
| `#para(name)` | 把作用域内变量绑定成占位参数（配合 `#if` 用） | — |

操作符白名单：`= != <> > >= < <= LIKE "NOT LIKE" ILIKE`。
**不要**用 `sql!` 拼表名/列名 —— 插值只参数化**值**，标识符必须来自代码常量。

**终态动词**（`SqlQueryExt`，`&Db` / `&Transaction` 都行）：
`find` / `find_first` / `find_one` / `find_one_or_null` / `exists` / `paginate` /
`for_each` / `for_each_page` / `scalar` / `execute`。

**建议**：筛选项 ≤ 2 个且必填 → 直接 `find_by` + 元组参数，不必上宏；
筛选项多且可选 → `sql!` + `#and`；超大结果集要遍历 → `for_each_page`（`for_each` 会一次性取回）。

## 5.2 联表聚合、窗口函数、数据库特有语法

**什么时候需要**：报表、大宽表、`GROUP BY` 统计、CTE、窗口函数。
门面**不试图封装 SQL 全集**，这种场景直接写 SQL 最省心。

```rust,no_run
use alun::prelude::*;
use std::result::Result;

#[derive(Model, Debug, Clone)]
pub struct SysUser {
    pub id: i64,
    pub status: i32,
}

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
// ① 结果不是单一 Model 形状 → 取动态 Row 自己映射
let rows: Vec<Row> = db()
    .find("SELECT u.id, COUNT(o.id) AS cnt FROM sys_user u JOIN orders o ON o.user_id = u.id GROUP BY u.id")
    .await?;

// ② 结果恰好是某个 Model → 走 Model 的原始 SQL 脊柱，省一次手写映射
let users: Vec<SysUser> = SysUser::sql(sql!("SELECT * FROM sys_user WHERE status = ", { 1i32 }))
    .find(db())
    .await?;

// ③ 取单个聚合值
let total: i64 = db().scalar("SELECT COUNT(*) FROM sys_user").await?;

// ④ 带参数的值一律参数化绑定
let rows = db()
    .find_with_params("SELECT * FROM sys_user WHERE status = ?", vec![Value::I32(1)])
    .await?;
# let _ = (rows, users, total);
# Ok(())
# }
```

**注意**：`find_first*` 在 SQL 以 `SELECT` / `WITH` / `(` 开头且不含 `" LIMIT"` 时**会自动追加 `LIMIT 1`**，
所以不要自己再拼 `LIMIT` 造成重复。
`execute_script` 用来跑批量 DDL；MySQL 需要服务端开启多语句，否则返回 `UnsupportedFeature`。

## 5.3 一个操作要动多张表

**什么时候需要**：转账、下单扣库存、主从表一起写 —— 「要么全成、要么全不成」。
事务是**闭包式**的：返回 `Ok` 提交，返回 `Err` 回滚（回滚由驱动句柄的 `Drop` 兜底）。

```rust,no_run
use alun::prelude::*;
use alun::prelude::DbError;
use std::result::Result;

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
db().transaction(|tx| async move {
    tx.execute("UPDATE accounts SET balance = balance - 100 WHERE id = 'A'").await?;
    tx.execute("UPDATE accounts SET balance = balance + 100 WHERE id = 'B'").await?;

    let rows = tx.find("SELECT balance FROM accounts WHERE id = 'A'").await?;
    if rows.is_empty() {
        return Err(DbError::NotFound);   // 提前返回 → 自动回滚
    }
    Ok(())                                // 正常返回 → 自动提交
}).await?;
# Ok(())
# }
```

**最关键的一条边界**：事务借出了专属连接，**事务内的一切都必须走 `tx`**。
在事务里调用 `db()` 上的方法（或把 `db()` 传给 Model）会走**另一条连接**，不受事务保护、也不会回滚。

好消息是 Model 层 API 天然支持事务（首参是 `&impl SqlExecutor`）：

```rust,ignore
db().transaction(|tx| async move {
    SysUser::find(&tx, 1i64).await?;                          // 读走事务
    SysUser::of(1i64).set(SysUser::status(), 2).update(&tx).await?;  // 写走事务
    sql!("UPDATE accounts SET flag = ", { true }).execute(&tx).await?;
    Ok(())
}).await?;
```

跨函数边界要显式句柄：`let tx = db().begin().await?;` → 把 `&tx` 往下传 → `tx.commit()` / `tx.rollback()`（都幂等）。
同一段逻辑既要能跑在池上又要能跑在事务里，用 `SqlExecutor::in_tx`（`&Db` 开新事务，`&Transaction` 复用外层，**不支持嵌套事务**）。

**建议**：事务闭包尽量短，只包「必须原子」的写操作，把 HTTP 调用/文件写入挪到事务外；
`Deadlock` / `SerializationFailure` / `LockTimeout` 这类可重试错误在**事务外层**重试；
需要在事务内提前终止（例如幂等短路）就直接 `tx.commit()` / `tx.rollback()`。

## 5.4 批量导入 / 存在就更新

**什么时候需要**：Excel/CSV 导入、上游数据同步、初始化脚本。

```rust,ignore
// ① 同构批量（列完全一致、确定是新增）：最快
SysUser::insert_many(&rows, db()).await?;      // rows: &[SysUser]

// ② 异构批量（列不齐，如手工整理的导入表）
let mut a = Row::new().with_entity("sys_user", "id");
a.insert("name", "张三");
let mut b = Row::new().with_entity("sys_user", "id");
b.insert("name", "李四");
b.insert("email", "li@example.com");           // 字段集不同也能同批
let inserted: u64 = db().batch_insert("sys_user", &[a, b]).await?;
let updated: u64 = db().batch_update("sys_user", &rows_with_pk).await?;

// ③ 存在则更新（幂等写入）
user.upsert(db(), &["email"], &["name"]).await?;   // ON CONFLICT(email) DO UPDATE SET name
```

**必须知道的边界**：
- `insert_many` **不触发 Hook、不回填自增主键**；需要时间戳请在构造 `rows` 时自己填好，需要回填就逐条 `insert`。
- 它会按方言占位符上限自动分块（SQLite 999、PG / MySQL 65535），超限时**整批包在一个事务里**。
- `upsert` 只有 PG / SQLite 支持；MySQL 上返回 `UnsupportedFeature`，需要退回「先查后写」或写 `INSERT ... ON DUPLICATE KEY UPDATE` 原生 SQL。

**建议**：大批量 + 幂等 + 多方言 → `batch_insert` 写入临时表，再用原生 SQL 做 `MERGE` / `ON CONFLICT` 落库。

## 5.5 表名运行时才知道

**什么时候需要**：通用导入导出、低代码平台、多表后台、元数据驱动。

```rust,no_run
use alun::prelude::Sid;          // UUID 主键生成器
use alun::prelude::*;
use std::result::Result;

#[derive(Model, Debug, Clone)]
#[table("sys_user")]
pub struct SysUser {
    pub id: i64,
    pub name: String,
    pub status: i32,
}

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
// 动态行：表名 + 主键列 + 有序字段表；没有变更追踪，写回时整行提交
let mut row = Row::new().with_entity("sys_user", "id");
row.insert("id", Sid::uuid7());                 // UUID v7 主键（有序、无中心化依赖）
row.insert("name", "张三");
let inserted: Row = db().insert_row(&row).await?;   // 主键缺失/Null/0 视为自增

let found: Option<Row> = match inserted.pk_value() {
    Some(pk) => db().find_row_by_pk("sys_user", "id", pk.clone()).await?,
    None => None,
};

if let Some(mut row) = found {
    row.insert("name", "李四");
    db().update_row(&row).await?;                   // 整行写回：主键不参与 SET
    db().delete_row(&row).await?;
}

// 表达式拼装的 Builder（运行时定表名，但表达式类型仍编译期校验）
let page: Page<Row> = db()
    .page(1, 20)
    .table("sys_user")
    .filter(SysUser::status().eq(1))
    .order_by("created_at", Order::Desc)
    .exec(db())
    .await?;
# let _ = page;
# Ok(())
# }
```

**建议**：
- 只想改一列却走 `Row` 是不合适的 —— 用变更集（[4.4](crud.md) §4.4）或 `batch_update`。
- `Row` **不支持组合主键**（只携带一个主键列）；组合主键表请写原生 SQL。
- **想绕过麻烦就记住**：动态 `Row` 路径**不触发 Hook、不享受软删保护**，需要这些语义就走 Model 层。
- 表名/列名一律过 `validate_identifier`（只允许 `[a-zA-Z_][a-zA-Z0-9_]*`），所以**绝不能传用户输入**。

Builder 三个入口与终态：

| 入口 | 构造 | 链式修饰 | 终态 |
| ---- | ---- | ---- | ---- |
| `ModelQuery<T>`（强类型） | `User::query()` | `filter` `filter_fragment` `order_by` `limit` `offset` `page_after` | `find` `find_first` `find_one` `find_one_or_null` `exists` `paginate` `for_each` `for_each_page` `scalar` `execute` |
| `ModelQuery<T>`（原始 SQL） | `User::sql(sql!(...))` | **不可用**（会返回 `DbError::InvalidValue`） | 同上 |
| `FindBuilder` | `db().query()` | `table` `filter` `filter_fragment` `join` `order_by` `page_after` `limit` `offset` | `exec(&ex) -> Vec<Row>` / `first(&ex) -> Option<Row>` |
| `PageBuilder` | `db().page(p, s)` | `table` `filter` `order_by` `page_after` | `exec(&ex) -> Page<Row>` |
| `CountBuilder` | `db().count()` | `table` `filter` | `exec(&ex) -> i64` |

表达式能力（`FieldExpr<T>`）：比较 `eq ne gt ge lt le`、空值 `is_null is_not_null`、
区间 `between`、排序 `asc desc`、字符串 `like not_like ilike in_`、数值 `add sub mul div`。

**没有 `select(...)` 列投影 API** —— 投影结果无法构造 `T`。需要投影时用
`sql!("SELECT id, email FROM sys_user ...").find(db())` 取动态 `Row`。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [③ 增删改查](crud.md) | [⑤ 视图 / Hook / 迁移 / 多库 / 错误映射](ops.md) → |

<sub>本分册属于「数据库实操（7 分册）」，共 7 篇。首篇 / 末篇可点「返回总目录」。</sub>

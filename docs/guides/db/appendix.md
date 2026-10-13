---
id: guides.db.appendix
title: 附录：API 速查、反模式清单、已删除 API
summary: 按类型分组的 API 签名速查（Db/Transaction/CrudExt/ModelQuery/ChangeSet/Row/Page…）、反模式清单、本次重构已删除或改名的 API。
tags: [database, api, cheatsheet, antipattern]
status: active
owner: alun-core-team
source_of_truth: [code, alun-db/src, alun-db-core/src]
depends_on: [guides.database]
last_reviewed: 2026-10-06
---

# 附录：API 速查、反模式清单、已删除 API

> 本文件是 [`database.md`](../database.md) 的 `appendix` 分册；总目录与阅读顺序见 [数据库指南](../database.md)。


## 附录 A：API 速查

## 全局入口

| 符号 | 说明 |
| ---- | ---- |
| `db()` | `&'static Db`，未初始化 panic |
| `try_db()` | `Option<&'static Db>` |
| `alun_db::init(url)` / `init_with(url, cfg)` | 初始化全局单例 |
| `Db::connect(url)` / `connect_with(url, cfg)` | 建池 + `SELECT 1` 自检 |

## `Db` 查询与执行

| 方法 | 返回 |
| ---- | ---- |
| `find` / `find_with_params` | `Vec<Row>` |
| `find_first` / `find_first_with_params` | `Option<Row>` |
| `scalar::<T>` / `scalar_with_params::<T>` | `T` |
| `paginate` / `paginate_with_params` | `Page<Row>` |
| `execute` / `execute_with_params` | `u64`（受影响行数） |
| `execute_script` | `()` |
| `insert_returning_id` | `i64` |
| `query()` / `page(p, s)` / `count()` | Builder |
| `begin()` / `transaction(f)` | `Transaction` / 闭包式事务 |
| `in_tx(f)` | 事务内联 / 新事务（`SqlExecutor`） |
| ~~`migrate` / `migrate_down` / `migrate_from_dir` / `migrate_down_from_dir`~~ | **已移除**：迁移统一走 `Migrator`（见 [ops.md](ops.md) §5.8） |
| `find_row_by_pk` / `insert_row` / `update_row` / `delete_row` | 动态 `Row` CRUD |
| `batch_insert(table, &[Row])` / `batch_update(table, &[Row])` | `u64`（`RowBatchExt`） |
| `dialect()` / `config()` | 方言 / 配置 |

> `Db::find_as::<T>` / `find_as_with_params::<T>` **已删除**：需要强类型结果时走
> `User::sql(sql!(...)).find(&db)`。

## `Transaction`

`execute` / `execute_with_params` / `find` / `find_with_params` / `commit` / `rollback` / `dialect`
（`commit` / `rollback` 幂等；终结后再用报 `DbError::Transaction`）。
`Transaction` 也实现 `SqlExecutor` 与 `RowBatchExt`，可直接传给 Model / `sql!` / Builder。

## `CrudExt`（关联函数，首参 `&impl SqlExecutor`）

`of` / `query` / `sql` / `find` / `find_or_none` / `find_by` / `find_first_by` /
`find_in` / `find_in_ids` / `count` / `count_by` / `delete_by_pk` / `delete_by` /
`delete_in` / `delete_in_ids`

## `ModelQuery<T>`

`User::query()` / `User::sql(sql!(...))` 构造；链式 `filter` / `order_by` / `limit` / `offset` / `page_after`
（仅 `query()` 形态）；终态 `find` / `find_first` / `find_one` / `find_one_or_null` / `exists` /
`paginate` / `for_each` / `for_each_page` / `scalar` / `execute`。

## `ChangeSet<T>`

`User::of(pk)` 构造；`set(FieldExpr, value)` / `update(&ex)` / `delete(&ex)`。

## `PersistExt`（实例方法，首参 `&impl SqlExecutor`）

`insert` / `insert_or_update` / `update` / `delete` / `insert_many`（关联函数 `Model::insert_many(&rows, &ex)`）/ `upsert`

## `RowBatchExt`（`Db` 与 `Transaction` 共同实现）

`batch_insert(&self, table, &[Row]) -> u64` / `batch_update(&self, table, &[Row]) -> u64`

## `Row`

| 分类 | 方法 |
| ---- | ---- |
| 构造 / 元数据 | `new` `with_capacity` `with_entity` `set_table` `set_primary_key` `table` `primary_key` `pk_value` |
| 读写 | `insert` `get` `get_at` `name_at` `get_typed` `get_typed_at` `columns_match` |
| 迭代 | `len` `is_empty` `iter` `keys` `values` `contains` `into_entries` |

## `Page<T>` / `Cursor`

`Page`：`list` / `total` / `page` / `size` / `pages` + `has_next` / `has_prev` / `is_empty` / `map` / `calc_pages`。
`Cursor`：`key`（排序键）/ `pk`（主键），不变量是「同一行、非 Null」。

游标分页写法（深分页替代 `OFFSET`）：

```rust,no_run
use alun::prelude::*;
use std::result::Result;

# async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
let page: Page<Row> = db().page(1, 20).table("sys_user").order_by("id", Order::Asc).exec(db()).await?;

let last = page.list.last().expect("上一页非空");
let cursor = Cursor {
    key: last.get("id").cloned().unwrap_or(Value::Null),  // 排序键
    pk: last.get("id").cloned().unwrap_or(Value::Null),   // 主键（同一行、非 Null）
};

let next: Vec<Row> = db()
    .query()
    .table("sys_user")
    .order_by("id", Order::Asc)
    .page_after("id", "id", Order::Asc, Some(cursor))
    .exec(db())
    .await?;
# let _ = next;
# Ok(())
# }
```

> keyset 模式下 `Page` 的 `total` / `pages` / `page` 恒为 `0`，只返回本页 `list`。
> 排序键必须有唯一且稳定的顺序：单列排序时把同一列同时传给 `sort_col` 与 `pk_col`；
> 排序键可能重复时，`pk_col` 用于打破平局。

## `SqlQueryExt`（`sql!` 产物的终态动词）

`find` / `find_first` / `find_one` / `find_one_or_null` / `exists` / `paginate` / `for_each` /
`for_each_page` / `scalar` / `execute`（`E: SqlExecutor`，`&Db` 与 `&Transaction` 都行）。

## Builder 终态

`FindBuilder::exec(&ex)` → `Vec<Row>` / `FindBuilder::first(&ex)` → `Option<Row>` /
`PageBuilder::exec(&ex)` → `Page<Row>` / `CountBuilder::exec(&ex)` → `i64`（`ex: &impl SqlExecutor`）。

## 关键枚举

| 类型 | 变体 |
| ---- | ---- |
| `Order` | `Asc` / `Desc` |
| `NullsOrder` | `First` / `Last` |
| `JoinKind` | `Inner` / `Left` / `Right` / `Full` |
| `ConflictAction` | `DoNothing` / `DoUpdate { .. }` |
| `Placeholder` | `Question`（MySQL / SQLite）/ `Dollar`（PostgreSQL） |
| `Type` | `Bool` / `I8..I64` / `U8..U64` / `F32` / `F64` / `Text` / `Bytes` / `Date` / `Time` / `Timestamp` / `TimestampTz` / `Uuid` / `Ulid` / `Decimal` / `Json` / `Enum` / `Inet` / `Cidr` / `Array` / `Nullable` |

## 错误类型

`DbError`：`Connection` / `Query` / `NotFound` / `Readonly` / `UniqueViolation` / `ForeignKeyViolation` /
`CheckViolation` / `Deadlock` / `SerializationFailure` / `LockTimeout` / `ColumnNotFound` / `TypeMismatch` /
`UnexpectedNull` / `UnsupportedFeature` / `Transaction` / `InvalidValue` / `Migration`；
`alun_db::prelude::Result<T>` 即 `Result<T, DbError>`。

---


## 附录 B：反模式清单

| 反模式 | 问题 | 正确做法 |
| ------ | ---- | -------- |
| 在事务里调 `db()` 而不是 `tx` | 走另一条连接，不受事务保护 | 事务内统一传 `&tx` |
| 把用户输入拼进 `find_by` 的字符串片段 | SQL 注入（片段是直接拼接的） | 用 `?` 占位符 + `Params`，或 `sql!` 插值 |
| 用 `find_by("age > ?", (18,))` 这类两参数写法 | 单参数才走「字段 = ?」分支，两参数按片段处理导致占位符对不上 | 单字段用 `find_by(&db, "age", (18,))`，复杂条件写完整 WHERE 片段或改用 `query().filter(...)` |
| 把用户输入当表名 / 列名传给 Builder | `validate_identifier` 会拒绝，或语义错误 | 表名 / 列名只能来自代码常量 |
| 在 `sql!` / `find_by` 里拼 `LIMIT` 做分页 | 与 `find_first*` 的自动 `LIMIT 1` 冲突，且拿不到总数 | 用 `query().paginate(&db, page, size)` / `PageBuilder` |
| 在 `User::sql(sql!(...))` 上再调 `filter` / `order_by` / `limit` | 原始 SQL 脊柱不支持链式修饰 | 条件写进 `sql!`，或改用 `User::query()` |
| 想要列投影却去找 `select(...)` | **不存在**该 API（投影结果无法构造 `T`） | 用 `sql!("SELECT id, email FROM ...").find(&db)` 取动态 `Row` |
| 深分页仍用 `OFFSET` | 越翻越慢 | 换 `page_after` + `Cursor` |
| 拿 keyset 结果的 `total` 渲染总页数 | keyset 模式 `total` / `pages` / `page` 恒为 0 | 用 `has_next()` |
| 以为 `insert_many` / `batch_insert` 会跑 Hook / 回填主键 | 拿到的主键仍是占位值，时间戳为空 | 构造行时自行填好；需要回填就逐条 `insert` |
| 以为动态 `Row` 写操作会跑 Hook / 软删 | 无审计、软删数据仍出现在列表 | 需要这些语义就走 Model 层 |
| `unwrap()` 处理 `db()` / `get_typed` | 未初始化 panic、类型不符 panic | 用 `try_db()`、`?`、`unwrap_or_default()` |
| 手动搬 `Page.size` 到响应 | 响应字段叫 `page_size`，字段名对不上 | 用 `into_res()` / `into_page_data()` |
| 修改已发布的迁移文件 | checksum 校验失败 | 追加新版本 |
| `upsert` 跑 MySQL | 运行时 `UnsupportedFeature` | 见 [recipes.md §5.4](recipes.md) |
| 给非 `Option` 的 `deleted_at` 字段加 `#[soft_delete]` | 非 `Option` 的 `deleted_at` 本身会编译报错 | 用 `Option<...>`，或 `#[model(hard_delete)]` 关掉软删 |

**不存在的 API**（旧文档 / 设计稿中的写法，勿用）：

## 附录 C：本次重构已删除或改名的 API

`Row::set` / `Row::get_as` / `Row::get_id` / `Row::has` / `Row::clear_changes` / `Row::primary_keys`、
`Row::table("t")` 构造器、`Db::insert` / `update` / `delete` / `find_by_id` / `delete_by_id` / `query_page`、
`User::where_`、`FieldExpr::json_get`、`HookChain` / `NullHook`、`Isolation`、`IdKind`、
`DbResult` / `ActiveTx`、`alun_db::PageQuery`、`#having` / `#join` 模板指令、`alun migrate` CLI。

---


| 旧写法 | 新写法 |
| ------ | ------ |
| `Model::find_by_pk` / `find_or_none_by_pk` | `find` / `find_or_none` |
| `Model::all` | `Model::query().find(&db)` |
| `Model::paginate` | `Model::query().paginate(&db, page, size)` |
| `Model::find_where` / `find_first_where` / `count_where` / `delete_where` | `find_by` / `find_first_by` / `count_by` / `delete_by` |
| `PersistExt::save` | `insert`（纯 INSERT）或 `insert_or_update`（按主键占位判定） |
| `PersistExt::update_columns` | 变更集 `User::of(pk).set(...).update(&db)` |
| `Model::insert_batch` | `Model::insert_many`（同构）/ `RowBatchExt::batch_insert`（异构 `Row`） |
| `sql!` 链式 `fetch_all` / `fetch_one` / `fetch_or_none` | `find` / `find_one` / `find_first` |
| `Db::find_as` / `find_as_with_params` | `User::sql(sql!(...)).find(&db)` |
| `alun_db::hook` 模块 | `alun_db_core::hook`（`TimestampHook` 由 prelude 导出） |
| Builder 的 `exec` / `first` / 终态动词只接 `&Db` | 泛化为 `&impl SqlExecutor`（事务内传 `&tx`） |

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [⑥ 多租户隔离](multi-tenant.md) | [返回总目录](../database.md) → |

<sub>本分册属于「数据库实操（7 分册）」，共 7 篇。首篇 / 末篇可点「返回总目录」。</sub>

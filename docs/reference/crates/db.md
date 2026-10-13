---
id: reference.crates.db
title: `alun-db` — 数据库门面
summary: `alun-db` — 数据库门面——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [db, reference, sql, orm, migration, tenant]
status: active
owner: alun-core-team
source_of_truth: [alun-db/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-db` — 数据库门面

> 本文件是 [`crates.md`](../crates.md) 的 `db` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun-db/src/`（门面 crate）

数据访问门面：**SQL 级 API（统一写 `?` 占位符，PostgreSQL 执行前自动改写为 `$1`/`$2`）+ Row 模式动态行 + RAII 事务 + 同步 Hook 生命周期 + `Model` derive 与 `CrudExt`/`PersistExt` + 强类型查询脊柱 `ModelQuery` + 变更集 `ChangeSet` + Builder + 迁移**。核心类型（`Row`/`Model`/`Hook`/`Page`/`Expr`/`Value`…）定义在 `alun-db-core`，SQL 渲染与方言在 `alun-db-sql`，三个驱动见 [`alun-db` 家族](db-family.md)。

对齐 aifei（`Db`/`Dao`/`Row`）的设计原则：**SQL 表达语义，链式 API 只负责选择执行方式（终态动词）**。

## 模块与再导出

| 模块                   | 内容                                                       |
| -------------------- | -------------------------------------------------------- |
| `alun_db::config`    | `DatabaseConfig`、`PoolConfig`                             |
| `alun_db::db`        | `Db`（SQL 级方法）                                            |
| `alun_db::rowops`    | `Db` 的动态行写操作（`impl Db`）与 `RowBatchExt`（异构批量）             |
| `alun_db::builder`   | `FindBuilder`、`PageBuilder`、`CountBuilder`                |
| `alun_db::crud`      | `CrudExt`（`impl<T: Model> CrudExt for T`）                 |
| `alun_db::persist`   | `PersistExt`（`impl<T: Model> PersistExt for T`）           |
| `alun_db::model_query` | `ModelQuery<T>` —— 强类型链式查询脊柱（`Model::query()` / `Model::sql`） |
| `alun_db::changeset` | `ChangeSet<T>` —— 变更集（`Model::of(pk)`）                   |
| `alun_db::tx`        | `Transaction` 与 `Db::begin` / `Db::transaction`           |
| `alun_db::migrate`   | `Migrator` / `Migration` / `MigratorOptions` / `Version` 等  |
| `alun_db::query`     | `SqlExecutor`、`SqlQueryExt`                               |
| `alun_db::factory`   | 驱动选择与连接池创建（`pub(crate)`，**无任何公开函数**）                       |
| `alun_db::prelude`   | `alun_db_core::prelude::*`（`Row`/`Model`/`Hook`/`Value`/`Type`/`Page`/`Expr`/`DbError`/`Result`…）+ Derive 宏 `Model`/`Row`/`TypeMapper`/`sql` + `Db`/`Transaction`/`Migrator`/`Migration`/`MigratorOptions`/`Version`/`TimestampHook`/`CrudExt`/`PersistExt`/`ModelQuery`/`ChangeSet`/`RowBatchExt`/三个 Builder/`SqlExecutor`/`SqlQueryExt`/`DatabaseConfig`/`PoolConfig` + `db()`/`try_db()`/`init()`/`init_with()` |

> **`alun_db::hook` 模块已删除**：`TimestampHook` / `now_value` / Hook 洋葱执行器（`run_before_*` / `run_after_*`）
> 全部迁到 **`alun_db_core::hook`**；`TimestampHook` 由 `alun_db::prelude` 再导出，
> 写 `#[hook(...)]` 时直接写 `TimestampHook` 或全路径 `alun_db_core::hook::TimestampHook`。
> `Hook` trait 本身一直在 `alun_db_core::traits::hook`。

crate 根直接再导出：`Db`、`Row`、`Page`、`DatabaseConfig`、`PoolConfig`、`Migration`、`Migrator`、`MigratorOptions`、`Version`、`Transaction`。

**全局实例**：

| 函数                                  | 说明                                        |
| ----------------------------------- | ----------------------------------------- |
| `init(url)`                         | 创建全局 `Db` 单例（`OnceLock`，重复调用返回错误）          |
| `init_with(url, config)`            | 同上，附带 `DatabaseConfig`                    |
| `db()`                              | 取 `&'static Db`，未初始化时 panic                |
| `try_db()`                          | 安全版本，返回 `Option<&'static Db>`              |
| `Db::connect(url)` / `connect_with` | 创建独立实例，不写全局                               |

## 关键结构体

### `Db` （[db.rs](../../../alun-db/src/db.rs) / [rowops.rs](../../../alun-db/src/rowops.rs)）

`Db` 是 `Clone` 的轻量句柄（内部共享连接池），全部方法取 `&self`。

| 方法                                                              | 说明                              |
| --------------------------------------------------------------- | ------------------------------- |
| `connect(url)` / `connect_with(url, cfg)`                       | 建连并选择方言（按 URL/`db_type`）         |
| `config()` / `dialect()`                                        | 取配置 / 方言实现                      |
| `find(sql)`                                                     | 查询多行，无参数                        |
| `find_with_params(sql, Vec<Value>)`                             | 查询多行，带参数                        |
| `find_first(sql)` / `find_first_with_params(sql, params)`        | 查询单行（`Option<Row>`）              |
| `scalar::<T>(sql)` / `scalar_with_params::<T>(sql, params)`      | 取单个标量（`T: TypeMapper`）          |
| `paginate(sql, page, size)`                                     | 分页查询，返回 `Page<Row>`              |
| `paginate_with_params(sql, params, page, size)`                 | 同上，带参数                          |
| `execute(sql)` / `execute_with_params(sql, params)`              | 执行写操作，返回影响行数 `u64`               |
| `execute_script(sql)`                                           | 一次执行多条语句（建表脚本等）                 |
| `insert_returning_id(sql, params)`                              | 插入并回取自增主键 `i64`                 |
| `query()` / `page(page, size)` / `count()`                      | 返回 `FindBuilder` / `PageBuilder` / `CountBuilder` |
| `begin()`                                                       | 开启事务，返回 `Transaction`            |
| `transaction(closure)`                                          | 事务闭包，`Ok` → commit，`Err` → rollback |
| （迁移 API 不在 `Db` 上）                                          | 迁移统一走 `Migrator`，见“迁移”小节          |

`SqlExecutor` 提供的通用入口（`Db` 与 `Transaction` 都实现）：

| 方法 | 说明 |
| ---- | ---- |
| `exec_sql` / `query_sql` / `query_first_sql` / `scalar_sql` / `paginate_sql` | Model / `sql!` / Builder 的统一执行入口 |
| `insert_returning_id_sql` | 插入并回取自增主键 |
| `in_tx(f)` | `&Db` 开新事务、`&Transaction` 内联执行（不支持嵌套事务） |

动态行写操作（`rowops.rs`，均为 `impl Db`）：

| 方法                                        | 说明                    |
| ----------------------------------------- | --------------------- |
| `find_row_by_pk(table, pk_column, pk)`    | 按主键查 `Row`（主键列名显式传入）   |
| `insert_row(&Row)`                        | 插入 `Row`，返回回填后的 `Row` |
| `update_row(&Row)`                        | 按主键更新该行全部字段           |
| `delete_row(&Row)`                        | 按主键删除                 |

异构批量（`RowBatchExt`，`Db` 与 `Transaction` 都实现，prelude 导出）：

| 方法 | 说明 |
| ---- | ---- |
| `batch_insert(table, &[Row]) -> u64` | 各行字段集可不同；列集相同者合并为多行 `VALUES`，多条语句包一个事务 |
| `batch_update(table, &[Row]) -> u64` | 只写各行携带的非主键字段，多条语句整批一个事务 |

> 已不存在 `Db::insert/update/delete/find_by_id/delete_by_id/query_page`：写路径只有 SQL 级的
> `execute*`、Row 级的 `insert_row`/`update_row`/`delete_row`/`batch_insert`/`batch_update`，
> 以及 `PersistExt`（Model）与 Builder。
> 也已不存在 `Db::find_as::<T>` / `find_as_with_params::<T>`（与 `ModelQuery` 重复）：
> 需要强类型结果时用 `SysUser::sql(sql!("SELECT ... WHERE x = ", { v })).find(&db)`，结果直接是 `Vec<SysUser>`。

### `Row` （[row.rs](../../../alun-db-core/src/row.rs)）

像操作 Map 一样承载一行数据。`entries` / `table` / `primary_key` **全部私有**；行**没有变更追踪机制**（旧文档的 `changes` 字段已删除）。

| 方法                                                       | 说明                                     |
| -------------------------------------------------------- | -------------------------------------- |
| `Row::new()` / `Row::with_capacity(n)`                   | 创建空行                                   |
| `Row::with_entity(table, pk)`                            | 创建并指定表名与主键列（链式）                        |
| `set_table(name)` / `set_primary_key(name)`              | 设置表名 / 主键列（`&mut Self`）                 |
| `table()`                                                | 读取表名，`Option<&str>`                    |
| `primary_key()`                                          | 主键列名，未设置时默认 `"id"`                      |
| `pk_value()`                                             | 主键值，`Option<&Value>`                    |
| `insert(name, value)`                                    | 写入字段（旧文档的 `set()` 对应此方法）                |
| `get(name)`                                              | 读取字段，`Option<&Value>`                  |
| `get_typed::<T>(name)`                                   | 按 `TypeMapper` 取类型化值                    |
| `contains(name)`                                         | 字段是否存在（旧文档的 `has()` 对应此方法）              |
| `len()` / `is_empty()`                                   | 字段数量 / 是否为空                            |
| `keys()` / `values()` / `iter()`                         | 遍历字段名 / 值 / 键值对                         |
| `get_at(i)` / `name_at(i)` / `get_typed_at::<T>(name, i)` | 按下标访问                                  |
| `columns_match(&["a", "b"])`                             | 校验列名序列（驱动解码用）                          |
| `into_entries()`                                         | 取出 `Vec<(String, Value)>`（并实现 `FromIterator`） |

### `Transaction` （[tx.rs](../../../alun-db/src/tx.rs)）

事务句柄（同步构造、异步方法），由 `Db::begin()` 或 `Db::transaction()` 产生。

| 方法                                                    | 说明               |
| ----------------------------------------------------- | ---------------- |
| `execute(sql)` / `execute_with_params(sql, params)`   | 事务内写操作           |
| `find(sql)` / `find_with_params(sql, params)`         | 事务内查询            |
| `dialect()`                                           | 方言               |
| `commit()` / `rollback()`                             | 显式结束事务（均幂等）      |

- `Db::transaction(|tx: Transaction| async move { … Ok(()) })`：闭包**按值接收** `Transaction`，返回 `Result<T, DbError>`；返回 `Ok` 自动提交，返回 `Err` 自动回滚。**不再返回 `(ActiveTx, DbResult<T>)` 元组**。
- Drop 时未终结的事务自动回滚（RAII）；**没有隔离级别参数**，也没有 `set_rollback_only()` 之类的强制回滚标记。能力对应关系是 `Transaction::find*`（旧 `query_one`）。

### 迁移：`Migrator` 与 `Migration` （[migrate/mod.rs](../../../alun-db/src/migrate/mod.rs)）

```rust,ignore
pub struct Migration {
    pub version: Version,   // 通用版本号："1" / "1.2" / "20250101_120000"
    pub name: String,
    pub up: String,
    pub down: String,       // 单向迁移为空串
}
impl Migration {
    pub fn new(
        version: impl Into<Version>,
        name: impl Into<String>,
        up: impl Into<String>,
        down: impl Into<String>,
    ) -> Self;
}

pub struct Migrator { /* 来源（多目录 / 内存清单）+ 策略（MigratorOptions） */ }
impl Migrator {
    pub fn new() -> Self;
    pub fn source_dir(self, dir: impl Into<PathBuf>) -> Self;   // 可多次调用，版本号跨目录全局唯一
    pub fn source_seed(self, dir: impl Into<PathBuf>) -> Self;  // 种子数据目录
    pub fn options(self, options: MigratorOptions) -> Self;
    pub fn from_dir(dir: impl Into<PathBuf>) -> Self;           // 单目录
    pub fn from_migrations(list: impl IntoIterator<Item = Migration>) -> Self;

    pub async fn run(&self, db: &Db) -> Result<usize>;                // 应用待应用迁移
    pub async fn rollback(&self, db: &Db) -> Result<Option<Version>>; // 回滚最近一次（LIFO）
    pub async fn status(&self, db: &Db) -> Result<Vec<MigrationStatus>>;
    pub async fn plan(&self, db: &Db) -> Result<Vec<MigrationStatus>>;
    pub async fn baseline(&self, db: &Db, baseline: Version) -> Result<usize>;
    pub async fn repair(&self, db: &Db) -> Result<RepairReport>;
    pub async fn dry_run(&self, db: &Db) -> Result<DryRunReport>;
}
```

| 入口                                                     | 说明                                |
| ------------------------------------------------------ | --------------------------------- |
| `Migrator::from_dir(&dir).run(&db)`                    | 从目录加载并应用未应用的 up，返回应用数          |
| `Migrator::from_migrations(vec![…]).run(&db)`          | 应用内存清单                            |
| `Migrator::from_dir(&dir).rollback(&db)`               | 回滚最近一次应用的迁移（LIFO），返回 `Option<Version>` |
| `Migrator::new().source_dir(a).source_dir(b).run(&db)` | 多来源目录合并应用                          |
| `Migrator::from_dir(&dir).status(&db)` / `.plan(&db)`  | 观察每条迁移状态 / 待应用清单                  |
| `Migrator::from_dir(&dir).baseline(&db, Version::new("3"))` | 存量库把 `<= v3` 标记为已应用（不执行 SQL）      |
| `Migrator::from_dir(&dir).repair(&db)` / `.dry_run(&db)` | 修复记录表 / 预演（执行后回滚，不落库）             |

> `Db` 上**不再有** `migrate` / `migrate_down` / `migrate_from_dir` / `migrate_down_from_dir` 方法，也没有同名自由函数；`Migration` 也不再是 `Copy` 的 `&'static str` 常量（`new` 已不是 `const fn`），因此清单需在运行时构造。`MigratorOptions` 提供 `split_statements` / `up_down_sections` / `version_prefix` / `name_separator` / `transaction_default` / `validate_checksum` / `divergent`（默认 `Abort`）/ `out_of_order` / `import_legacy` / `baseline_on_migrate` / `baseline_version`；其中 `import_legacy`（导入旧 `_sqlx_migrations`）**默认关闭**，存量库接入时需显式开启。

### 配置：`DatabaseConfig` / `PoolConfig` （[config.rs](../../../alun-db/src/config.rs)）

```rust,ignore
pub struct DatabaseConfig {
    pub db_type: String,          // "sqlite" / "postgres" / "mysql"
    pub url: String,              // 不含密码
    pub password: Option<String>,
    pub pool: PoolConfig,
}
impl DatabaseConfig {
    pub fn from_url(url: &str) -> Self;                              // 按 URL 推断 db_type
    pub fn with_password_from_env(self, env_var: &str) -> Self;      // 密码取自环境变量
}

pub struct PoolConfig {
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout: Duration,
    pub idle_timeout: Option<Duration>,
    pub max_lifetime: Option<Duration>,
}
```

`PoolConfig::default()`：最大连接 10、最小 1、连接超时 30s、空闲超时 600s、最长存活 1800s。

> 注意有两个同名 `PoolConfig`：`alun_db::config::PoolConfig`（门面，`Duration` 字段）与 `alun_db_core::traits::driver::PoolConfig`（core），由门面的 `From` 实现转换。密码只支持从环境变量读取，**没有** `password_encrypted` 之类的加密密码字段。

## 分页体系：`PageQuery`（core）↔ `Page<T>`（db-core）

全框架只有**一个** `PageQuery`，定义在 `alun_web::api`（`alun_db::PageQuery` 已删除）：

| 成员                                                       | 说明                                                          |
| -------------------------------------------------------- | ----------------------------------------------------------- |
| `PageQuery { page: u64, page_size: u64 }`                | `Deserialize`；**反序列化即规整**，`?page=0`、参数缺省都不会 400、不会 panic       |
| `PageQuery::new(page, page_size)`                        | 规整到 `page >= 1`、`1 <= page_size <= 1000`（`page_size = 0` 回落默认 10） |
| `offset()` / `limit()`                                   | SQL OFFSET / LIMIT，越界值先规整，不会下溢                                |
| `calc_pages(total)`                                      | 计算总页数，口径与 `Page::calc_pages` 一致                               |
| `DEFAULT_PAGE_SIZE = 10` / `MAX_PAGE_SIZE = 1000`        | `alun_core` 常量（门面 prelude 已导出）                               |

数据库侧容器 `alun_db_core::page::Page<T>`（门面再导出为 `alun_db::Page`）：

| 字段      | 类型        | 说明                        |
| ------- | --------- | ------------------------- |
| `list`  | `Vec<T>`  | 数据列表                      |
| `total` | `u64`     | 总条数                       |
| `page`  | `u64`     | 当前页码                      |
| `size`  | `u64`     | 每页条数（**是 `size`，不是 `page_size`**） |
| `pages` | `u64`     | 总页数                       |

方法：`has_next()`、`has_prev()`、`is_empty()`、`map(f)`、`Page::calc_pages(total, size)`，并实现 `IntoIterator`。

响应侧容器是 `alun_core::PageData<T>`（字段 `list` / `total` / `page` / `page_size`）。两者的转换唯一入口是 `alun_web::PageResponseExt`（`#[cfg(feature = "db")]`）：`into_page_data()`（`Page<T> → PageData<Vec<T>>`）与 `into_res()`（`Page<T> → Res<PageData<Vec<T>>>`）。`Db::paginate` 与 `PageBuilder` 自身也做 `page.max(1)` / `size.max(1)` 下界保护。

```rust,no_run
use alun_db::prelude::*;                       // Db/Row/Page/Value…
use alun_core::{ApiError, PageQuery, Res};
use alun_web::PageResponseExt;

#[derive(serde::Serialize)]
struct ItemRes { id: i64 }

# async fn demo(pq: PageQuery) -> std::result::Result<Res<alun_core::PageData<Vec<ItemRes>>>, ApiError> {
let page = db()
    .paginate("SELECT id FROM items ORDER BY id", pq.page, pq.page_size)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .map(|r| ItemRes { id: r.get_typed("id").unwrap_or_default() });   // Row → DTO（Row 不实现 Serialize）
Ok(page.into_res())   // Page<ItemRes> → Res<PageData<Vec<ItemRes>>>
# }
```

## 关键 Trait

### `Model` （[traits/model.rs](../../../alun-db-core/src/traits/model.rs)）

```rust,ignore
pub trait Model: Sized + Send + Sync + 'static {
    const TABLE: &'static str;
    const READONLY: bool;
    const PRIMARY_KEY: &'static str;
    const SOFT_DELETE: Option<&'static str> = None;
    fn primary_key_column() -> &'static str;
    fn fields() -> &'static [FieldMeta];
    fn from_row(row: Row) -> Result<Self>;
    fn as_row(&self) -> Result<Row>;
    fn pk_value(&self) -> Value;
    fn pk_row(value: Value) -> Row;
    fn hooks() -> &'static [&'static dyn Hook] { &[] }
}
```

`Model` 由 `#[derive(Model)]` 生成，属性见“派生宏”小节；`fields()` 来自编译期生成的 `static` 元数据（零运行时开销）。

### `Hook` （[traits/hook.rs](../../../alun-db-core/src/traits/hook.rs)）—— **同步方法**

```rust,ignore
pub trait Hook: Send + Sync + 'static {
    fn before_insert(&self, _row: &mut Row) -> Result<()> { Ok(()) }
    fn after_insert(&self, _row: &Row) -> Result<()> { Ok(()) }
    fn before_update(&self, _row: &mut Row) -> Result<()> { Ok(()) }
    fn after_update(&self, _row: &Row) -> Result<()> { Ok(()) }
    fn before_delete(&self, _pk: &Value) -> Result<()> { Ok(()) }
    fn after_delete(&self, _pk: &Value) -> Result<()> { Ok(()) }
}
```

- **同步**方法（不是 `async_trait`），错误类型为 `alun_db_core::error::Result`。
- 删除钩子参数是主键 **`&Value`**（旧文档的 `table: &str, id: &str` 已删除）。
- **不存在 `HookChain`、`NullHook`**，也没有 Hook 工厂函数。多个 Hook 直接在 `Model` 上逐个声明：

```rust,ignore
#[derive(Model)]
#[table("users")]
#[hook(TimestampHook)]     // 每个 Hook 一条属性，可写多条；写 TimestampHook 时不再自动挂载
#[hook(AuditHook)]
struct User { /* … */ }
```

`Model::hooks()` 汇总这些 Hook；内置的唯一实现是 `TimestampHook`。
A 批约定：Model 含 `created_at` / `updated_at` 字段时**自动挂载** `TimestampHook`（排在 Hook 链最前，
且**强制覆盖**这两列，语义对齐 Rails `timestamps`），无需显式声明；显式声明的作用是决定它在链中的位置。
`created_at` 承载业务时间（开票、记账、事件时刻）的模型写 `#[model(no_timestamps)]` 关闭自动挂载，
由调用方自行取值。

**`TimestampHook`**：单元结构体，`before_insert` 写 `created_at`、`before_update` 写 `updated_at`，列名固定，**没有** `TimestampHook::new("created_at", "updated_at")` 构造器。
它定义在 **`alun_db_core::hook`**（随 `now_value` 与 Hook 洋葱执行器 `run_before_*` / `run_after_*` 一起从
`alun-db` 迁出；`alun_db::hook` 模块已删除）；`alun_db::prelude` 已导出 `TimestampHook`，
`#[hook(...)]` 里直接写 `TimestampHook` 即可（全路径为 `alun_db_core::hook::TimestampHook`）。

自定义 Hook（同步实现）：

```rust,no_run
use alun_db::prelude::*;

struct AuditHook;

impl Hook for AuditHook {
    fn after_insert(&self, row: &Row) -> Result<()> {
        tracing::info!("新增 {}: {:?}", row.table().unwrap_or("?"), row.pk_value());
        Ok(())
    }
}
```

### `CrudExt` （[crud.rs](../../../alun-db/src/crud.rs)，对 `T: Model` 的 blanket impl）

关联函数（第一个参数是 `&impl SqlExecutor`；事务外传 `&Db`，事务内传 `&tx` 即纳入该事务）：

| 方法                                                       | 说明                          |
| -------------------------------------------------------- | --------------------------- |
| `T::of(pk) -> ChangeSet<T>`                              | 只带主键构造引用对象（见下）              |
| `T::query() -> ModelQuery<T>`                            | 链式强类型查询（表名固定，自动追加软删过滤）        |
| `T::sql(sql!(...)) -> ModelQuery<T>`                     | 原始 SQL 脊柱（不做软删过滤，`WHERE` 自己写） |
| `T::find(&db, pk) -> Result<T>`                          | 按主键查，未命中返回 `DbError::NotFound` |
| `T::find_or_none(&db, pk) -> Result<Option<T>>`           | 按主键查，未命中返回 `None`           |
| `T::find_by(&db, where_or_field, params) -> Result<Vec<T>>` | `where_or_field` 双态查询（见下）    |
| `T::find_first_by(&db, where_or_field, params) -> Result<Option<T>>` | 同上，返回首条        |
| `T::find_in(&db, field_expr, values) -> Result<Vec<T>>`   | 单列 `IN`（列名编译期校验）             |
| `T::find_in_ids(&db, ids) -> Result<Vec<T>>`              | 主键 `IN`                     |
| `T::count(&db) -> Result<i64>`                            | 全表计数（软删模型自动排除已删行）           |
| `T::count_by(&db, where_or_field, params) -> Result<i64>` | 条件计数                        |
| `T::delete_by_pk(&db, pk) -> Result<u64>`                 | 按主键删除（软删模型改写为标记删除）          |
| `T::delete_by(&db, where_or_field, params) -> Result<u64>` | 条件删除                       |
| `T::delete_in(&db, field_expr, values) -> Result<u64>` / `delete_in_ids(&db, ids)` | `IN` 删除 |

**`where_or_field` 双态签名**（`find_by` / `find_first_by` / `count_by` / `delete_by` 的第一个参数）：

```rust,ignore
User::find_by(&db, "age", (18,)).await?;                        // 字段名 + 值 → age = ?
User::find_by(&db, "age = ? ORDER BY id DESC", (18,)).await?;   // WHERE 片段（自带占位符）
```

规则：**只有一个参数且该参数是合法标识符**时按「字段 = ?」处理，否则整段按 WHERE 片段处理
（这是 aifei 刻意的取舍：一个签名覆盖两种高频写法，不为单字段提供额外重载）。
字段名走标识符白名单校验；片段是直接拼接的，不能拼用户输入。

> 没有 `T::all` / `T::paginate`：全表用 `T::query().find(&db)`，分页用
> `T::query().paginate(&db, page, size) -> Page<T>`。静态按主键删除**保留 `delete_by_pk` 之名**
> （实例侧 `PersistExt::delete(&self, ex)` 已占用 `delete`，同名会在 `User::delete(&db, pk)` 处产生 E0034 歧义）。

### `ModelQuery<T>` （[model_query.rs](../../../alun-db/src/model_query.rs)）

两条构造入口，终态动词一致：

| 入口 | 链式修饰 | 说明 |
| ---- | -------- | ---- |
| `T::query()` | `filter(Expr)` / `filter_fragment(Expr)` / `order_by(OrderBy)` / `limit(u64)` / `offset(u64)` / `page_after(sort_col, pk_col, Order, Option<Cursor>)` | 表名 + 链式条件，自动追加软删过滤；`filter_fragment` 为末位片段（不加括号，可自带 `ORDER BY`/`LIMIT`） |
| `T::sql(sql!(...))` | **不可用**（调用后终态返回 `DbError::InvalidValue`） | 原始 SQL 脊柱，不做软删过滤，`WHERE` 自己写 |

终态动词（`E: SqlExecutor`）：`find(&ex)`、`find_first(&ex)`、`find_one(&ex)`、`find_one_or_null(&ex)`、
`exists(&ex)`、`paginate(&ex, page, size)`、`for_each(&ex, |t| Ok(true))`、
`for_each_page(&ex, size, |page| Ok(true))`、`scalar::<V, _>(&ex)`、`execute(&ex)`。

- `find_one`：**恰好一行**，0 行 → `DbError::NotFound`，多于一行 → `DbError::InvalidValue`
  （内部用 `LIMIT 2` 探测，不需全量取回）。
- `for_each`：结果一次性取回后遍历（协议无游标）；超大结果集用 `for_each_page`
  （真分页，`Page.total` / `pages` 为 0，`page` 为当前页号）。
- **没有** `select(...)` 列投影 API（投影结果无法构造 `T`）；需要投影时用
  `sql!("SELECT id, email FROM user ...").find(&db)` 取动态 `Row`。

### `ChangeSet<T>` （[changeset.rs](../../../alun-db/src/changeset.rs)）

`T::of(pk)` 构造，**只带主键、无需先 SELECT**：

```rust,ignore
User::of(123)
    .set(User::name(), "james")     // 键是 FieldExpr：字段名 + 值类型编译期校验
    .set(User::status(), Status::Paid)
    .update(&db).await?;            // 只 UPDATE 被 set 的列
User::of(123).delete(&db).await?;   // 免 SELECT 删除（软删模型自动改写）
```

- SET 目标**只含被 `set` 过的列**，未 set 的列保持数据库原值；同一列多次 `set` 保留最后一次。
- 传给 Hook 的行**只含主键 + 本次变更列**。
- `set` 主键、未 `set` 任何列、字段不属于该模型（`field.table() != T::TABLE`）→ `DbError::InvalidValue`。

### `PersistExt` （[persist.rs](../../../alun-db/src/persist.rs)，对 `T: Model` 的 blanket impl）

实例方法（`&self` / `&mut self`，第一个参数是 `&impl SqlExecutor`；`&Db` 与 `&Transaction` 通用，事务内语义与事务外一致）：

| 方法                                                                    | 说明                     |
| --------------------------------------------------------------------- | ---------------------- |
| `insert(&mut self, ex) -> Result<()>`                                 | 纯 INSERT；自增主键占位（`Null` / `0`）时由数据库生成并回填 |
| `insert_or_update(&mut self, ex) -> Result<()>`                       | 主键占位则 INSERT，否则 UPDATE（回填与 Hook 改写同步写回 `self`） |
| `update(&self, ex) -> Result<()>`                                     | 整行 UPDATE（全部非主键字段）     |
| `delete(&self, ex) -> Result<u64>`                                    | 按主键删除；软删模型改写为标记删除       |
| `Model::insert_many(rows: &[Self], ex) -> Result<()>`                  | 同构批量插入（**默认实现**，不触发**同步** Hook、逐行派发**异步** Hook、不回填自增主键，分块 + 单事务） |
| `upsert(&mut self, ex, conflict_fields, update_fields) -> Result<()>` | 冲突时更新（**默认实现**）         |

> 旧文档的 `save` / `update_columns` / `insert_batch` 已分别被 `insert`（或 `insert_or_update`）、
> 变更集（`T::of(pk).set(...).update(&ex)`）、`insert_many` 取代。

## Builder 体系 （[builder.rs](../../../alun-db/src/builder.rs)）

入口为 `Db::query()` / `Db::page(page, size)` / `Db::count()`；Builder 接受运行时表名与列名，内部做标识符白名单校验（防注入）。
**执行单元由 `&Db` 泛化为 `&impl SqlExecutor`** —— 事务内的 `&Transaction` 同样可用。

| `FindBuilder`                                                    | 说明                             |
| ---------------------------------------------------------------- | ------------------------------ |
| `new()`                                                          | 创建                             |
| `table(name)` / `filter(Expr)`                                   | 表名 / WHERE                     |
| `join(kind, table, on)`                                          | 连接（`table` 为 `&'static str`）   |
| `order_by(column, Order)` / `page_after(sort, pk, dir, after)`     | 排序 / 游标分页                       |
| `limit(n)` / `offset(n)`                                         | 分页                             |
| `exec(&ex)` / `first(&ex)`                                       | 执行 → `Vec<Row>` / `Option<Row>` |

| `PageBuilder`                                                     | 说明                          |
| ----------------------------------------------------------------- | --------------------------- |
| `new(page, size)`                                                 | 创建（`page.max(1)`/`size.max(1)`） |
| `table(name)` / `filter(Expr)` / `order_by(col, Order)` / `page_after(...)` | 同 FindBuilder       |
| `exec(&ex)`                                                       | 执行 → `Page<Row>`（自动 COUNT）  |

| `CountBuilder`                       | 说明          |
| ------------------------------------ | ----------- |
| `new()` / `table(name)` / `filter(Expr)` | 创建、表名、WHERE |
| `exec(&ex)`                          | 返回 `i64`    |

## `sql!` 宏与派生宏 （`alun-db-macro`，经 `alun_db::prelude` 导出）

| 宏 / 派生                          | 说明                                                                       |
| -------------------------------- | ------------------------------------------------------------------------ |
| `sql!`                           | 编译期拼接 SQL 片段并生成 `SqlQuery`（SQL 文本 + 绑定参数），配合 `SqlQueryExt` 的终态动词执行 |
| `#[derive(Model)]`               | 生成 `Model` 实现、字段表达式访问器、Hook 注册；属性（**除 `#[view]` 外全部可选**）：`#[table]`（缺省由结构体名蛇形推导）/`#[view]`/`#[hook]`/`#[key]`（缺省回退到 `id` 字段）/`#[auto]`（缺省时 `id` 整型主键自增）/`#[unique]`/`#[column]`/`#[json]`/`#[soft_delete]`（缺省时 `Option` 的 `deleted_at` 自动生效）/`#[model(strict)]`/`#[model(hard_delete)]`/`#[model(no_timestamps)]`/`#[model(schema = "path")]` |
| `#[derive(Row)]`                 | 为结构体生成与 `alun_db_core::row::Row` 的双向转换（常配合 serde 用于 JSON 字段嵌套）         |
| `#[derive(TypeMapper)]`          | 为枚举生成 `TypeMapper`；属性：`#[int]`、`#[value_map("A" => VariantA)]`（首个标识符写 `serde` 可额外生成与落库同编码的 serde 实现）、`#[native_enum("type")]`  |

`#[derive(Model)]` 的 A 批默认约定：表名由结构体名蛇形推导（`SysUser` → `sys_user`、`HTTPLog` → `http_log`）、
未标 `#[key]` 时名为 `id` 的字段即主键、名为 `id` 且为整型的主键默认自增、
含 `created_at` / `updated_at` 字段时自动挂载 `TimestampHook`（**强制覆盖**这两列，排在 Hook 链最前；
业务时间列用 `#[model(no_timestamps)]` 关闭）、
`Option<...>` 的 `deleted_at` 字段自动作为软删标记（非 `Option` 的 `deleted_at` 会编译报错）。

`sql!` 的安全模型：插值 `{ expr }` 一律转成 `?` 占位符 + 绑定参数，值永不内联；
`#where`/`#and`/`#or`/`#orderBy`/`#groupBy`/`#limit` 指令写在字符串字面量内并在编译期白名单校验；
新增 `#if(cond) ... #else ... #end`（条件块，可嵌套，`cond` 是 Rust 布尔表达式）与
`#para(name)`（把作用域内变量绑定为占位参数，字面量内的插值入口）。

```rust,no_run
use alun_db::prelude::*;

# async fn demo(email: String, tid: i64, status_filter: bool, status_value: i32) -> Result<()> {
// 插值 → 参数化
let users: Vec<Row> = sql!("SELECT * FROM users WHERE email = ", { email })
    .find(db())
    .await?;

// 条件写在字面量内部：#if 控制片段、#para 绑定参数
let rows = sql!(
    "SELECT * FROM invoice WHERE tenant_id = ", { tid },
    " #if(status_filter) AND status = #para(status_value) #end",
)
.find(db())
.await?;

// 嵌套组合：子查询的 SQL 与参数成对合并
let ids = sql!("SELECT user_id FROM orders WHERE amount > ", { 100.0f64 });
let rows = sql!("SELECT * FROM users WHERE id IN (", ids, ")")
    .find(db())
    .await?;
# let _ = (users, rows);
# Ok(())
# }
```

> `#if(cond)` 的 `cond` 是 Rust 布尔表达式（token 级 `if cond { ... }` 片段是更常用的写法）；
> `#para(name)` 把作用域内变量绑成占位参数，是字符串字面量内部唯一的插值入口。

> 命名冲突提示：`#[derive(Row)]`（宏）与 `alun_db_core::row::Row`（类型）同名，`alun_db::prelude::*` 同时导出二者；`use alun_db::prelude::*` 后写 `Row::new()` 用的是类型，`#[derive(Row)]` 用的是宏。

`SqlExecutor` / `SqlQueryExt` 负责执行：`Db` 与 `Transaction` 均实现 `SqlExecutor`；
`SqlQueryExt` 的终态动词为 `find(&e)`、`find_first(&e)`、`find_one(&e)`、`find_one_or_null(&e)`、
`exists(&e)`、`paginate(&e, page, size)`、`for_each(&e, f)`、`for_each_page(&e, size, f)`、
`scalar::<T, _>(&e)`、`execute(&e)`。

## 错误类型 （[error.rs](../../../alun-db-core/src/error.rs)）

```rust,ignore
pub type Result<T> = std::result::Result<T, DbError>;   // 旧文档中的 DbResult 即此别名
```

`DbError` 变体：`Connection`、`Query { sql, source }`、`NotFound`、`Readonly`、`UniqueViolation { column }`、`ForeignKeyViolation { constraint }`、`CheckViolation { constraint }`、`ColumnNotFound`、`TypeMismatch { expected, actual }`、`UnexpectedNull`、`UnsupportedFeature { feature, dialect }`、`Transaction`、`InvalidValue`、`Migration`。

> `DbError` **不属于** `alun-core`：两者互不依赖（这是硬约束）。但 handler 的
> `ResResult<T>` 照样能直接 `?` 收下它——`ApiError` 住在 `alun-web` 是本地类型，
> `From<DbError> for ApiError` 合法，映射时给出业务语义的状态码（404 / 409 / 500），
> 原文（含 SQL）只进 `internal_detail` 与日志。
> 只有**非 HTTP 场景**（`alun::Result<T>`）收不下 `DbError`，那里请用 `DbResult<T>`。

## 使用示例

```rust,no_run
use alun_db::prelude::*;

# async fn demo() -> Result<()> {
// ── 初始化（应用启动时一次） ──
alun_db::init("sqlite://./dev.db").await?;

// ── 查询 ──
let all: Vec<Row> = db().find("SELECT * FROM users").await?;
let one: Option<Row> = db().find_first("SELECT * FROM users LIMIT 1").await?;
let total: i64 = db().scalar("SELECT COUNT(*) FROM users").await?;

// 参数化查询：统一写 `?`，PostgreSQL 侧自动改写为 `$1`
let adults: Vec<Row> = db()
    .find_with_params("SELECT * FROM users WHERE age > ?", vec![Value::I64(18)])
    .await?;

// ── 动态行写入 ──
let mut row = Row::new().with_entity("users", "id");
row.insert("name", Value::Text("张三".into()));
row.insert("email", Value::Text("zhangsan@example.com".into()));
let inserted: Row = db().insert_row(&row).await?;      // 返回回填后的行
let mut upd = db().find_row_by_pk("users", "id", Value::Text("u1".into())).await?
    .ok_or(DbError::NotFound)?;
upd.insert("age", Value::I64(29));
let affected: u64 = db().update_row(&upd).await?;
let removed: u64 = db().delete_row(&upd).await?;

// ── 分页（Db::paginate + PageResponseExt，见上文分页小节） ──
let page: Page<Row> = db().paginate("SELECT * FROM users ORDER BY id", 1, 20).await?;
let _ = (page.total, page.size, page.pages, page.has_next());

// ── RAII 事务：闭包返回值决定提交或回滚 ──
db().transaction(|tx: Transaction| async move {
    tx.execute_with_params(
        "UPDATE accounts SET balance = balance - 100 WHERE id = ?",
        vec![Value::Text("a1".into())],
    ).await?;
    tx.execute_with_params(
        "UPDATE accounts SET balance = balance + 100 WHERE id = ?",
        vec![Value::Text("a2".into())],
    ).await?;
    Ok::<(), DbError>(())          // Ok → commit；返回 Err 或提前 ? 都会 rollback
}).await?;

let _ = (all, one, total, adults, inserted, affected, removed);
# Ok(())
# }
```

**Model 路径**（derive + `CrudExt`/`PersistExt`/`ModelQuery`/`ChangeSet`）：

```rust,ignore
use alun_db::prelude::*;

#[derive(Debug, Clone, Model)]
#[table("users")]              // `User` 推导出来是 user，表名不同才需要显式覆盖
#[hook(TimestampHook)]         // 有 created_at / updated_at 时本可省略（自动挂载）
struct User {
    id: i64,                   // 未标 #[key] 时，名为 id 的字段即主键；整型主键默认自增
    #[unique]
    email: String,
    name: String,
}

// 主键查询（CrudExt）
let u: Option<User> = User::find_or_none(db(), 1i64).await?;

// 链式查询（ModelQuery，终态动词）
let page: Page<User> = User::query().paginate(db(), 1, 20).await?;
let adults: Vec<User> = User::query().filter(User::id().gt(0i64)).find(db()).await?;

// 实例写入（PersistExt）
let mut u = User { id: 0, email: "a@b.c".into(), name: "张三".into() };
u.insert(db()).await?;                       // 纯 INSERT（自增主键回填）

// 变更集：只写被 set 的列，且不需要先 SELECT
User::of(u.id).set(User::name(), "李四").update(db()).await?;
```


***

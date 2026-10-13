---
id: reference.crates.db-family
title: `alun-db` 家族其余 Crate（core/sql/macro/build/驱动）
summary: `alun-db` 家族其余 Crate（core/sql/macro/build/驱动）——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [db, reference, driver, dialect, derive]
status: active
owner: alun-core-team
source_of_truth: [alun-db-core/src, alun-db-sql/src, alun-db-macro/src, alun-db-build/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-db` 家族其余 Crate（core/sql/macro/build/驱动）

> 本文件是 [`crates.md`](../crates.md) 的 `db-family` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


`alun-db`（[`alun-db`](db.md)）只是门面；真正的类型定义、SQL 渲染与驱动实现分散在下面 7 个 crate。

## 2.14.1 `alun-db-core` — 核心类型与 trait

**路径**: `alun-db-core/src/`

**职责**：定义整套数据库抽象语言——行、模型、Hook、类型映射、表达式/语句 AST、方言契约、驱动契约、分页容器与错误类型。**零异步、零驱动、零生态依赖**（生态类型全部走可选 feature），因此可被门面与驱动共同依赖而不产生循环。

| 模块                     | 内容                                                                              |
| ---------------------- | ------------------------------------------------------------------------------- |
| `row`                  | `Row`（动态行）                                                                      |
| `error`                | `DbError`、`pub type Result<T> = std::result::Result<T, DbError>`                 |
| `traits::model`        | `Model`                                                                         |
| `traits::hook`         | `Hook`（**同步**方法）                                                                |
| `hook`                 | `TimestampHook`（内置时间戳 Hook）、`now_value()`、Hook 洋葱执行器 `run_before_insert` / `run_after_insert` / `run_before_update` / `run_after_update` / `run_before_delete` / `run_after_delete`（本次重构从 `alun-db` 迁入，`alun_db::hook` 已删除） |
| `traits::type_mapper`  | `TypeMapper`（类型 ↔ `stmt::Value`）                                                 |
| `traits::params`       | `Params`（`()` 与 0\~12 元组的参数列表）                                                  |
| `traits::dialect`      | `Dialect`、`DialectCapability`、`Placeholder`、`RenderedSql`、`RenderCtx`、`RawDbError`  |
| `traits::driver`       | `Driver`、core 版 `PoolConfig`                                                     |
| `traits::executor`     | `ExecuteResult`、`Rows`                                                           |
| `expr`                 | `Expr`、`BinaryOp`、`UnaryOp`、`Function`                                          |
| `statement`            | `Statement`、`SelectStmt`/`InsertStmt`/`UpdateStmt`/`DeleteStmt`/`RawStmt`、`OrderBy`、`Join`、`OnConflict` 等 |
| `field`                | `FieldExpr<T>`、`FieldValue<T>`、`FieldMeta`                                       |
| `page`                 | `Page<T>`（`list/total/page/size/pages`）、`Cursor`                                 |
| `query`                | `SqlQuery`、`SqlFragment`、`IntoValue`                                             |
| `stmt::{ty,value,compact}` | `Type`、`Value` 与紧凑时间/十进制等载体类型                                                  |

- `Driver` trait：`name()`、`dialect()`、`type Pool`、`connect(url, PoolConfig)`——三个驱动各自实现。
- 可选 feature：`chrono`、`time`、`jiff`、`rust_decimal`、`bigdecimal`、`uuid`、`ulid`、`json`、`ipnetwork`（分别启用对应 `TypeMapper` 实现与类型再导出）；默认关闭。
- 依赖：仅 `thiserror`。

## 2.14.2 `alun-db-sql` — SQL 渲染器与方言

**路径**: `alun-db-sql/src/`

**职责**：把 core 的 `Expr`/`Statement` AST 渲染为具体 SQL 文本 + 绑定参数；集中处理三种方言差异，不让差异泄漏回 core。

| 导出                                        | 说明                                                                     |
| ----------------------------------------- | ---------------------------------------------------------------------- |
| `SqliteDialect` / `PostgresDialect` / `MysqlDialect` | 三个 `Dialect` 实现（crate 根导出，`dialects::{sqlite,postgres,mysql}` 亦可） |
| `rewrite_placeholders(sql, dialect)`      | 把 `?` 占位符按方言改写为 `$1`/`$2`…（`?` 方言原样返回；字符串字面量内的 `?` 不改写）               |
| `render::render_identifier(name, QuoteStyle)` | 渲染并校验标识符（仅允许 `[A-Za-z_][A-Za-z0-9_]*`，防注入）                             |
| `render::validate_identifier(name)`       | 仅校验                                                                |
| `render::QuoteStyle`                      | `Double`（PG/SQLite）/ `Backtick`（MySQL）/ `None`                        |

内部按语句类型分文件渲染：`render::{select, insert, update, delete, expr, statement, json, pagination, placeholder}`。

- feature：`default = ["sqlite", "postgres", "mysql"]`；依赖 `alun-db-core`。

## 2.14.3 `alun-db-macro` — 过程宏

**路径**: `alun-db-macro/src/`

| 宏                                                   | 展开结果                                                            |
| --------------------------------------------------- | --------------------------------------------------------------- |
| `#[derive(Model)]`                                  | `Model` 实现 + 字段表达式访问器 + Hook 注册（属性：`table`（可选，缺省由结构体名推导）/`view`/`hook`/`key`（可选，缺省回退 `id`）/`auto`（可选，缺省时 `id` 整型主键自增）/`unique`/`column`/`json`/`soft_delete`（可选，缺省时 `Option` 的 `deleted_at` 自动生效）/`model`（`strict` / `hard_delete` / `no_timestamps` / `schema = "path"`）） |
| `#[derive(Row)]`                                    | 结构体 ↔ `alun_db_core::row::Row` 双向转换（常配合 serde 用于 JSON 字段）          |
| `#[derive(TypeMapper)]`                             | 枚举的 `TypeMapper`（属性：`int`、`value_map`（可加 `serde` 标记）、`native_enum`）             |
| `sql!`（函数式宏）                                       | 展开为构造 `alun_db_core::query::SqlQuery` 的表达式（编译期校验片段与指令）             |

生成的代码全部使用 `::alun_db_core::`/`::std::` 绝对路径，字段元数据以 `static` 数组生成（零运行时开销）。

## 2.14.4 `alun-db-build` — `build.rs` 助手

**路径**: `alun-db-build/src/lib.rs`

**职责**：在 `build.rs` 中解析迁移目录里的 `*.sql`，生成供编译期 schema 校验使用的快照。

```rust,ignore
pub fn build_snapshot(migrations_dir: &str) -> std::io::Result<()>;
```

行为：读取 `CARGO_MANIFEST_DIR/migrations_dir`（不存在报错）→ 按文件名排序解析 `CREATE TABLE` / `ALTER TABLE ADD COLUMN` → 渲染快照写入 `$OUT_DIR/schema_snapshot.sql` → 通过 `cargo:rustc-env=ALUN_DB_SCHEMA_SNAPSHOT=<path>` 暴露给编译期（配合 `#[model(strict)]` 做 schema 校验）。同时为目录与每个 SQL 文件输出 `cargo:rerun-if-changed`。**无外部依赖**（只用 `std`）。

## 2.14.5 `alun-db-driver-{sqlite,postgres,mysql}` — 驱动

**职责**：实现 `alun_db_core::traits::Driver`，提供连接池与连接/事务句柄；懒加载/占位符改写/自增主键等方言细节在此收口，异步包装（`spawn_blocking` 等）由门面统一处理。

三个驱动的 crate 根导出结构一致：

| 导出                        | SQLite                          | PostgreSQL                       | MySQL                          |
| ------------------------- | ------------------------------- | -------------------------------- | ------------------------------ |
| `*Driver`                 | `SqliteDriver`                  | `PostgresDriver`                 | `MysqlDriver`                  |
| 连接池                       | `SqlitePool`                    | `PostgresPool`                   | `MysqlPool`                    |
| 池化连接                      | `PooledSqliteConnection`        | `PooledPgConnection`             | `PooledMysqlConnection`        |
| 事务句柄                      | `SqlitePoolTransaction`         | `PostgresPoolTransaction`        | `MysqlPoolTransaction`         |
| 连接                        | `SqliteConnection`              | `PostgresConnection`             | `MysqlConnection`              |
| 方言再导出                     | `SqliteDialect`                 | `PostgresDialect`                | `MysqlDialect`                 |

| 驱动      | 底层库               | 要点                                                                                                                                        |
| ------- | ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| sqlite  | `rusqlite 0.40.2` | 同步核心（`execute_sync`/`query_sync`），池化 `get()` 异步（信号量等待）；`#![deny(missing_docs)]`；feature `default = ["async"]`（`async = ["dep:tokio"]`） |
| postgres | `tokio-postgres 0.7.18` | 不启用 chrono/uuid/serde_json 集成，`stmt::Value` ↔ PG 二进制格式由手写 `convert` 模块完成；Raw SQL 统一以 `?` 书写，执行前经 `rewrite_placeholders` 改写为 `$n`；未终结事务在 Drop 时丢弃连接（服务端断连即回滚） |
| mysql   | `mysql_async 0.37.1`（`default-features = false, features = ["minimal"]`） | 裸用 `Conn`，池化由自研 `pool` 模块完成；`?` 占位符与 MySQL 原生一致，无需改写；无 `RETURNING`，自增主键取 `last_insert_id`；未终结事务同样 Drop 丢弃连接 |

> 以上三个 crate 均依赖 `alun-db-core` + `alun-db-sql`，不依赖门面 `alun-db`；门面通过 `factory`（`pub(crate)`）按 `DatabaseConfig.db_type` 选择驱动。

***

# Database (`alun-db`)

Requires `alun` 的 `features = ["db"]`（默认的 `full` 已含；`db` 自带 SQLite 驱动）。`Db` is a unified facade over PostgreSQL (`$N` placeholders), MySQL (`?`), and SQLite (`?`).

`db()` / `try_db()` / `set_db()` / `Row` / `Page` / `PageResponseExt`、`Model` 派生宏、`CrudExt` / `PersistExt` / `ModelQuery` / `ChangeSet` / `SqlQueryExt` / `RowBatchExt`、`Hook` / `TimestampHook`、`sql!`、`Value` / `Order` / `Cursor` / `DbError`、`FindBuilder` / `PageBuilder` / `CountBuilder`、`SqlQuery` / `SqlFragment`、`Expr` / `Statement` / `FieldExpr`、`Db` / `Transaction` / `Migration` / `migrate` / `DatabaseConfig` 等**全部**由 `alun::prelude::*` 提供：`alun::prelude` 完整转发 `alun_db::prelude::*`，后者又转发 `alun_db_core::prelude::*`（`Hook` 执行器与 `TimestampHook` 定义在 `alun-db-core`）。因此普通项目**只需在 `Cargo.toml` 声明 `alun`**，无需声明 `alun-db` 或 `alun-db-core`：

```toml
[dependencies]
# 驱动 / 生态类型 / xss 都在 alun 上选（逐个转发到 alun-db）
alun = { version = "0.3", features = [
    "postgres",                # 驱动必须显式选：sqlite / postgres / mysql
    "chrono", "rust_decimal",  # 生态类型（DateTime / Decimal / Uuid…）
    "json",                    # #[json] 字段与 serde_json 转发
    "xss",                     # 可选：HTML 净化（xss::sanitize_html）
] }
```

> 例外：`alun_db::init` / `alun_db::init_with` / `alun_db::db()` / `alun_db::try_db()` 操作的是 `alun-db` 自带的**另一个全局实例**，与本框架 `alun::db()` 指向的实例不是同一个；框架内一律用 `alun::db()`，手动挂载用 `set_db(Db::connect(url).await?)`。

## 依赖与 Feature

**驱动按需开启（0.3.0 起 `db` 不再隐含任何驱动）。** `db` 只开「数据库能力」（含 `config`），**不含驱动**；必须再显式选一个驱动 feature。`sqlite` / `postgres` / `mysql` 各自隐含 `db`，所以通常只写驱动名即可：

| feature | 效果 |
|---|---|
| `db` | 仅数据库门面 + 异步运行时（**无驱动**，建池会报 `UnsupportedFeature`） |
| `sqlite` | 隐含 `db` + SQLite 驱动（最省事的默认选择） |
| `postgres` | 隐含 `db` + PostgreSQL 驱动 |
| `mysql` | 隐含 `db` + MySQL 驱动 |

```toml
# ✅ 推荐：显式写驱动名，它已隐含 db
alun = { version = "0.3", features = ["sqlite"] }

# 也可以分层写（等价）
alun = { version = "0.3", features = ["db", "sqlite"] }
```

只开 `db` 却把 `[database] type` 设成 `postgres` / `mysql`（或任何未开的驱动），建池时报 `DbError::UnsupportedFeature { feature: "driver", dialect: "postgres" }`（驱动挂在 `#[cfg(feature = "...")]` 上，见 `alun-db/src/factory.rs` 的 `create_driver`）。生态类型 feature 缺失则是**编译期**错误（字段类型不满足 `TypeMapper`）。

**生态类型经 prelude 直接引用**，无需自己声明对应 crate（只需在 `alun` 上开同名 feature）：`alun::prelude::chrono::DateTime` / `Utc`、`alun::prelude::rust_decimal::Decimal`、`alun::prelude::uuid::Uuid`、`alun::prelude::ulid::Ulid`、`alun::prelude::bigdecimal::BigDecimal`、`alun::prelude::serde_json::{json, Value}`，以及 `alun::prelude::{time, jiff, ipnetwork}` 模块。

```rust,ignore
use alun::prelude::*;

let now: chrono::DateTime<chrono::Utc> = chrono::Utc::now();     // feature "chrono"
let id: uuid::Uuid = uuid::Uuid::now_v7();                       // feature "uuid"
let amount: rust_decimal::Decimal = "1.23".parse().unwrap();     // feature "rust_decimal"
let payload: serde_json::Value = serde_json::json!({ "k": 1 });  // feature "json"
```

`#[json]` 字段的生成代码按调用方 `Cargo.toml` 里**实际声明**的 crate 解析路径：只声明 `alun` 时用 `::alun::__serde_json`（门面转发），因此**无需**为 `#[json]` 自己声明 `serde_json`；只需在 `alun` 上开 `json` feature。同理，派生宏 / `sql!` 生成代码里的内核路径解析为 `::alun::__db_core`（只声明 `alun` 时）或 `::alun_db::__db_core`（声明 `alun-db` 时）。

`#[model(strict)]` 是**构建期** schema 校验，需应用 crate 在 `[build-dependencies]` 里声明 `alun-db-build`（构建期工具，门面不转发），并在 `build.rs` 调用 `alun_db_build::build_snapshot("migrations")`。

**Decimal 的落库类型随方言不同**（`render_type` 的 DDL 渲染映射，逻辑值与 Rust 侧类型一致）：

| 方言 | `Type::Decimal` | 依据 |
|---|---|---|
| SQLite | `TEXT`（十进制文本往返） | `alun-db-sql/src/dialects/sqlite.rs:58` |
| MySQL | `DECIMAL(38, 10)` | `alun-db-sql/src/dialects/mysql.rs:62` |
| PostgreSQL | `NUMERIC` | `alun-db-sql/src/dialects/postgres.rs:61` |

## Global Access

```rust,ignore
db()                                         // &'static Db — panics if DB not initialized
try_db()                                     // Option<&'static Db> — safe accessor
```

框架内手动挂载（不使用 `App::start()` 时）：`set_db(Db::connect(url).await?)`，或用配置版 `set_db(Db::connect_with(url, config).await?)`（`Db` / `set_db` 均由 prelude 导出）。两个 `Db` 构造函数在建池后立即执行一次 `SELECT 1` 连接测试，使认证失败 / 库不可达在启动期暴露。**不要**用 `alun_db::init` / `alun_db::init_with`——它们写入的是 `alun-db` 自带的全局实例，`alun::db()` 读不到。

## Row 动态 CRUD

`Row` 是 alun 的动态数据载体：一个「表名 + 主键列 + 有序字段表」，**没有变更追踪**（写回时整行提交，按需只改想改的字段）。

```rust,no_run
use alun::prelude::*;
use std::result::Result;

# async fn demo() -> Result<(), Box<dyn std::error::Error>> {
// 插入：Row 携带「表名 + 主键列」元数据
let mut row = Row::new().with_entity("users", "id");
row.insert("id", Sid::uuid7());               // UUID v7 主键（推荐）
row.insert("name", "张三");
row.insert("email", "zhangsan@example.com");
row.insert("age", 28);
let inserted: Row = db().insert_row(&row).await?;   // 返回写入后的行

// 按主键查询（主键值从返回行回读）
let user: Option<Row> = match inserted.pk_value() {
    Some(pk) => db().find_row_by_pk("users", "id", pk.clone()).await?,
    None => None,
};

// 更新 / 删除（整行写回）
if let Some(mut row) = user {
    row.insert("age", 29);
    let updated: u64 = db().update_row(&row).await?;
    let deleted: u64 = db().delete_row(&row).await?;
    # let _ = (updated, deleted);
}

// 原始 SQL：查询 / 单条 / 标量 / 执行
let users: Vec<Row> = db().find("SELECT * FROM users WHERE age > 18").await?;
let one: Option<Row> = db().find_first("SELECT * FROM users ORDER BY id LIMIT 1").await?;
let total: i64 = db().scalar("SELECT COUNT(*) FROM users").await?;
let affected: u64 = db().execute("UPDATE users SET active = 1 WHERE age > 18").await?;
# let _ = (users, one, total, affected);
# Ok(())
# }
```

带占位符参数的 SQL 用 `db().find_with_params(sql, Vec<Value>)` / `db().execute_with_params(sql, Vec<Value>)` / `db().scalar_with_params::<T>(sql, params)`（`Value` 由 `alun::prelude` 导出），或直接用 `sql!` 宏让插值自动参数化（见下）。

异构批量写由 `RowBatchExt` 提供（`Db` 与 `Transaction` 都实现，prelude 导出）：各行的字段集可以不同，列集相同者合并为多行 `VALUES`，多条语句包在一个事务里（不触发任何 Model Hook）：

```rust,ignore
use alun::prelude::*;

let inserted: u64 = db().batch_insert("users", &rows).await?;   // rows: &[Row]，各行列集可不同
let updated: u64 = db().batch_update("users", &rows).await?;    // 每行按自身主键定位，只更新携带的非主键字段
```

> `PersistExt::insert_many` 是另一条路：强类型、同构（同一模型），适合大批量等结构数据。

`Db` 的错误类型是 `DbError`（定义在 `alun-db-core` 的 `error` 模块，经 `alun::prelude` 导出，别名 `DbResult<T>`）。

| 上层返回 | 能 `?` 一个 `DbResult` 吗 | 说明 |
| -------- | ------------------------- | ---- |
| `ResResult<T>`（handler，**唯一出口**） | ✅ `From<DbError> for ApiError` 内置 | `Ok(Res::ok(repo.load(id).await?))` |
| `DbResult<T>`（数据访问层） | ✅ 同类 | — |
| `alun::Result<T>`（启动 / 插件 / 定时任务） | ❌ 非 HTTP 场景 | 用 `DbResult<T>` 或显式转换 |

按孤儿规则，`From<DbError> for X` 只能写在拥有 `DbError`（`alun-db-core`）或拥有 `X`
的 crate 里。`ApiError` 住在 `alun-web` 是那里的**本地类型**，所以这条 `From` 天然合法
——既不必让 `alun-core` 依赖数据库内核，也不必再新造一个中间错误类型。

需要自定义对外文案时显式分支即可：
`repo.load(id).await.map_err(|e| match e { DbError::NotFound => ApiError::not_found("用户不存在"), .. })?`。
默认映射表（`alun-web/src/api.rs` 的 `db_error_to_api_error`，按孤儿规则合法）：

| `DbError` 变体 | HTTP | 对外文案 |
| ---- | ---- | ---- |
| `NotFound` | **404** | 记录不存在 |
| `UniqueViolation` | **409** | 数据重复：唯一约束冲突 |
| `ForeignKeyViolation` / `CheckViolation` | **409** | 数据约束冲突 |
| `Deadlock` / `SerializationFailure` / `LockTimeout` | 500 | 服务繁忙，请重试 |
| 其余（连接/查询/类型/迁移…） | 500 | 模糊文案，细节写进 `internal_detail`（仅日志，不出响应体） |

> 完整迁移指南与更多踩坑见仓库 `docs/guides/db/ops.md` §5.10（本表为速查，省略分支细节）。

## Row 字段访问

```rust,no_run
use alun::prelude::*;

# fn demo(row: &Row) {
let a = row.get("name");                  // Option<&Value>
let b = row.get_typed::<String>("name");  // Result<String>
let c = row.get_typed::<i64>("age");      // Result<i64>
let d = row.pk_value();                   // Option<&Value> — 主键值
let e = row.contains("field");            // bool — 字段是否存在
let f = row.table();                      // Option<&str> — 表名（未设置时为 None）
let g = row.primary_key();                // &str — 主键列名（未设置时默认 "id"）
let h = row.len();                        // usize — 字段数量
# let _ = (a, b, c, d, e, f, g, h);
# }
```

自定义主键：`Row::new().with_entity("products", "code")`，或事后 `row.set_primary_key("code")` / `row.set_table("products")`。**不支持组合主键**（`Row` 只携带一个主键列）。

## Model 与 CrudExt / PersistExt

强类型模型用 `#[derive(Model)]` 声明；查询是**关联函数**，持久化是**实例方法**（两者都由 `alun-db` 对 `T: Model` 的 blanket impl 提供，derive 本身不生成这些方法）。`#[derive(Model)]` 带一组**可覆盖的默认约定**：表名由结构体名蛇形推导、名为 `id` 的字段即主键、`id` 整型主键默认自增、含 `created_at` / `updated_at` 字段时自动挂载 `TimestampHook`、`Option<...>` 的 `deleted_at` 自动作为软删标记（属性清单见本节末）。

```rust,ignore
use alun::prelude::*;

/// `SysUser` 表对应的强类型模型（表名蛇形推导为 `sys_user`）。
#[derive(Model, Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SysUser {
    pub id: i64,                                    // 名为 `id` 的字段默认主键；整型主键默认自增
    #[unique]
    pub email: String,
    pub name: String,
    pub status: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,  // 含 created_at/updated_at → 自动挂载 TimestampHook
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,   // Option<...> 的 deleted_at → 自动软删
}

// ==================== 静态查询糖（CrudExt，首参 &impl SqlExecutor）====================
let user: SysUser = SysUser::find(&db, 1).await?;                        // 按主键查单条；不存在 → DbError::NotFound
let user: Option<SysUser> = SysUser::find_or_none(&db, 1).await?;        // 按主键查单条，可空
let users: Vec<SysUser> = SysUser::query().find(&db).await?;             // 全表（自动排除软删行）
let users = SysUser::find_by(&db, "status", (1,)).await?;                // 字段名 + 值 → status = ?
let users = SysUser::find_by(&db, "status = ? ORDER BY id DESC", (1,)).await?;   // WHERE 片段
let user = SysUser::find_first_by(&db, "email", ("a@b.com",)).await?;
let users = SysUser::find_in(&db, SysUser::status(), [1, 2]).await?;     // 单列 IN（FieldExpr 键，列名编译期校验）
let users = SysUser::find_in_ids(&db, [1, 2]).await?;                    // 主键 IN
let page: Page<SysUser> = SysUser::query().paginate(&db, 1, 20).await?;  // Page: list/total/page/size/pages
let n: i64 = SysUser::count(&db).await?;
let n: i64 = SysUser::count_by(&db, "status", (0,)).await?;

// ==================== 持久化（实例方法）====================
let mut user = SysUser {
    id: 0, email: "a@b.com".into(), name: "张三".into(), status: 1,
    created_at: Default::default(), updated_at: Default::default(), deleted_at: None,
};
user.insert(&db).await?;                                  // 纯 INSERT，自增主键回填到 user.id
user.insert_or_update(&db).await?;                        // 主键占位 → INSERT，否则整行 UPDATE
user.name = "李四".into();
user.update(&db).await?;                                  // UPDATE 全部字段
SysUser::of(user.id).set(SysUser::name(), "李四").update(&db).await?;   // 只 UPDATE 被 set 的列
user.delete(&db).await?;                                  // 软删模型自动改写为标记删除

SysUser::delete_by_pk(&db, 1).await?;                     // 按主键删除，无需先查出来
SysUser::delete_by(&db, "status", (0,)).await?;
SysUser::delete_in_ids(&db, [1, 2]).await?;
SysUser::insert_many(&rows, &db).await?;                  // 同构批量 INSERT（不触发同步 Hook、逐行派发异步 Hook、不回填自增主键）
user.upsert(&db, &["email"], &["name"]).await?;           // ON CONFLICT(email) DO UPDATE SET name
```

**`where_or_field` 双态签名**：`find_by` / `find_first_by` / `count_by` / `delete_by` 的第一个参数既是**字段名**也是 **WHERE 片段**——**只有一个参数且该参数是合法标识符**时按「`字段 = ?`」处理，否则整段按 WHERE 片段处理（片段**直接拼进 `WHERE` 之后**，可带 `ORDER BY`，占位符自己写）。字段名走标识符白名单校验；需要方言引号安全时改用类型化 Builder（`filter(SysUser::status().eq(1))`）。参数来自 `Params`：`()` 与 0~12 元组，元素需 `Into<Value>`。

**`CrudExt` / `PersistExt` 的首参是 `&impl SqlExecutor`：`&Db`（连接池）与 `&Transaction`（事务）通用**，
因此 **Model API 在事务内同样可用**，无需退回 raw SQL：

```rust,ignore
db().transaction(|tx| async move {
    let inv = Invoice::find(&tx, id).await?;                  // 事务内读
    Invoice::of(id).set(Invoice::status(), Status::Paid)
        .update(&tx).await?;                                  // 事务内部分更新（Hook 只收「主键 + 变更列」）
    InvoiceItem::insert_many(&items, &tx).await?;             // 事务内批量插入（复用外层事务）
    audit_record(&tx).await?;                                 // 审计与业务在同一事务
    Ok(())
}).await?;
```

事务内语义与事务外一致：自增主键回填、Hook 触发、分块批量的原子性都成立；
`&Transaction` 不会另开嵌套事务（框架不支持嵌套，见缺口 G4）。

### `ModelQuery<T>` —— 强类型查询脊柱

两条构造入口产出同一种查询对象，**终态动词一致**：

- `User::query()` —— 表名 + 链式条件，**自动追加软删除过滤**（模型声明了软删标记列时）；
- `User::sql(sql!(...))` —— 原始 SQL 直通，**不做软删过滤**（`WHERE` 由调用方书写），链式修饰不可用。

```rust,ignore
// ① 表名 + 链式条件
let page: Page<SysUser> = SysUser::query()
    .filter(SysUser::status().eq(1))                   // 多次 filter 之间为 AND
    .order_by(SysUser::created_at().desc())
    .limit(20)
    .paginate(&db, 1, 20).await?;                      // Page<SysUser>

// ② 原始 SQL（链式修饰不可用）
let users: Vec<SysUser> = SysUser::sql(sql!("SELECT * FROM sys_user WHERE tenant_id = ", { tid }))
    .find(&db).await?;
```

链式修饰（**仅 ① 可用**）：`filter(Expr)`、`order_by(OrderBy)`、`limit(u64)`、`offset(u64)`、
`page_after(sort_col, pk_col, Order, Option<Cursor>)`、`filter_fragment(Expr)`（末位 WHERE 片段，不加括号，可自带 `ORDER BY`/`LIMIT`，`where_or_field` 片段形态即由它实现）。② 形态调用它们会在终态返回 `DbError::InvalidValue`。

| 终态动词 | 返回 | 说明 |
|---|---|---|
| `find(&db)` | `Vec<T>` | 全部结果 |
| `find_first(&db)` | `Option<T>` | 首条（无匹配 `None`，追加 `LIMIT 1`） |
| `find_one(&db)` | `T` | **恰好一行**：0 行 → `DbError::NotFound`，多于一行 → `DbError::InvalidValue`（`LIMIT 2` 探测，不需全量取回） |
| `find_one_or_null(&db)` | `Option<T>` | 0 或 1 行；多于一行 → `DbError::InvalidValue` |
| `exists(&db)` | `bool` | 是否存在匹配行 |
| `paginate(&db, page, size)` | `Page<T>` | 页码从 1 开始 |
| `for_each(&db, \|t\| Ok(true))` | `()` | 结果**一次性取回**后遍历（协议无游标）；回调返回 `Ok(false)` 提前结束 |
| `for_each_page(&db, size, \|page\| Ok(true))` | `()` | 真分页遍历（`Page.total/pages` 为 0，`page` 为当前页号），内存受 `size` 约束 |
| `scalar::<V, _>(&db)` | `V` | 取首行首列单值 |
| `execute(&db)` | `u64` | 执行写语句，返回受影响行数 |

**没有** `select(...)` 列投影 API：投影结果无法构造 `T`，需要投影时直接用
`sql!("SELECT id, email FROM sys_user ...").find(&db)` 拿动态 `Row`。

### `ChangeSet<T>` —— 变更集

```rust,ignore
SysUser::of(123)                                  // 只带主键，无需先 SELECT
    .set(SysUser::name(), "james")                // 键是 FieldExpr：字段名 + 值类型编译期校验
    .set(SysUser::status(), 2)
    .update(&db).await?;                          // 只 UPDATE 被 set 的列

SysUser::of(123).delete(&db).await?;              // 免 SELECT 删除（软删模型自动改写）
```

- SET 目标**只含被 `set` 过的列**，未 set 的列保持数据库原值；同一列多次 `set` 保留最后一次；
- 传给 Hook 的行**只含主键 + 本次变更列**（`Row::keys()` 即变更集合）；
- `set` 主键 → `DbError::InvalidValue`；未 `set` 任何列 → `DbError::InvalidValue`；
  字段不属于该模型（`field.table() != T::TABLE`）→ `DbError::InvalidValue`。

### 方法速查（签名以 `alun-db` 源码为准）

| trait | 方法 |
|---|---|
| `CrudExt`（关联函数，首参 `&impl SqlExecutor`） | `of(pk) -> ChangeSet<Self>`、`query() -> ModelQuery<Self>`、`sql(SqlQuery) -> ModelQuery<Self>`、`find(&ex, pk)`、`find_or_none(&ex, pk)`、`find_by(&ex, where_or_field, params)`、`find_first_by(&ex, where_or_field, params)`、`find_in(&ex, FieldExpr<V>, values)`、`find_in_ids(&ex, ids)`、`count(&ex)`、`count_by(&ex, where_or_field, params)`、`delete_by_pk(&ex, pk)`、`delete_by(&ex, where_or_field, params)`、`delete_in(&ex, FieldExpr<V>, values)`、`delete_in_ids(&ex, ids)` |
| `PersistExt`（实例方法） | `insert(&mut self, &ex)`、`insert_or_update(&mut self, &ex)`、`update(&self, &ex)`、`delete(&self, &ex) -> u64`、`insert_many(&[Self], &ex)`、`upsert(&mut self, &ex, conflict_fields, update_fields)` |
| `ModelQuery<T>` | 链式修饰 + 上表终态动词 |
| `ChangeSet<T>` | `set(FieldExpr<F>, V)`、`update(&ex) -> u64`、`delete(&ex) -> u64` |
| `SqlQueryExt`（`sql!` 产物） | `find` / `find_first` / `find_one` / `find_one_or_null` / `exists` / `paginate` / `for_each` / `for_each_page` / `scalar::<V, _>` / `execute` —— 签名与 `ModelQuery` 终态一致，只是返回 `Row` 而非 `T` |
| `RowBatchExt`（`Db` 与 `Transaction` 都实现，prelude 导出） | `batch_insert(&self, table: &str, rows: &[Row]) -> u64`、`batch_update(&self, table: &str, rows: &[Row]) -> u64` —— 异构批量：各行列集可不同，列集相同者合并为多行 `VALUES`，多语句包一个事务 |

- 与整行 `update` 的区别：变更集**不会**自动刷新 `updated_at`（与 Rails `update_columns` 一致）；需要刷新时显式 `.set(User::updated_at(), now)` 或改用整行 `update`。

> 静态按主键删除刻意保留 `delete_by_pk` 之名（未采用 `delete`）：实例侧 `PersistExt::delete(&self, ex)` 已占用 `delete`，同名会在 `User::delete(&db, pk)` 处产生 E0034 歧义；aifei 同样用 `deleteById` 区分。

> `upsert` 后两个参数为 `conflict_fields` / `update_fields`（模型字段名数组，冲突判定列与冲突时待更新列），与已删除的同名旧方法 `update_columns` 无关；定值更新请用 `ChangeSet`。

### `#[derive(Model)]` 属性清单

| 属性 | 作用域 | 说明 |
|------|--------|------|
| `#[table("name")]` | 结构体 | 表名；**缺省由结构体名蛇形推导**（`SysUser` → `sys_user`、`HTTPLog` → `http_log`、`UserV2` → `user_v2`） |
| `#[view("name")]` | 结构体 | 只读视图名（`READONLY = true`） |
| `#[hook(Type)]` | 结构体 | 注册同步 Hook，可重复声明多个（决定链上顺序） |
| `#[hook_async(Type)]` | 结构体 | 注册异步 Hook（`AsyncHook`，**提交后**触发），可重复声明多个 |
| `#[model(strict)]` | 结构体 | 编译期 schema 校验（配合 `alun-db-build` 快照）；也可 `#[model(strict, schema = "path")]` 指定 DDL 文件 |
| `#[model(hard_delete)]` | 结构体 | 关闭 `deleted_at` 自动软删 |
| `#[model(no_timestamps)]` | 结构体 | 关闭 `created_at` / `updated_at` 的 `TimestampHook` 自动挂载（时间戳由调用方维护，适合业务时间列） |
| `#[key]` | 字段 | 主键；**缺省回退到名为 `id` 的字段**（必须有且仅有一个） |
| `#[auto]` | 字段 | 自增；**缺省在名为 `id` 的整型主键上开启**（只能用于主键） |
| `#[unique]` | 字段 | 唯一列 |
| `#[column("name")]` | 字段 | 指定列名 |
| `#[json]` / `#[json(nullable)]` | 字段 | JSON 列：字段类型需满足 `serde_json` 的 `Serialize + Deserialize`（嵌套结构自行 `#[derive(serde::Serialize, serde::Deserialize)]`），写入走 `Value::Json`、读取接受 `Json`/`Text`/`Null`。可空性由字段是否为 `Option<...>` 决定（`nullable` 参数语法上接受，当前不改变生成逻辑） |
| `#[soft_delete]` | 字段 | 软删除列（至多一个）；**缺省时 `Option<...>` 的 `deleted_at` 自动作为软删标记**，非 `Option` 的 `deleted_at` 会编译报错 |

自增语义细化：主键值为**占位值**（`Value::Null` / 数值 `0`）时才排除该列交由数据库生成；
显式非零主键（雪花 ID 等）照常写入。`insert_many` 仅在**所有行**主键都未赋值时排除该列。

## 枚举字段（`#[derive(TypeMapper)]`）

枚举可以直接做 Model 字段，派生 `TypeMapper` 决定它与 `Value` 的互转（`alun-db-macro/src/type_mapper.rs`）：

| 写法 | 落库 / 读回 | 说明 |
|------|-------------|------|
| `#[derive(TypeMapper)]` | `Value::Text` | 默认：变体名即编码，读回只接受文本载荷 |
| `#[int]` | `Value::I32` | 判别值编码；读回经 `int_discriminant` 归一任意整数宽度（驱动读整数列常归一到 `I64`），未知判别值报 `DbError::InvalidValue`，越界/非整数报 `TypeMismatch` |
| `#[value_map("A" => VariantA, …)]` | `Value::Text("A")` | 显式编码表；首个标识符写 `serde`（`#[value_map(serde, …)]`）会**额外**生成 `Serialize`/`Deserialize`，让 JSON 编码与落库编码一致——该模式需应用自己声明 `serde` 依赖 |
| `#[native_enum("type_name")]` | `Value::Enum` | 数据库原生 ENUM 类型；读回同时接受 `Value::Enum` 与文本承载列 |

```rust,no_run
use alun::prelude::*;

/// 状态机字段：落库键与 JSON 键统一为 `draft` / `paid`。
#[derive(Debug, Clone, Copy, PartialEq, TypeMapper)]
#[value_map(serde, "draft" => Draft, "paid" => Paid)]
pub enum InvoiceStatus {
    Draft,
    Paid,
}
```

## Transactions (RAII)

事务是闭包式的：返回 `Ok` 提交，返回 `Err` 回滚。回滚由**驱动事务句柄的 `Drop`** 保证（`Transaction` 本身不实现 `Drop`，因为 `Drop::drop` 不能 `await`）：

```rust,no_run
use alun::prelude::*;
use alun::prelude::DbError;
use std::result::Result;

# async fn demo() -> Result<(), Box<dyn std::error::Error>> {
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

闭包签名是 `FnOnce(Transaction) -> Fut`，`Fut` 输出 `Result<T, DbError>`（**不再**返回 `(ActiveTx, DbResult<T>)` 元组）。

跨函数边界时需要显式句柄：

```rust,ignore
let tx = db().begin().await?;
do_something(&tx).await?;
do_something_else(&tx).await?;
tx.commit().await?;      // 幂等
// 或 tx.rollback().await?;  （同样幂等）
```

> 事务从池中借出**专属连接**：事务内的语句必须走 `tx` 句柄；通过 `db()` 执行的语句走其他连接，不受该事务保护。

## Hooks (CRUD Lifecycle)

`Hook` 是**同步** trait（不是 `async fn`）——它只做行数据改写与校验，不做 IO。注册在 **Model** 上，因此只对 `PersistExt` 的单行 CRUD（`insert` / `insert_or_update` / `update` / `delete`）与 `ChangeSet` 生效；绕过 Model 直接写 SQL（`db().execute*` / `sql!`）或用动态 `Row` API（`insert_row` 等）时 Hook **不触发**，`PersistExt::insert_many` 的批量写入也**不触发同步 Hook**（但会**逐行派发异步 Hook** `after_insert`，并遵循同一套事务边界语义）。

```rust,no_run
use alun::prelude::*;
use alun::prelude::{DbError, Hook, Value};

struct UppercaseNameHook;

impl Hook for UppercaseNameHook {
    // 参数是 &mut Row：可就地修改待写入的行
    fn before_insert(&self, row: &mut Row) -> std::result::Result<(), DbError> {
        let name = row.get_typed::<String>("name").unwrap_or_default();
        row.insert("name", name.to_uppercase());
        Ok(())
    }
    fn before_update(&self, row: &mut Row) -> std::result::Result<(), DbError> {
        let name = row.get_typed::<String>("name").unwrap_or_default();
        row.insert("name", name.to_uppercase());
        Ok(())
    }
    fn after_insert(&self, row: &Row) -> std::result::Result<(), DbError> {
        tracing::info!("审计 - 新增: table={:?}", row.table());
        Ok(())
    }
    // 删除类回调拿到的是主键值（不再是 table + id 两个参数）
    fn before_delete(&self, _pk: &Value) -> std::result::Result<(), DbError> {
        Ok(())
    }
}
```

在 Model 上声明（可重复，按声明顺序执行；`before_*` 正序、`after_*` 逆序，任一 `before_*` 返回 `Err` 即中断并把错误原样上抛）。含 `created_at` / `updated_at` 字段的模型会**自动挂载** `TimestampHook` 并排在链首，**无需**手写 `#[hook(TimestampHook)]`；显式写出它只用于把它摆到指定位置：

```rust,ignore
#[derive(Model)]
#[hook(UppercaseNameHook)]                   // 自定义 Hook（TimestampHook 已自动挂载在链首）
pub struct User {                            // 表名由 `User` 蛇形推导
    id: i64,                                 // 名为 id 的字段默认主键，整型默认自增
    name: String,
    created_at: chrono::DateTime<chrono::Utc>,   // 插入时自动填充
    updated_at: chrono::DateTime<chrono::Utc>,   // 插入和更新时自动填充
}
```

内置 `TimestampHook` 是**单元结构体**（没有 `TimestampHook::new(...)` 构造器），列名固定为 `created_at` / `updated_at`，只对 Row 中已存在的键生效；它已从 `alun-db` 迁到 **`alun_db_core::hook`**（`alun_db::hook` 模块已删除，prelude 已导出 `TimestampHook`）。没有 `HookChain` / `NullHook`——多个 Hook 直接用多条 `#[hook(...)]` 声明。

### AsyncHook（需要 IO 的副作用，提交后触发）

`Hook` 全同步且 `after_*` 在**事务内**执行 —— 所以**不要在 `Hook` 里写库**（会出现「审计已写、业务回滚」）。需要 IO 的副作用（审计落库、缓存失效、发通知）用 **`AsyncHook`**：只提供 `after_*`，且**绑定事务提交边界**。

```rust,no_run
use alun::prelude::*;

struct AuditHook;

#[alun::async_trait]                       // 属性宏由 alun 重导出，无需依赖 async-trait
impl AsyncHook for AuditHook {
    async fn after_insert(&self, row: &Row) -> Result<()> {
        // 此处可 await：IO / 其它库写入 / HTTP 通知
        tracing::info!("审计(提交后) - 新增: {:?}", row.table());
        Ok(())
    }
}

#[derive(Model)]
#[table("invoices")]
#[hook_async(AuditHook)]                   // 与同步的 #[hook(..)] 可并存
pub struct Invoice { #[key] id: i64, amount: i64 }
```

触发时机：`&Db`（自动提交）**立即**；`&Transaction` 下**排队至 `commit()` 成功后**，`rollback()` **整体丢弃**；钩子返回 `Err` **不回滚**事务（已提交），仅记 `warn`。`before_*` **没有**异步版本（事务内 `await` 会拉长持锁、放大死锁）。

软删除：`Option<...>` 类型的 `deleted_at` 字段**自动**作为软删标记（无需 `#[soft_delete]`），`delete` 族改写为 `UPDATE` 标记列 = now，查询族自动追加 `标记列 IS NULL`；用 `#[model(hard_delete)]` 关闭，非 `Option` 的 `deleted_at` 会编译报错：

```rust,ignore
#[derive(Model)]
pub struct Article {                         // 表名推导为 `article`
    id: i64,
    title: String,
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,   // 自动软删标记列
}

article.delete(&db).await?;           // UPDATE article SET deleted_at = now WHERE id = ?
Article::query().find(&db).await?;    // SELECT * FROM article WHERE deleted_at IS NULL
```

## SQL 片段（`sql!` 宏）

`sql!` 在编译期校验模板结构：插值 `{ expr }` **一律转为占位符并绑定参数**（不做字符串拼接），字符串内的 `#` 指令做白名单校验。没有 `SqlTemplate` / Jinja2 模板这一层。

```rust,ignore
use alun::prelude::*;

// 插值 → 参数化
let users: Vec<Row> = sql!("SELECT * FROM users WHERE email = ", { email })
    .find(db())
    .await?;

// 动态条件（Option 为 None 时整段省略）+ 模板指令
let status: Option<i32> = Some(1);
let users = sql!(
    "SELECT * FROM users WHERE 1=1 #and(status, '=', status)",
    " #orderBy(created_at desc) #limit(20)",
)
.find(db())
.await?;

// 条件写在字面量内部：#if 块 + #para 插值（字面量内无法用 { expr }）
let users = sql!(
    "SELECT * FROM users WHERE 1=1 #if(status_filter) AND status = #para(status_filter) #end",
)
.find(db())
.await?;

// 嵌套组合：子查询的 SQL 与参数成对合并
let ids = sql!("SELECT user_id FROM orders WHERE amount > ", { 100.0f64 });
let rows = sql!("SELECT * FROM users WHERE id IN (", ids, ")")
    .find(db())
    .await?;

// 执行：同一段 sql! 既跑 &Db 也跑 &tx（SqlQueryExt 接受任意 E: SqlExecutor）
let affected = sql!("UPDATE users SET status = ", { 9i32 }, " WHERE email = ", { email })
    .execute(&tx)
    .await?;
```

`SqlQueryExt`（`sql!` 产物的终态动词，返回 `Row`）提供 `find` / `find_first` / `find_one`（恰好一行，0 行 `NotFound`、多行 `InvalidValue`）/ `find_one_or_null` / `exists` / `paginate` / `for_each` / `for_each_page` / `scalar::<T, _>` / `execute`。支持的指令：

| 指令 | 语义 | 空参数行为 |
|------|------|-----------|
| `#where(field, op, param)` | WHERE 条件（首个） | 参数为 `None` 时省略 |
| `#and(field, op, param)` | AND 条件 | 参数为 `None` 时省略 |
| `#or(field, op, param)` | OR 条件 | 参数为 `None` 时省略 |
| `#orderBy(f1 desc, f2 asc)` | 排序（方向缺省 ASC） | 始终生效 |
| `#groupBy(f1, f2)` | 分组 | 始终生效 |
| `#limit(n)` | 限制行数（静态整数） | 始终生效 |
| `#if(cond) … #else … #end` | 条件块（可嵌套；`cond` 是 Rust 布尔表达式，编译期解析、调用方作用域求值；`#else` 可选，未闭合是编译错误） | 按条件拼接 |
| `#para(name)` | 把作用域内变量绑定为占位参数（字面量内的插值入口，与 `#if` 配合） | 始终生效 |
| `#having` / `#join` | **不支持**：未实现的指令名会直接报 `unknown template directive`（JOIN 用 `FindBuilder::join`，HAVING 写进 SQL 文本） | — |

token 级 `if cond { ... }` 片段依然是首选写法，`#if/#para` 用于把条件写在**字面量内部**。

## Builder 与 keyset 游标分页

```rust,ignore
use alun::prelude::*;

// 链式条件（FieldExpr<T> 编译期拒绝类型不匹配：User::status().eq(1) ✅ / .eq("1") ❌）
let users: Vec<Row> = db().query()
    .table("users")
    .filter(User::status().eq(1))
    .filter(User::name().like("%张%"))
    .order_by("created_at", Order::Desc)
    .limit(20)
    .exec(db())
    .await?;

// 分页 Builder：自动 COUNT，返回 Page<Row>
let page: Page<Row> = db().page(1, 20)
    .table("users")
    .filter(User::status().eq(1))
    .exec(db())
    .await?;

// 计数 Builder
let n: i64 = db().count().table("users").filter(User::status().eq(1)).exec(db()).await?;

// keyset 游标分页：用上一页尾行的 (排序键, 主键) 构造 Cursor，避免深分页 OFFSET
let last = page.list.last().expect("上一页非空");
let cursor = Cursor {
    key: last.get("id").cloned().unwrap_or(Value::Null),
    pk: last.get("id").cloned().unwrap_or(Value::Null),
};
let next: Vec<Row> = db().query()
    .table("users")
    .order_by("id", Order::Asc)
    .page_after("id", "id", Order::Asc, Some(cursor))
    .exec(db())
    .await?;
```

`FindBuilder`：`table` / `filter` / `filter_fragment(Expr)` / `join(kind, table, on)` / `order_by(col, Order)` / `page_after(sort, pk, Order, Option<Cursor>)` / `limit(n)` / `offset(n)` / `exec(&ex)` / `first(&ex)`。
`PageBuilder`：`table` / `filter` / `order_by` / `page_after` / `exec(&ex) -> Page<Row>`（无 `join` / `limit` / `offset` / `filter_fragment`；keyset 模式下 `total` / `pages` / `page` 均为 0）。
`CountBuilder`：`table` / `filter` / `filter_fragment` / `exec(&ex) -> i64`。
末位 WHERE 片段（`filter_fragment`）只由 `FindBuilder` 与 `CountBuilder` 提供。`PageBuilder` 与 `Db::paginate` 都对 `page` / `size` 做 `max(1)` 下界保护。

**执行入口已由 `&Db` 泛化为 `&impl SqlExecutor`**：`exec` / `first` 既接受 `&Db`，也接受事务内的 `&Transaction`（与 `CrudExt` / `PersistExt` 一致）。

## 分页 → 响应

分页参数全框架只有 **`alun_core::PageQuery`** 一个类型：作为 `Query` 提取器使用时，`page` 缺失 / 为 0 / 越界都会被规整（不会 400，也不会因 `page = 0` 下溢），`page_size` 落在 1..=`MAX_PAGE_SIZE`（缺失时用 `DEFAULT_PAGE_SIZE = 10`）。

```rust,no_run
use alun::prelude::*;
use std::result::Result;

#[derive(serde::Serialize)]
struct UserRes {
    id: String,
    name: String,
}

#[alun::get("/api/users")]
async fn list_users(Query(pq): Query<PageQuery>) -> ResResult<Res<PageData<Vec<UserRes>>>> {
    // DbError 由 From<DbError> for ApiError 自动转 404/409/500 脱敏，无需 map_err
    let page = db()
        .paginate("SELECT * FROM users ORDER BY id", pq.page, pq.page_size)
        .await?;

    // Row 不实现 Serialize：先映射成 DTO，分页元数据（total / page / size）原样保留
    let page = page.map(|r| UserRes {
        id: r.get_typed::<String>("id").unwrap_or_default(),
        name: r.get_typed::<String>("name").unwrap_or_default(),
    });

    Ok(page.into_res())   // { code:0, msg:"ok", data:{ list, total, page, page_size } }
}
```

两个分页容器**字段名不同**，转换入口只有一个：

| 容器 | 来源 | 字段 |
|------|------|------|
| `Page<T>`（`alun::prelude` 导出） | `db().paginate(...)` / `User::query().paginate(...)` / `PageBuilder::exec` | `list` / `total` / `page` / **`size`** / `pages` |
| `alun_core::PageData<T>` | 响应体 `Res::page(...)` 的 `data` | `list` / `total` / `page` / **`page_size`** |

`alun_web::PageResponseExt`（`alun::prelude` 已导出）是二者唯一桥梁：`into_page_data()` → `PageData<Vec<T>>`，`into_res()` → `Res<PageData<Vec<T>>>`。`Page` 还提供 `has_next()` / `has_prev()` / `is_empty()` / `map(f)` / `calc_pages(total, size)`。

强类型 Model 走同一条路径（`T` 只需实现 `Serialize`）：

```rust,ignore
let page: Page<SysUser> = SysUser::query().paginate(db(), pq.page, pq.page_size).await?;
Ok(page.into_res())
```

## Migrations

迁移是**纯 SQL**，没有 DSL。统一由执行器 `Migrator`（`alun::prelude` 导出）驱动，
它自动维护 `schema_migrations(kind, name, version, checksum, applied_seq, …)` 表，
逐条在独立事务中应用；重复版本或失败都会返回 `DbError` 并回滚该条：

- **内容指纹**：每个版本记录 `up` 脚本的 SHA-256；已应用版本的文件被改动后再次运行会报「内容不一致」，
  不会静默跳过（历史导入行无指纹，首次运行时补录）；
- **并发保护**：PG 用 `pg_try_advisory_lock`、MySQL 用 `GET_LOCK`、SQLite 用进程内互斥，
  多进程/多任务同时迁移会串行化，同一版本只应用一次；
- **回滚顺序**：`Migrator::rollback` 按 `applied_seq`（应用顺序）回退最近一条，
  若该版本不属于本执行器的来源则报 `DbError::Migration` 且不改动任何表——多目录（base → common → evolve）需严格逆序回滚；

```rust,ignore
use alun::prelude::*;

// 方式一：代码内清单（Migration 为拥有型，需运行时构造）
let migrations = vec![
    Migration::new(
        1,
        "create_users",
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
        "DROP TABLE users",
    ),
];
Migrator::from_migrations(migrations).run(&db).await?;      // 应用待应用迁移，返回应用数

// 方式二：目录扫描（文件名编码版本号与方向）
let migrator = Migrator::from_dir(std::path::Path::new("migrations"));
migrator.run(&db).await?;                                   // 应用
migrator.rollback(&db).await?;                              // 回退最近一条（LIFO，返回 Option<Version>）

// 方式三：多来源目录合并（版本号跨目录全局唯一）
Migrator::new()
    .source_dir("migrations/system")
    .source_dir("migrations/support")
    .run(&db)
    .await?;
```

目录 runner 的文件命名规则：`V{n}__Name.sql`（仅 up）或 `V{n}__Name.up.sql` + `V{n}__Name.down.sql`（成对）；
`version` 全局唯一；**每个文件可写多条** SQL 语句（内置方言感知拆分器逐条执行，`-- +no-transaction` 可跳出事务）；
支持纯数字前缀（`01_base.sql`）与子目录递归，非 `.sql` 文件跳过：

```text
migrations/
  V1__create_users.sql
  V2__add_users_age.up.sql
  V2__add_users_age.down.sql
  R__active_user_view.sql      # 可重复：内容变化即重跑
  S__sys_dict.sql              # 种子：幂等重灌
  B2__baseline.sql             # 基线标记：声明 <= 2 已应用（内含语句会报错）
```

`R` / `S` / `B` 三个种类前缀可改写（`repeatable_prefix` / `seed_prefix` / `baseline_prefix`），
设为空串即关闭该种类；三者必须互不相同且不以数字开头，否则加载前报 `DbError::InvalidValue`。

`App::start()` 可结合 `[database.migration]` 配置自动执行（需 `enabled = true` 且 `auto_migrate = true`；`path` 配置目录则走 `Migrator::from_dir`，为空则只应用框架注入的静态清单）。另有 `alun-db-build`：应用 crate 在 `[build-dependencies]` 声明它，并在 `build.rs` 调用 `alun_db_build::build_snapshot("migrations")` 生成编译期 schema 快照，配合 `#[model(strict)]` 把「拼错列名」变成编译错误（快照目录用 `YYYYMMDD_HHMMSS_desc.sql` + `-- +migrate Up/Down` 分段标记）。

旧 API（`Db::migrate` / `migrate_down` / `migrate_from_dir` / `migrate_down_from_dir` 及同名自由函数）**已全部移除**，迁移入口统一为 `Migrator`。除 `run` / `rollback` 外还提供：`status` / `plan`（观察状态、待应用清单）、`baseline`（存量库把 `<= version` 标记为已应用）、`repair`（修记录表指纹、清理已删记录）、`dry_run`（预演，执行后回滚不落库）。执行策略由 `MigratorOptions` 控制：`split_statements` / `up_down_sections` / `version_prefix` / `name_separator` / `repeatable_prefix`（默认 `R`）/ `seed_prefix`（默认 `S`）/ `baseline_prefix`（默认 `B`）/ `transaction_default` / `validate_checksum` / `divergent`（默认 `Abort`）/ `out_of_order` / `import_legacy` / `baseline_on_migrate` / `baseline_version`；其中 `import_legacy`（导入旧 `_sqlx_migrations`）**默认关闭**，存量库接入时需显式开启。`Migration` 不再是 `Copy` 的 `&'static str` 常量（`new` 不是 `const fn`），清单须在运行时构造。

## Configuration

```toml
[database]
enabled = true
type = "postgres"           # postgres | mysql | sqlite
host = "localhost"
port = 5432                 # 可选；sqlite 不需要
name = "mydb"
user = "app_user"
password = ""
password_encrypted = false  # AES-GCM 加密密码（server.key 解密）
max_connections = 10
min_connections = 2
connect_timeout = 30        # 秒
sql_logging = false
slow_query_ms = 0           # 0 = 关闭慢查询日志
ssl_mode = "prefer"         # postgres 专用：disable|prefer|require|verify-ca|verify-full

[database.migration]
enabled = false
path = "migrations"
auto_migrate = false
sources = []             # 非空时取代 path；多来源目录合并
statement_split = true   # 是否按方言拆分单文件多语句
up_down_sections = true  # 是否识别 -- +migrate Up/Down 分段
version_prefix = ""      # 空串 = 自动（V 前缀或无前缀）
name_separator = ""      # 空串 = 自动（优先 __，回退 _）
```

## Key Enums

| 类型 | 变体 / 说明 |
|------|------------|
| `Order` | `Asc`、`Desc` — 排序方向（`order_by` / `page_after`） |
| `NullsOrder` | `First`、`Last` — NULL 排序位置 |
| `JoinKind` | `Inner`、`Left`、`Right`、`Full` |
| `ConflictAction` | `DoNothing`、`DoUpdate { ... }` — `upsert` / `ON CONFLICT` |
| `Placeholder` | `Question`（MySQL/SQLite）、`Dollar`（PostgreSQL）— 由方言决定 |
| `Type` | `Bool`/`I8..I64`/`U8..U64`/`F32`/`F64`/`Text`/`Bytes`/`Date`/`Time`/`Timestamp`/`TimestampTz`/`Uuid`/`Ulid`/`Decimal`/`Json`/`Enum`/`Inet`/`Cidr`/`Array`/`Nullable`/`Unknown` |
| `DbError` | `Connection`、`Query`、`NotFound`、`Readonly`、`UniqueViolation`、`ForeignKeyViolation`、`CheckViolation`、`ColumnNotFound`、`TypeMismatch`、`UnexpectedNull`、`UnsupportedFeature`、`Transaction`、`InvalidValue`、`Migration`；事务冲突类另有 `Deadlock`（PG `40P01` / MySQL `1213`）、`SerializationFailure`（PG `40001`）、`LockTimeout`（MySQL `1205` / PG `55P03` / SQLite busy 家族）。约束类变体的 `column`/`constraint` 为**结构化 `Option<String>`**（取不到为 `None`，驱动原文保留在 `source()` 链）。DB 层的 `Result<T, E = DbError>` 别名定义在 `alun_db_core::error`；在 `alun::prelude::*` 下该名字被框架的 `Result<T>`（`alun_core::Result`，单泛型、错误类型 `Error`）遮蔽，写 DB 错误结果请用 `std::result::Result<T, DbError>` |

**不存在**：`Isolation`（无隔离级别参数）、`IdKind`（主键类型自动按值推断）、`DbResult` / `ActiveTx`（分别由 `alun_db_core::error::Result` 与 `Transaction` 取代）、`alun_db::PageQuery`（已删除，统一用 `alun_core::PageQuery`）、`Row::table("t")` 构造器 / `Row::set` / `Row::get_as` / `Row::get_id` / `Row::has` / `Row::clear_changes` / `Row::primary_keys`、`Db::insert` / `update` / `delete` / `find_by_id` / `delete_by_id` / `query_page`、`Db::find_as` / `Db::find_as_with_params`（改用 `Model::sql(sql!(...)).find(&db)`）、`ModelQuery::select` 列投影（改用原始 `sql!` + `Row`）。

**旧 API 已全部改名**（本次破坏性重构，迁移时按下表替换）：`update_columns` → `ChangeSet`（`User::of(pk).set(field, v).update(&db)`）、`insert_batch` → `insert_many`、`save` → `insert` / `insert_or_update`、`fetch_all` / `fetch_one` / `fetch_or_none` → `find` / `find_one` / `find_first`（`sql!` 链式终态）、`find_by_pk` / `find_or_none_by_pk` → `find` / `find_or_none`、`find_where` / `find_first_where` / `count_where` / `delete_where` → `find_by` / `find_first_by` / `count_by` / `delete_by`、`User::all(&db)` → `User::query().find(&db)`、`User::paginate(&db, p, s)` → `User::query().paginate(&db, p, s)`、`alun_db::hook::` → `alun_db_core::hook::`（prelude 已导出 `TimestampHook`）。静态按主键删除仍叫 `delete_by_pk`。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [路由 · 统一响应 · 分页](routing.md) | [进阶 DB（联查 / 白名单 / 租户安全 / 多数据源）](advanced-db.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

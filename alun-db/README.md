# alun-db 设计文档

> 一个融合 [aifei](https://github.com/jfinal/aifei) 便捷性与 Toasty 类型安全的 Rust 数据库操作库（设计理念溯源见 [docs/architecture/aifei-tribute.md](../docs/architecture/aifei-tribute.md)）

---

## 目录

1. [项目概述](#一项目概述)
2. [设计思路](#二设计思路)
3. [整体架构](#三整体架构)
4. [核心类型系统](#四核心类型系统)
5. [多数据库支持](#五多数据库支持)
6. [查询与持久化 API](#六查询与持久化-api)
7. [宏系统](#七宏系统)
8. [SQL 模板与渲染器](#八sql-模板与渲染器)
9. [事务与 Hook](#九事务与-hook)
10. [迁移系统](#十迁移系统)
11. [库亮点](#十一库亮点)
12. [使用示例](#十二使用示例)
13. [实施路线](#十三实施路线)
14. [附录：Crate 清单](#十四附录crate-清单)

---

## 一、项目概述

### 1.1 定位

**alun-db** 是一款面向 AI Coding 时代的 Rust 数据库操作库。它深度融合了 Java 生态 **aifei 框架**的低认知负荷调用风格，与 Rust 生态 **Toasty 库**的编译期类型安全设计，为 Rust 开发者提供一种既便捷又安全的数据库访问方式。

> 作为 **`alun` 框架**的一部分使用时，应用无需在自己的 `Cargo.toml` 里声明本 crate：
> `alun::prelude::*` 已完整转发本 crate 的 prelude（`Model` / `TypeMapper` / `CrudExt` / `PersistExt` /
> `ModelQuery` / `ChangeSet` / `sql!` / `DbError` …），驱动与生态类型在 `alun` 的 feature 里选；
> `alun-db-macro` 也会按应用实际声明的 crate 解析生成代码里的内核路径。本 crate 可独立使用，
> 但标准做法是「只依赖 `alun`」。

### 1.2 设计目标

| 目标 | 说明 |
|---|---|
| **统一入口** | 所有数据库操作从 `Model` 关联函数或 `Db` 门面出发，风格可预测 |
| **编译期安全** | 字段类型与 Schema（`#[model(strict)]`）在编译期校验，AI 生成的代码一编译即知对错；方言差异在渲染期协商，可降级则降级、不可降级则显式报错 |
| **多数据库原生** | PostgreSQL / MySQL / SQLite 三方言一等公民，差异由框架透明处理 |
| **AI 友好** | 低 Token 消耗、可预测的调用模式、指向明确位置的宏报错 |
| **低开销抽象** | 表达式树与渲染路径不引入额外分配与动态派发拷贝，异步与同步共享核心 |

### 1.3 一句话概括

**alun-db = aifei 的调用形式 + Toasty 的编译期校验 + Rust 的 RAII 事务 + 方言能力协商。**

---

## 二、设计思路

### 2.1 从 aifei 继承什么

aifei 框架的核心洞察是：**AI Coding 时代框架的首要任务是降低上下文噪音**。它通过以下设计实现这一目标：

- **统一数据库入口**：所有查询从 `sql()` 开始链式调用，避免 8 个不同 API 入口的选择困难
- **Model is-a Row**：Model 与 Row 不是并列的两套体系，Model 本身就是 Row 的强类型投影
- **SQL 模板语法**：`#where(...)`、`#orderBy(...)` 等指令简化动态 SQL 拼接
- **全局资源单例**：`db()` 直接访问，无需 State 注入

alun-db 全部继承这些理念，但将其转换为 Rust 惯用形式。

### 2.2 从 Toasty 继承什么

Toasty 库证明了 **Rust 可以在不牺牲类型安全的前提下提供 ORM 的便利**：

- **双层 Schema**：App Schema 与 DB Schema 解耦，通过 Mapping 层连接
- **Capability 协商**：驱动不试图隐藏数据库差异，而是显式声明支持的能力
- **RPITIT 异步**：不使用 `async_trait`，trait 方法直接返回 `impl Future + Send`，零堆分配
- **编译期代码生成**：`#[derive(Model)]` 生成字段表达式访问器、`Model` 实现与 Hook 注册

alun-db 采纳其核心设计，并增强了类型映射层（生态类型 feature）与 schema 校验
（`#[model(strict)]` + build.rs 快照）。

### 2.3 核心设计原则

| 原则 | 含义 |
|---|---|
| **编译期能推导的不运行时做** | 字段类型与 Schema 在宏展开时校验；方言能力声明固定，差异由渲染期协商处理 |
| **aifei 能用一行表达的不拆成三行** | 日常 CRUD 保持简洁，复杂场景才用 Builder |
| **Toasty 能安全暴露的不做无谓抽象** | Raw SQL 是显式逃逸口，不隐藏但也不主动鼓励 |
| **Hook 在 Model 层拦截** | 绕过 Model 即主动放弃 Hook，语义清晰 |
| **迁移文件是真相来源** | 不发明 DSL，纯 SQL 迁移文件是 schema 唯一定义 |

---

## 三、整体架构

### 3.1 Crate 结构

```text
alun-db/
├── alun-db              # facade，用户唯一直接依赖
├── alun-db-core         # trait、AST、错误、Page、Row
├── alun-db-macro        # proc-macro：derive(Model)、derive(Row)、derive(TypeMapper)、sql!
├── alun-db-sql          # SQL 渲染器、方言
├── alun-db-migrate      # 迁移执行器（规划中，尚未单列）  【规划中，尚未建立】
├── alun-db-build        # build.rs 辅助库：迁移文件 → schema 快照
├── alun-db-driver-sqlite
├── alun-db-driver-postgres
└── alun-db-driver-mysql
```

> 迁移执行器（`Migrator`，位于 `alun_db::migrate` 模块）当前内置在 facade `alun-db` 中，
> 尚未拆分为独立的 `alun-db-migrate` crate。

### 3.2 依赖方向

```text
alun-db ──┬─> alun-db-core ──> alun-db-sql
          ├─> alun-db-macro (proc-macro，编译期)
          ├─> alun-db-migrate ──> alun-db-sql  【规划中，尚未建立】
          └─> alun-db-driver-* ──> alun-db-core, alun-db-sql
```

**依赖约束**：`alun-db-core` 零异步依赖、零驱动依赖、零生态类型依赖。生态类型（chrono / rust_decimal / uuid）通过 feature 逐个启用。

### 3.3 分层模型

```text
┌─────────────────────────────────────────────┐
│  用户应用                                    │
├─────────────────────────────────────────────┤
│  alun-db (facade)                            │
│  - Db 门面 / ModelQuery / ChangeSet / Hook  │
├─────────────────────────────────────────────┤
│  alun-db-macro        alun-db-sql           │
│  - #[derive(Model)]   - 方言渲染器           │
│  - sql!               - 表达式/语句渲染      │
├─────────────────────────────────────────────┤
│  alun-db-core                                │
│  - Model / Row / Expr / Statement / Type    │
│  - TypeMapper / Dialect / SqlExecutor / Hook│
├─────────────────────────────────────────────┤
│  alun-db-driver-*                            │
│  - SQLite / PostgreSQL / MySQL               │
└─────────────────────────────────────────────┘
```

---

## 四、核心类型系统

### 4.1 三层类型解耦

alun-db 的核心是**三层类型解耦**：

```text
Rust 类型（应用层）
    ↓  TypeMapper trait
中间类型（stmt::Type，方言无关抽象）
    ↓  Dialect::render_type()
数据库列类型（每个方言的具体类型）
    ↓  驱动 bind/read
数据库存储值
```

**为什么需要中间层**：

- SQLite 没有原生 `BOOLEAN`，用 `INTEGER 0/1`
- MySQL 没有原生 `TIMESTAMPTZ`，用 `DATETIME(6)`
- PostgreSQL 有原生 `JSONB`、`UUID`，SQLite 只能存 `TEXT`

有了中间层，Rust 类型→中间类型固定一次定义，中间类型→数据库类型由每个方言独立决定。

### 4.2 类型映射对照表

| Rust 类型 | 中间类型 | PostgreSQL | MySQL | SQLite |
|---|---|---|---|---|
| `bool` | `Bool` | `BOOLEAN` | `TINYINT(1)` | `INTEGER` (0/1) |
| `i32` | `I32` | `INTEGER` | `INT` | `INTEGER` |
| `i64` | `I64` | `BIGINT` | `BIGINT` | `INTEGER` |
| `f64` | `F64` | `DOUBLE PRECISION` | `DOUBLE` | `REAL` |
| `String` | `Text` | `TEXT` | `VARCHAR(255)` | `TEXT` |
| `Vec<u8>` | `Bytes` | `BYTEA` | `BLOB` | `BLOB` |
| `Option<T>` | `Nullable(T)` | `T` (nullable) | `T` (nullable) | `T` (nullable) |
| `NaiveDate` | `Date` | `DATE` | `DATE` | `TEXT` (ISO 8601) |
| `NaiveDateTime` | `Timestamp` | `TIMESTAMP` | `DATETIME` | `TEXT` |
| `DateTime<Utc>` | `TimestampTz` | `TIMESTAMPTZ` | `DATETIME(6)` | `TEXT` |
| `Uuid` | `Uuid` | `UUID` | `CHAR(36)` | `TEXT` |
| `Ulid` | `Ulid` | `UUID` | `CHAR(26)` | `TEXT` |
| `Decimal` | `Decimal` | `NUMERIC` | `DECIMAL(38, 10)` | `TEXT` |
| `IpAddr` | `Inet` | `INET` | `VARCHAR(45)` | `TEXT` |
| `serde_json::Value` | `Json` | `JSONB` | `JSON` | `TEXT` |

### 4.3 生态类型支持

alun-db 通过 Cargo feature 逐项启用生态类型，核心层只依赖 `thiserror`：

| Feature | 支持类型 |
|---|---|
| `chrono` | `NaiveDate`、`NaiveTime`、`NaiveDateTime`、`DateTime<Utc>`、`DateTime<FixedOffset>` |
| `time` | `time::Date`、`time::Time`、`PrimitiveDateTime`、`OffsetDateTime` |
| `jiff` | `civil::Date`、`Timestamp`、`Zoned`（`civil::Time` 暂未实现） |
| `rust_decimal` | `rust_decimal::Decimal` |
| `bigdecimal` | `bigdecimal::BigDecimal` |
| `uuid` | `uuid::Uuid` |
| `ulid` | `ulid::Ulid` |
| `json` | `serde_json::Value` |
| `ipnetwork` | `ipnetwork::IpNetwork`、`std::net::IpAddr` |

### 4.4 TypeMapper trait

```rust,ignore
pub trait TypeMapper: Sized {
    /// 该类型对应的中间类型
    fn stmt_type() -> Type;

    /// 转为运行时值
    fn to_value(&self) -> Value;

    /// 从运行时值恢复
    fn from_value(v: &Value) -> Result<Self>;
}
```

**双向可逆性**是核心约束：`from_value(&x.to_value())` 与 `x` 语义等价（在精度范围内）。这是跨数据库读写不丢失信息的前提。

### 4.5 枚举类型四模式

枚举不支持 `#[derive(Model)]`（仅支持 struct，见 7.1），通过
`#[derive(TypeMapper)]` 生成 `TypeMapper` 实现，支持四种编码模式：

```rust,ignore
// 模式一：字符串编码（默认）
#[derive(TypeMapper)]
enum Status { Active, Inactive }
// 存为 "Active" / "Inactive"

// 模式二：整数编码
#[derive(TypeMapper)]
#[int]
enum Status { Active = 1, Inactive = 0 }
// 存为 1 / 0

// 模式三：自定义编码
#[derive(TypeMapper)]
#[value_map("A" => Active, "I" => Inactive)]
enum Status { Active, Inactive }
// 存为 "A" / "I"

// 模式四：PostgreSQL 原生 ENUM
#[derive(TypeMapper)]
#[native_enum("order_status")]
enum OrderStatus { Pending, Confirmed, Shipped }
// 存为原生 order_status 类型
```

### 4.6 JSON 与嵌套结构

```rust,ignore
#[derive(Model)]
#[table("users")]
pub struct User {
    #[key] #[auto] id: i64,
    name: String,
    #[json] profile: Profile,          // 自动序列化为 JSON
    #[json(nullable)] extra: Option<Extra>, // NULL 时返回 None
}

#[derive(Row, Serialize, Deserialize)]
pub struct Profile {
    avatar: Option<String>,
    bio: String,
}
```

`#[json]` 属性在派生时生成 `TypeMapper` 实现，将结构体序列化为 `serde_json::Value`。方言渲染时自动选择 `JSONB` / `JSON` / `TEXT`。**用户代码中 `user.profile.bio` 就是普通的 Rust 字段访问**。

---

## 五、多数据库支持

### 5.1 Capability 协商

每个方言声明自己支持的能力集合：

```rust,ignore
pub struct DialectCapability {
    pub native_bool: bool,          // 是否有原生 BOOLEAN
    pub native_uuid: bool,          // 是否有原生 UUID
    pub native_json: bool,          // 是否有原生 JSON
    pub native_enum: bool,          // 是否有原生 ENUM
    pub returning_clause: bool,     // INSERT ... RETURNING
    pub ilike: bool,                // ILIKE 运算符
    pub array_type: bool,           // 数组类型
    pub on_conflict: bool,          // ON CONFLICT
    pub nulls_order: bool,          // NULLS FIRST / LAST
    pub placeholder: Placeholder,   // ? 或 $1
}
```

### 5.2 方言能力对照

| 能力 | PostgreSQL | MySQL | SQLite |
|---|---|---|---|
| `native_bool` | ✅ | ❌ | ❌ |
| `native_uuid` | ✅ | ❌ | ❌ |
| `native_json` | ✅ JSONB | ✅ JSON | ❌ |
| `native_enum` | ✅ | ✅ | ❌ |
| `returning_clause` | ✅ | ❌ | ✅ (3.35+) |
| `ilike` | ✅ | ❌ | ❌ |
| `array_type` | ✅ | ❌ | ❌ |
| `on_conflict` | ✅ | ❌ | ✅ |
| `nulls_order` | ✅ | ❌ | ✅ |
| `placeholder` | `$1` | `?` | `?` |
| 标识符引用 | `"name"` | `` `name` `` | `"name"` |

### 5.3 方言能力不可用时的处理

能力协商的结果在**渲染期**生效，而不是编译期：

- **可降级的能力**：渲染器自动改写为语义等价的写法，保留语义、不报错。
  例如 `ILIKE` 在 MySQL / SQLite 上渲染为 `LOWER(x) LIKE LOWER(y)`；
  无 `returning_clause` 的方言不追加 `RETURNING`，自增主键改由驱动回读
  （`last_insert_id`）。
- **不可降级的能力**：在真正的调用点返回 `DbError::UnsupportedFeature`，
  由调用方决定处理方式。例如 `PersistExt::upsert` 在无 `on_conflict` 能力的
  方言（MySQL）上、`Db::execute_script` 在未启用多语句的驱动上。

```rust,ignore
// 仅 PostgreSQL / SQLite（on_conflict = true）支持 upsert
match user.upsert(db(), &["email"], &["name"]).await {
    Err(DbError::UnsupportedFeature { feature, dialect }) => {
        // feature = "upsert"、dialect = "mysql"
    }
    other => other?,
}
```

### 5.4 JSON 路径查询的方言适配

> **规划中。** 表达式树已具备 `Expr::JsonGet` 节点（见 8.1），渲染器可按方言输出，
> 但面向用户的字段级入口 `FieldExpr::json_get` 尚未开放，暂以 `find_by` + SQL
> 片段替代。以下为设计意图（条件进入已实现的 `ModelQuery` 链）：

```rust,ignore
// 目标写法（待实现）
Event::query()
    .filter(Event::payload().json_get("channel").eq("alipay"))
    .find(&db).await?;
```

| 方言 | 渲染结果 |
|---|---|
| PostgreSQL | `profile->>'bio' = $1` |
| MySQL | `JSON_UNQUOTE(JSON_EXTRACT(profile, '$.bio')) = ?` |
| SQLite | `json_extract(profile, '$.bio') = ?` |

---

## 六、查询与持久化 API

### 6.1 调用风格总览

alun-db 的调用风格以 **Model 关联函数**为主入口，**实例方法**做持久化。
静态糖（`CrudExt`）全部是 `ModelQuery`（见 6.2）/ `CountBuilder` 的一行转发，
**唯一实现永远只有一份**：

```rust,ignore
// ==================== 查询（关联函数）====================

// 主键查询（不存在 → DbError::NotFound）
let user: User = User::find(&db, 1).await?;

// 主键查询，不存在返回 None
let user: Option<User> = User::find_or_none(&db, 1).await?;

// unique 字段没有专门的 finder：单参数 + 合法标识符 → 「字段 = ?」
let user = User::find_first_by(&db, "email", ("a@b.com",)).await?;

// 全部（软删模型自动追加 `deleted_at IS NULL`）
let users: Vec<User> = User::query().find(&db).await?;

// 条件查询：字段名 + 值
let users = User::find_by(&db, "status", (1,)).await?;
// 条件查询：WHERE 片段（自带占位符，可带 ORDER BY / LIMIT）
let users = User::find_by(&db, "status = ? order by created desc", (1,)).await?;

// 单条
let user = User::find_first_by(&db, "email = ?", ("a@b.com",)).await?;

// 单列 IN / 主键 IN
let users = User::find_in(&db, User::status(), [1, 2]).await?;
let users = User::find_in_ids(&db, [1, 2, 3]).await?;

// 分页
let page: Page<User> = User::query().paginate(&db, 1, 20).await?;

// 计数
let n: i64 = User::count(&db).await?;
let n: i64 = User::count_by(&db, "status", (1,)).await?;

// ==================== 持久化（实例方法）====================

let mut user = User { email: "a@b.com".into(), name: "张三".into(), .. };
user.insert(&db).await?;                        // 纯 INSERT，自增 id 回填到 user.id
user.insert_or_update(&db).await?;              // 自增主键为占位值时 INSERT，否则按存在性判定
user.name = "李四".into();
user.update(&db).await?;                        // UPDATE 全部字段
user.delete(&db).await?;                        // DELETE（软删模型改写为标记删除）

// 按主键删除，不需先查出来（静态侧保留 `delete_by_pk` 之名）
User::delete_by_pk(&db, 1).await?;
User::delete_by(&db, "status", (0,)).await?;
User::delete_in_ids(&db, [1, 2, 3]).await?;

// ==================== 变更集（免 SELECT 的部分更新 / 删除）====================

User::of(1)
    .set(User::name(), "李四")
    .set(User::status(), 2)
    .update(&db).await?;                        // 只 UPDATE 被 set 的列
User::of(1).delete(&db).await?;                 // 免 SELECT 删除
```

`Params` 由 `()` 与 0~12 元组实现，元素只需 `Into<Value>`。`find_by` /
`find_first_by` / `count_by` / `delete_by` 的第一个参数是 **`where_or_field`
双态签名**：只有一个参数且恰好是合法标识符时按「字段 = ?」处理，否则整段作为
拼进 `WHERE` 之后的 SQL 片段（片段内自行书写占位符，因此可以带 `ORDER BY` /
`LIMIT`）。字段名走标识符白名单校验；需要方言引号安全时改用类型化链式写法
（`User::query().filter(User::status().eq(1))`）。

**关于分页**：`Page<T>` 的字段是 `list / total / page / size / pages`；Web 层
响应用的是 `alun_core::PageData<T>`（字段为 `list / total / page / page_size`）。
两者之间的唯一转换入口是 `alun_web::PageResponseExt`（`into_page_data` /
`into_res`），且 `Row` 不实现 `Serialize`，放进响应前需先 `page.map(...)`
映射成自定义 DTO。分页参数同样是全框架唯一的 `alun_core::PageQuery`
（`page` / `page_size`，反序列化即规整），数据库侧直接取字段：

```rust,ignore
use alun::prelude::*;

let pq = PageQuery::new(1, 20);                 // 或从查询串反序列化
let page = db().paginate("SELECT * FROM users", pq.page, pq.page_size).await?
    .map(|r| UserRes { id: r.get_typed("id").unwrap_or_default() }); // Row → DTO
let res = page.into_res();                       // Res<PageData<Vec<UserRes>>>
```

### 6.2 `ModelQuery` —— 强类型查询脊柱

链式查询只有两条构造入口，产出同一种 `ModelQuery<T>`，**终态动词完全一致**；
链式 API 只负责选择执行方式，语义一律由 SQL 表达：

```rust,ignore
// ① 表名 + 链式条件（自动追加软删除过滤）
let page: Page<User> = User::query()
    .filter(User::status().eq(1))
    .filter(User::name().like("%张%"))
    .filter(User::created_at().ge(since))
    .order_by(User::created_at().desc())
    .paginate(&db, 1, 20).await?;

// ② 原始 SQL 脊柱（不做软删过滤，WHERE 自己写；链式修饰不可用）
let users: Vec<User> = User::sql(sql!(
    "SELECT * FROM user WHERE tenant_id = ", { tid },
)).find(&db).await?;
```

链式修饰（仅 ① 可用）：`filter(Expr)` / `order_by(OrderBy)` / `limit(u64)` /
`offset(u64)` / `page_after(sort_col, pk_col, Order, Option<Cursor>)` / `filter_fragment(Expr)`。
- 另有低级入口 `filter_fragment(Expr)`：片段渲染在 WHERE 子句**最后且不加括号**，因此可自带 `ORDER BY` / `LIMIT` 等尾部子句 —— `find_by` / `find_first_by` / `count_by` 的 `where_or_field` 片段形态正是通过它实现。


终态动词：`find` / `find_first` / `find_one` / `find_one_or_null` / `exists` /
`paginate(&db, page, size)` / `for_each(&db, |t| Ok(true))` /
`for_each_page(&db, size, |page| Ok(true))` / `scalar::<V, _>(&db)` / `execute(&db)`。

| 终态 | 返回 | 语义 |
|---|---|---|
| `find` | `Vec<T>` | 全部匹配行 |
| `find_first` | `Option<T>` | 首条（追加 `LIMIT 1`） |
| `find_one` | `T` | **恰好一行**：0 行 → `DbError::NotFound`，多行 → `DbError::InvalidValue`（用 `LIMIT 2` 探测，不需全量取回） |
| `find_one_or_null` | `Option<T>` | 0 或 1 行；多行 → `DbError::InvalidValue` |
| `exists` | `bool` | 是否存在匹配行 |
| `paginate` | `Page<T>` | 页码从 1 开始，返回完整 `total` / `pages` |
| `for_each` | `()` | 结果**一次性取回**后遍历，回调返回 `Ok(false)` 提前结束 |
| `for_each_page` | `()` | 真分页遍历（`total` / `pages` 为 0，`page` 为当前页号），内存占用受 `size` 约束 |
| `scalar::<V, _>` | `V` | 首行首列（聚合查询用） |
| `execute` | `u64` | 写语句的受影响行数 |

语义要点：

- 静态糖与 `ModelQuery` 是**同一实现**：`User::find_by(...)` 与
  `User::query().find(...)` 落到同一条渲染路径，不存在两套语义。
- ② 形态调用 `filter` / `order_by` / `limit` / `offset` / `page_after` 时，
  终态返回 `DbError::InvalidValue`——原始 SQL 里直接写即可。
- **没有** `select(...)` 列投影 API：投影结果无法构造 `T`，需要投影时直接用
  `sql!("SELECT id, email FROM user ...").find(&db)` 取动态 `Row`。

字段表达式通过 `FieldExpr<T>` 携带类型信息，编译期拒绝类型不匹配：

```rust,ignore
User::age().eq(18)        // ✅ 编译通过
User::age().eq("18")      // ❌ 编译失败
```

动态 `Row` 查询仍可用 `Db` 门面上的 `FindBuilder`（自身不带类型，条件由 `Expr` 表达）：

```rust,ignore
let rows: Vec<Row> = db().query()
    .table("users")
    .filter(User::status().eq(1))
    .order_by("created_at", Order::Desc)
    .limit(20)
    .exec(db())        // exec 接受 `&impl SqlExecutor`，事务内直接传 `&tx`
    .await?;
```

> 三个 Builder 的执行入口都泛化于 `&impl SqlExecutor`：`FindBuilder::exec` /
> `first`、`PageBuilder::exec`（`Db::page` 构造）、`CountBuilder::exec`
> （`Db::count` 构造）——事务内传 `&tx` 即可（见 9.2）。

### 6.3 SQL 模板（aifei 风格）

aifei 的 `#` 指令用于处理复杂动态 SQL。当前入口是 **`sql!` 宏**（编译期校验，
见 7.4），产物是 `SqlQuery`：既可走 `SqlQueryExt` 的终态动词（返回动态 `Row`），
也可经 `User::sql(...)` 作为 `ModelQuery` 的原始 SQL 脊柱（返回强类型 `User`）：

```rust,ignore
// 已实现：sql! 宏（字符串内的 # 指令编译期校验）
// 条件指令的第三个参数是**作用域内的 Option<T> 变量名**，为 None 时整段省略
let status: Option<i32> = Some(1);
let users: Vec<Row> = sql!(
    "SELECT * FROM users WHERE 1=1 #and(status, '=', status)",
    " #orderBy(created desc) #limit(20)",
)
.find(&db)
.await?;

// 强类型：同一段 SQL 交给 ModelQuery，结果映射为 User
let users: Vec<User> = User::sql(sql!(
    "SELECT * FROM user WHERE status = ", { 1i32 },
))
.find(&db)
.await?;
```

`#where(...)` 只在参数为 `None` 时省略整个条件，`#orderBy` 处理动态排序。
插值 `{ expr }` 一律转为占位符并绑定参数（不拼接字符串）。支持的指令集：

| 指令 | 语义 | 空参数行为 | 状态 |
|---|---|---|---|
| `#where(f, op, p)` | WHERE 条件（首个条件） | 参数为 `None` 时省略 | 已实现 |
| `#and(f, op, p)` | AND 条件 | 参数为 `None` 时省略 | 已实现 |
| `#or(f, op, p)` | OR 条件 | 参数为 `None` 时省略 | 已实现 |
| `#orderBy(f1 desc, f2 asc)` | 排序（方向缺省为 ASC） | 始终生效 | 已实现 |
| `#groupBy(f1, f2)` | 分组 | 始终生效 | 已实现 |
| `#limit(n)` | 限制行数（静态整数） | 始终生效 | 已实现 |
| `#if(cond) … #else … #end` | 条件块（可嵌套，`#else` 可选） | `cond` 为 false 时整块省略 | 已实现 |
| `#para(name)` | 把作用域内变量绑定为占位参数 | —— | 已实现 |
| `#having(f, op, p)` | HAVING | —— | 规划中 |
| `#join(t, on, type)` | JOIN | —— | 规划中（链式 `FindBuilder::join` 已可用） |

`#` 指令的参数（列名、操作符、变量名、整数）全部在编译期做白名单校验，
非法输入直接编译失败；单引号字符串字面量内的 `#` 不会被当作指令解析。

**字面量内部的动态条件**：token 级 `if cond { ... }` 片段（见 7.4）依然是首选写法；
需要把条件写在**字符串字面量内部**时用 `#if` / `#para`：

```rust,ignore
sql!(
    "SELECT * FROM invoice WHERE tenant_id = ", { tid },
    " #if(status_filter) AND status = #para(status_filter) #end",
)
.find(&db)
.await?;
```

`#if` 的 `cond` 是 Rust 布尔表达式（编译期解析、调用方作用域求值），`#para(name)`
把作用域内变量绑定为占位参数（变量名走标识符白名单校验）；`#if` 可嵌套，
未闭合的 `#if` 是编译错误。

### 6.4 Row 动态查询

不定义 Model 时，直接使用 `Db` 门面操作动态 `Row`：

```rust,ignore
// 跨表聚合查询
let records: Vec<Row> = db()
    .find("select u.name, count(o.id) as cnt from users u
           left join orders o on o.user_id = u.id
           group by u.name")
    .await?;

for r in &records {
    println!("{}: {}", r.get_typed::<String>("name")?, r.get_typed::<i64>("cnt")?);
}

// 分页（Page 的字段为 list / total / page / size / pages）
let page: Page<Row> = db().paginate("select * from users", 1, 20).await?;

// 单值 / 首行
let n: i64 = db().scalar("select count(*) from users").await?;
let first: Option<Row> = db().find_first("select * from users order by id").await?;

// 带参数执行（参数是 Vec<Value>；也可用 sql! 宏让插值自动参数化）
let affected = db()
    .execute_with_params("update users set status = 0 where created < ?", vec![Value::I64(cutoff)])
    .await?;

// 动态行的写操作：Row 自带实体元数据（表名 + 主键列名）即可免写 SQL
let mut row = Row::new().with_entity("users", "id");
row.insert("name", "张三");
let inserted: Row = db().insert_row(&row).await?;        // 主键未赋值时由数据库生成并回填
let found: Option<Row> = db().find_row_by_pk("users", "id", 1).await?;
let n: u64 = db().update_row(&inserted).await?;
let n: u64 = db().delete_row(&inserted).await?;
```

Model 的 `Model::as_row` / `Model::from_row` 让两种视图可以互转：Model 是强类型视图，
Row 是动态视图，两者共享同一套 `Db` / `SqlExecutor` 执行路径。

### 6.5 `ChangeSet` —— 变更集

`User::of(pk)` 返回一个**只带主键的引用对象**，无需先 `SELECT` 即可更新 / 删除：

```rust,ignore
User::of(123)
    .set(User::name(), "james")            // 键是 FieldExpr：字段名 + 值类型编译期校验
    .set(User::status(), Status::Paid)
    .update(&db).await?;                   // 只 UPDATE 被 set 过的列

User::of(123).delete(&db).await?;          // 免 SELECT 删除（软删模型自动改写）
```

- SET 目标**只含被 `set` 过的列**，未 set 的列保持数据库原值（不会像整行 `update`
  那样用内存中的旧值覆盖）。
- 同一列多次 `set` 保留最后一次。
- 与整行 `update` 的区别：变更集**不会**自动刷新 `updated_at`（与 Rails `update_columns` 一致）；需要刷新时显式 `.set(User::updated_at(), now)` 或改用整行 `update`。
- 传给 Hook 的行**只含主键 + 本次变更列**（`Row::keys()` 即变更集合），
  `update` 触发 `before_update` / `after_update` 且 Hook 能改写将要写入的值，
  审计 Hook 无需自行 `SELECT` + diff。
- 出错情形（均返回 `DbError::InvalidValue`）：`set` 主键、一次都没 `set` 就调用
  `update`、字段不属于该模型（`field.table() != T::TABLE`）。
- `update` / `delete` 接收 `&impl SqlExecutor`，事务内传 `&tx` 同样可用。

> 旧版 `model.update_columns(&db, &["a", "b"])` 已删除：列名数组无法校验字段与
> 值的类型。改用 `User::of(pk).set(User::a(), v).update(&db)`。

### 6.6 批量写：`insert_many` 与 `RowBatchExt`

**同构批量**（同一模型，列集天然一致）走 `PersistExt::insert_many`：

```rust,ignore
// 同构、分块、单事务；不触发 Hook，也不回填自增主键
User::insert_many(&rows, &db).await?;
```

**异构批量**（各行字段集可不同，如 Excel 导入）走 `RowBatchExt`——
它是泛化于 `SqlExecutor` 的 trait，`Db` 与 `Transaction` 都实现（`prelude` 已导出），
因此在事务里也能批量写且**不另开事务**：

```rust,ignore
use alun_db::prelude::*;

let mut a = Row::new().with_entity("users", "id");
a.insert("name", "张三");
let mut b = Row::new().with_entity("users", "id");
b.insert("name", "李四");
b.insert("status", 1);                                   // 各行字段集可以不同（异构）
let rows = [a, b];

let n: u64 = db().batch_insert("users", &rows).await?;   // 列集相同者合并为多行 VALUES
let n: u64 = db().batch_update("users", &rows).await?;   // 按主键更新各行携带的非主键字段
let n: u64 = tx.batch_insert("users", &rows).await?;     // 事务内：复用外层事务的原子性
```

- 多语句整批包在一个事务里保证原子性（单条语句时直连执行）。
- `batch_insert` 中未赋值的主键列（缺失 / `Null` / 数值 `0`）不参与 INSERT，
  交由数据库生成；表名按标识符白名单校验。
- 两者都不触发 Hook——需要 Hook（如时间戳自动填充）时改用逐行 `insert` / `insert_or_update`；
  `insert_many` 出于同一原因也不触发 Hook，且不逐行回填自增主键。

---

## 七、宏系统

### 7.1 `#[derive(Model)]`

为结构体生成完整的数据库操作能力（A 批默认约定下，最简写法不需要任何结构体级属性）：

```rust,ignore
#[derive(Model)]
pub struct User {                    // 表名自动推导为 `user`
    pub id: i64,                     // 名为 `id` → 主键；整型 id → 默认自增
    #[unique]
    pub email: String,
    pub name: String,
    pub status: i32,
    #[json]
    pub profile: Option<Profile>,
    pub created_at: DateTime<Utc>,   // 含 created_at / updated_at → 自动挂载 TimestampHook
}
```

**生成的代码**：

| 生成项 | 说明 |
|---|---|
| `impl Model for User` | `TABLE` / `PRIMARY_KEY` / `SOFT_DELETE` / `READONLY` / `fields()` / `from_row` / `as_row` / `pk_value` / `hooks` |
| `User::id()` 等 | 每个字段一个表达式访问器，返回 `FieldExpr<T>` |
| Hook 注册 | `#[hook(...)]` 声明的 Hook 以静态列表返回；含 `created_at` / `updated_at` 时自动把 `TimestampHook` 排在最前 |
| `SOFT_DELETE` 常量 | 软删标记列存在时生成（显式 `#[soft_delete]`，或默认的 `Option<...> deleted_at`），否则为 `None` |
| `CrudExt` / `PersistExt` | 不在宏里生成，由 facade 对 `T: Model` 的泛型（blanket）实现提供 |

**属性清单**：

| 属性 | 作用域 | 说明 |
|---|---|---|
| `#[table("name")]` | 结构体 | 指定表名（与 `#[view]` 二选一）；缺省由结构体名蛇形推导 |
| `#[view("name")]` | 结构体 | 指定只读视图名，`READONLY = true` |
| `#[hook(Type)]` | 结构体 | 注册 Hook（可重复声明多个），决定 Hook 在链中的位置 |
| `#[model(strict)]` | 结构体 | 启用编译期 schema 校验（可选 `schema = "…"`） |
| `#[model(hard_delete)]` | 结构体 | 关闭 `deleted_at` 自动软删；与显式 `#[soft_delete]` 同时出现会编译报错 |
| `#[model(no_timestamps)]` | 结构体 | 关闭 `created_at` / `updated_at` 的 `TimestampHook` 自动挂载；时间戳由调用方维护（适合 `created_at` 承载业务时间的模型） |
| `#[key]` | 字段 | 标记主键；缺省回退到名为 `id` 的字段 |
| `#[auto]` | 字段 | 标记自增（只能用于主键）；缺省在「名为 `id` 的整型主键」上开启 |
| `#[unique]` | 字段 | 标记唯一（记入 `FieldMeta::is_unique`） |
| `#[column("name")]` | 字段 | 指定列名（缺省与字段名相同） |
| `#[json]` / `#[json(nullable)]` | 字段 | 标记 JSON 字段 |
| `#[soft_delete]` | 字段 | 显式标记软删除列（至多一个）；缺省在 `Option<...>` 的 `deleted_at` 上开启 |

#### 默认约定（A 批）

| 约定 | 默认行为 | 覆盖方式 |
|---|---|---|
| 表名 | 由结构体名蛇形推导：`SysUser` → `sys_user`、`HTTPLog` → `http_log`、`UserV2` → `user_v2` | `#[table("...")]` |
| 主键 | 未标 `#[key]` 时，名为 `id` 的字段即主键；两者都没有则编译报错 | `#[key]` |
| 自增 | 名为 `id` 且为整型的主键默认自增（等同 `#[auto]`） | 显式 `#[auto]` 声明意图；非 `id` 主键不自增 |
| 时间戳 | 含 `created_at` / `updated_at` 字段时**自动挂载** `TimestampHook`（排在 Hook 链最前），写入时**强制覆盖**这两列的值为当前时间 | 显式 `#[hook(TimestampHook)]` 决定位置；`#[model(no_timestamps)]` 关闭自动挂载（调用方自行维护时间戳） |
| 软删除 | `Option<...>` 的 `deleted_at` 字段**自动作为软删标记**：查询追加 `deleted_at IS NULL`，`delete*` 改写为 `UPDATE SET deleted_at = now` | `#[model(hard_delete)]` 关闭；非 `Option` 的 `deleted_at` 会编译报错并提示 |
| 视图 | `#[view("...")]` 仍显式给出 | —— |

自增语义细化：主键值为**占位值**（`Value::Null` / 数值 `0`）时才排除该列交由数据库生成；
显式非零主键（雪花 ID 等）照常写入。`insert_many` 仅在**所有行**主键都未赋值时排除该列。

> `TimestampHook` / `now_value` 与 Hook 洋葱执行器已从 `alun-db` 迁到
> **`alun_db_core::hook`**（`alun_db::hook` 模块已删除）。`#[hook(...)]` 里写
> `TimestampHook`（prelude 已导出）或全路径 `alun_db_core::hook::TimestampHook`。

> `#[derive(Model)]` 只支持结构体；枚举用 `#[derive(TypeMapper)]` 接入（见 4.5）。

### 7.2 `#[derive(Row)]`

为嵌套结构生成与 `Row` 的双向转换：

```rust,ignore
#[derive(Row, Serialize, Deserialize)]
pub struct Profile {
    pub avatar: Option<String>,
    pub bio: String,
}
```

与 `Model` 的区别：Row 派生不要求主键、不生成字段表达式、不注册 Hook。它用于 `#[json]` 嵌套结构。

### 7.3 `#[derive(TypeMapper)]`

为枚举生成 `TypeMapper` 实现，支持四种编码模式（见 [4.5 节](../docs/guides/db/appendix.md)）。

### 7.4 `sql!` 宏

编译期校验的 SQL 片段拼接，产物是 `SqlQuery`（配合 `SqlQueryExt` 终态动词返回
动态 `Row`，或经 `User::sql(...)` 返回强类型模型）：

```rust,ignore
let users: Vec<Row> = sql!(
    "SELECT * FROM users WHERE 1=1",
    if let Some(n) = &name_filter {
        " AND name LIKE " { n }
    },
    if let Some(s) = status_filter {
        " AND status = " { s }
    },
    " #orderBy(created desc) #limit(20)",
)
.find(db())
.await?;

// 字面量内的条件块：#if + #para（见 6.3）
let rows = sql!(
    "SELECT * FROM invoice WHERE tenant_id = ", { tid },
    " #if(status_filter) AND status = #para(status_filter) #end",
).find(db()).await?;
```

`sql!` 的编译期校验针对**模板结构**：`#` 指令的列名/操作符/变量名/整数走白名单
校验，插值 `{ expr }` 一律参数化为占位符（不做字符串拼接），裸表达式片段只接受
`SqlFragment`（如嵌套的 `sql!` 结果）。它不校验表结构——schema 校验由
`#[model(strict)]` 负责（见 10.3）。

---

## 八、SQL 模板与渲染器

### 8.1 表达式树（Expr）

```rust,ignore
pub enum Expr {
    Column { table: &'static str, column: &'static str },
    Value(Value),
    Binary { left: Box<Expr>, op: BinaryOp, right: Box<Expr> },
    Unary { op: UnaryOp, expr: Box<Expr> },
    Call { func: Function, args: Vec<Expr> },
    JsonGet { expr: Box<Expr>, path: String },
    And(Vec<Expr>),
    Or(Vec<Expr>),
    AlwaysTrue,
    Raw { sql: String, binds: Vec<Expr> },
}
```

**不变量**：

1. `And` / `Or` 非空；空时使用 `AlwaysTrue`
2. `Raw` 是唯一的 SQL 拼接入口，用户显式构造即承担注入风险
3. `Like` / `NotLike` 的右操作数必须是 `Value(Text(_))`
4. `In` 的右操作数必须是 `Value(Array(_))`

### 8.2 语句 AST（Statement）

```rust,ignore
pub enum Statement {
    Select(SelectStmt),
    Insert(InsertStmt),
    Update(UpdateStmt),
    Delete(DeleteStmt),
    Raw(RawStmt),  // 已渲染的原始 SQL
}
```

### 8.3 渲染器

每个方言实现 `Dialect` trait，负责将 AST 渲染为方言特定的 SQL：

```rust,ignore
pub trait Dialect: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn capability(&self) -> &DialectCapability;
    fn render_type(&self, ty: &Type) -> String;
    fn render_expr(&self, expr: &Expr, ctx: &mut RenderCtx<'_>) -> String;
    fn render_statement(&self, stmt: &Statement) -> RenderedSql;
    fn map_error(&self, raw: RawDbError) -> DbError;
    fn render_now(&self) -> String;
    fn quote_identifier(&self, name: &str) -> String;
    // 带默认实现：内联渲染 `LIMIT n OFFSET m`（字面量，非绑定参数）
    fn push_limit_offset(&self, limit: Option<u64>, offset: Option<u64>, out: &mut String);
}
```

**渲染上下文（RenderCtx）** 在渲染过程中追踪参数索引与绑定值列表：

```rust,ignore
pub struct RenderCtx<'a> {
    pub params: Vec<Value>,
    pub next_index: usize,
    pub dialect: &'a dyn Dialect,
}
```

### 8.4 占位符差异处理

PostgreSQL 使用 `$1, $2`，MySQL / SQLite 使用 `?`。渲染时由 `RenderCtx::bind` 统一处理：

```rust,ignore
impl<'a> RenderCtx<'a> {
    pub fn bind(&mut self, value: Value) -> String {
        let idx = self.next_index;
        self.params.push(value);
        self.next_index += 1;
        match self.dialect.capability().placeholder {
            Placeholder::Question => "?".to_string(),
            Placeholder::Dollar => format!("${idx}"),
        }
    }
}
```

---

## 九、事务与 Hook

### 9.1 事务设计

alun-db 的事务采用 **RAII 模式**，这是相对 Java 方案的核心优势：

```rust,ignore
// 闭包式（推荐）
db().transaction(|tx| async move {
    tx.execute("INSERT INTO users ...").await?;
    tx.execute("INSERT INTO orders ...").await?;
    Ok(())
}).await?;
```

**设计决策**：`Transaction` 本身**不实现 `Drop`** —— Rust 不允许在 `Drop::drop`
中 `await`，把异步回滚塞进 `Drop` 会引入隐藏的运行时依赖。回滚由**驱动事务句柄
的 `Drop` 完成**：忘记调用 `commit` / `rollback`（含 panic 展开）时，事务在句柄
销毁时被驱动自动回滚，无需异步等待。`commit` / `rollback` 均可安全重复调用
（幂等，第一次生效，后续为无操作）。

**显式事务对象**用于跨函数边界的场景：

```rust,ignore
let tx = db().begin().await?;
do_something(&tx).await?;
do_something_else(&tx).await?;
tx.commit().await?;
```

> 事务从池中借出**专属连接**：事务内的语句必须走 `tx` 句柄；通过 `db()` 执行的
> 语句走其他连接，不受该事务保护。

### 9.2 执行上下文统一

SQL 执行层由 `SqlExecutor` trait 抽象，`Db` 与 `Transaction` 都实现它
（`sql_dialect` / `exec_sql` / `query_sql` / `scalar_sql` / `paginate_sql` / `in_tx` …）。
**几乎所有执行入口都泛化于 `SqlExecutor`**，因此同一段代码既能在 `db()` 上跑，
也能在 `&tx` 上跑：

| 入口 | 事务内用法 |
|---|---|
| `sql!` + `SqlQueryExt`（`find` / `find_first` / `find_one` / `find_one_or_null` / `exists` / `paginate` / `for_each` / `for_each_page` / `scalar` / `execute`） | `sql!("...").find(&tx).await?` |
| `ModelQuery` 终态动词 | `User::query().find(&tx).await?` |
| `CrudExt` 关联函数（`find` / `find_by` / `find_first_by` / `count_by` / `delete_by` / …） | `User::find_by(&tx, "status", (1,)).await?` |
| `PersistExt` 实例方法（`insert` / `insert_or_update` / `update` / `delete` / `insert_many`） | `user.insert(&tx).await?` |
| `ChangeSet`（`update` / `delete`） | `User::of(1).set(User::name(), "李四").update(&tx).await?` |
| Builder（`FindBuilder::exec` / `first`、`PageBuilder::exec`、`CountBuilder::exec`） | `db().query().table("users").exec(&tx).await?` |
| `RowBatchExt`（`batch_insert` / `batch_update`） | `tx.batch_insert("users", &rows).await?` |

```rust,ignore
sql!("UPDATE users SET status = ", { 9i32 }, " WHERE email = ", { email })
    .execute(&tx).await?;                    // 事务内
sql!("SELECT status FROM users WHERE email = ", { email })
    .find_one(&db).await?;                   // 事务外
```

> 事务从池中借出**专属连接**，事务内的写操作必须走 `tx` 句柄才能被同一事务保护；
> `SqlExecutor::in_tx` 在 `Db` 上开启事务、在 `Transaction` 上内联（框架不支持嵌套事务）。

### 9.3 Hook 设计

Hook 是 alun-db 的差异化能力，用于审计日志、时间戳自动填充、软删除等横切关注点：

```rust,ignore
// Result 为 alun_db_core::error::Result（即 Result<T, DbError>）
pub trait Hook: Send + Sync + 'static {
    fn before_insert(&self, _row: &mut Row) -> Result<()> { Ok(()) }
    fn after_insert(&self, _row: &Row) -> Result<()> { Ok(()) }
    fn before_update(&self, _row: &mut Row) -> Result<()> { Ok(()) }
    fn after_update(&self, _row: &Row) -> Result<()> { Ok(()) }
    fn before_delete(&self, _pk: &Value) -> Result<()> { Ok(()) }
    fn after_delete(&self, _pk: &Value) -> Result<()> { Ok(()) }
}
```

Hook 方法是**同步**的（不是 `async fn`）：Hook 只做行数据的改写与校验，不做 IO；
需要发消息、写远端日志这类异步副作用时，应在业务层完成。

**执行层级**：Hook 在 **Model 层**拦截，覆盖 `PersistExt`（`insert` /
`insert_or_update` / `update` / `delete`）、`ChangeSet`（`update` / `delete`）
与静态按主键删除 `CrudExt::delete_by_pk`。条件删除
（`delete_by` / `delete_in` / `delete_in_ids`）与批量入口
（`insert_many` / `RowBatchExt`）**不触发** Hook；绕过 Model 直接写
SQL（`Db::execute*` / `sql!`）或用动态 `Row` API（`insert_row` 等）时 Hook
同样不触发——这是设计决策，不是缺陷。

**执行顺序**：多个 Hook 按 `#[hook(...)]` 声明顺序执行：`before_*` 正序，`after_*`
逆序（洋葱模型）。任何 `before_*` 返回 `Err` 则中断操作并把错误原样传回调用方
（若该操作位于事务内，则由事务回滚）。

### 9.4 内置 Hook

**TimestampHook**：自动填充 `created_at` 和 `updated_at`。它是单元结构体，
**没有带参构造器**，列名固定为 `created_at` / `updated_at`，只对 Row 中已存在的
键生效（模型可按需裁剪列集）。

按 A 批默认约定，模型只要含 `created_at` / `updated_at` 字段就**自动挂载**
`TimestampHook`（且排在 Hook 链最前），无需手写 `#[hook(...)]`：

```rust,ignore
#[derive(Model)]
pub struct User {                // 表名自动推导为 `user`
    id: i64,                     // 主键 + 自增
    name: String,
    created_at: DateTime<Utc>,   // 插入时自动填充
    updated_at: DateTime<Utc>,   // 插入和更新时自动填充
}
```

> 需要让时间戳 Hook 排在用户 Hook 之后时，显式声明 `#[hook(TimestampHook)]`
> 决定其位置（不会重复挂载）。Hook 路径写全时可用
> `#[hook(alun_db_core::hook::TimestampHook)]`；`use alun_db::prelude::*;` 时
> `#[hook(TimestampHook)]` 亦可。`TimestampHook` / `now_value` 与 Hook 洋葱
> 执行器都位于 `alun_db_core::hook`（`alun_db::hook` 模块已删除）。

**软删除**：`Option<...>` 类型的 `deleted_at` 字段**自动成为软删标记**——
`delete` 族改为 `update(deleted_at = now)`，查询族自动追加 `deleted_at IS NULL`
过滤（显式 `#[soft_delete]` 亦可，至多一个标记列）：

```rust,ignore
#[derive(Model)]
pub struct Article {             // 表名自动推导为 `article`
    id: i64,
    title: String,
    deleted_at: Option<DateTime<Utc>>,   // 自动软删标记
}

article.delete(&db).await?;              // 改写为 UPDATE article SET deleted_at = now WHERE id = ?
Article::query().find(&db).await?;       // SELECT * FROM article WHERE deleted_at IS NULL
Article::of(1).delete(&db).await?;       // 免 SELECT 标记删除
```

- 覆盖入口——改写 UPDATE：`PersistExt::delete`、`CrudExt::delete_by_pk` /
  `delete_by` / `delete_in` / `delete_in_ids`、`ChangeSet::delete`；
  追加 NULL 过滤：`CrudExt` 的 `find` / `find_or_none` / `find_by` /
  `find_first_by` / `find_in` / `find_in_ids` / `count` / `count_by`，
  以及 `ModelQuery` 的 `find` / `find_first` / `find_one` /
  `find_one_or_null` / `exists` / `paginate` / `for_each` / `for_each_page`
- 原始记录保留在表中，仅置删除时间戳；软删标记列至多一个
- 关闭方式：`#[model(hard_delete)]`（此时 `delete` 族发出真正的 DELETE）；
  非 `Option` 的 `deleted_at` 会被当作误配并**编译报错**，提示加
  `#[model(hard_delete)]` 或 `#[column("...")]` 改名
- 例外：`User::sql(sql!(...))` 属于原始 SQL 直通，**不会**自动追加软删过滤，
  需要自己写 `WHERE deleted_at IS NULL`

---

## 十、迁移系统

> **已实现。** 进程内迁移执行器 `Migrator`（`alun_db::prelude::Migrator`）：
> - 代码内版本清单：`Migrator::from_migrations(vec![Migration::new(…)]).run(&db)`
> - 目录扫描（可多来源）：`Migrator::new().source_dir(a).source_dir(b).run(&db)`，
>   或单目录 `Migrator::from_dir(&dir).run(&db)`
> - 回退：`Migrator::from_dir(&dir).rollback(&db)`（LIFO 回退最近一次）
> - 观察 / 运维：`status` / `plan` / `baseline` / `repair` / `dry_run`
>
> 执行器自动维护 `schema_migrations` 表并逐条事务化应用；`#[view]` 只读视图模型
> 与 `#[model(strict)]` 编译期列名/类型校验；以及 **build.rs schema 快照**
> （`alun-db-build` crate，见 10.3）。独立的 `alun-db-migrate` 执行器 crate 仍未
> 单列（执行器内置在 facade `alun-db` 中）。
>
> `Db` 上原有的 `migrate` / `migrate_down` / `migrate_from_dir` / `migrate_down_from_dir`
> 方法与同名自由函数**已移除**；`Migration` 现为拥有型（`Version` + `String`），
> 不再是 `Copy` 常量，`Migration::new` 也不再是 `const fn`。

### 10.1 迁移文件格式

alun-db 采用**纯 SQL 迁移文件**，不发明 DSL。两种消费方各有一套文件命名约定：

**（a）运行时目录 runner**（`Migrator::from_dir`）——文件名编码版本号与方向，
文件内容就是该方向的 SQL（不写分段标记）：

```text
migrations/
  V1__create_users.sql
  V2__add_users_age.up.sql
  V2__add_users_age.down.sql
```

除 `V{n}__Name` 外还支持真实项目常见的风格（同一目录可混用，无需改名）：

```text
R__active_user_view.sql        # 可重复：内容变化即重跑（视图 / 函数）
S__dict_data.sql               # 种子：幂等重灌的初始化数据
B2__baseline.sql               # 基线标记：声明 <= 2 视为已应用（内含语句会报错）
01_base_tables_init.sql        # 纯数字前缀 + 单下划线（jinhu 风格）
31_storage_backend.up.sql      # 数字前缀 + .up/.down（facai 风格，允许 up-only）
system/V0002__create_sys_role.up.sql   # 子目录按业务域分（递归扫描）
```

三个种类前缀都可改（`MigratorOptions` / `[database.migration]`）：
`repeatable_prefix`（默认 `R`）、`seed_prefix`（默认 `S`）、`baseline_prefix`（默认 `B`）。
设为**空串即关闭该种类**；三者必须互不相同且不以数字开头，否则加载前报错。

```text
Fn__active_user_view.sql       # repeatable_prefix = "Fn"
Dict__sys_dict.sql             # seed_prefix = "Dict"
Base2__baseline.sql            # baseline_prefix = "Base"
```

- 文件名须匹配一种受支持的风格（`V{n}__Name.sql`、`{n}_Name.up.sql`、`{n}_Name.sql` 等），
  前缀与分隔符可用 `version_prefix` / `name_separator` 收紧
- `version` 全局唯一，重复版本报错；非 `.sql` 文件（如 `README.md`）被跳过
- 每个文件**可以写多条** SQL 语句：按方言切分后在事务内逐条执行
  （`-- +no-transaction` 可让该迁移跳出事务）

**（b）build.rs 快照目录**（`alun-db-build`）——按文件名升序读取，约定
`YYYYMMDD_HHMMSS_描述.sql`，文件内用分段标记区分方向：

```text
migrations/
  20260921_000001_create_users.sql
  20260921_000002_create_orders.sql
```

```sql
-- +migrate Up
CREATE TABLE users (
    id       INTEGER PRIMARY KEY AUTOINCREMENT,
    email    TEXT NOT NULL UNIQUE,
    name     TEXT NOT NULL,
    status   INTEGER NOT NULL DEFAULT 1,
    created  TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- +migrate Down
DROP TABLE users;
```

> build.rs 只抽取 `-- +migrate Up` 段，合并 `CREATE TABLE` 与
> `ALTER TABLE ... ADD COLUMN` 的增量演进，用于生成编译期 schema 快照；
> 运行时迁移则由 (a) 的目录 runner 或代码内 `Migration` 清单驱动（见 10.2）。

### 10.2 迁移执行

```rust,ignore
// 程序启动时：构造拥有型迁移清单（SQL 为 String；每条可含多条语句）
let migrations = vec![
    Migration::new(
        1,
        "create_users",
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
        "DROP TABLE users",
    ),
    Migration::new(
        2,
        "add_users_age",
        "ALTER TABLE users ADD COLUMN age INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE users DROP COLUMN age",
    ),
];
Migrator::from_migrations(migrations).run(&db).await?;

// 或者：扫描目录中的 V{n}__Name.sql（见 10.1(a)）
Migrator::from_dir(Path::new("migrations")).run(&db).await?;

// 多来源：同一执行器叠加多个目录（版本号跨目录全局唯一）
Migrator::new()
    .source_dir("migrations/system")
    .source_dir("migrations/support")
    .run(&db)
    .await?;

// 回退最近一次应用的迁移（LIFO），返回 Option<Version>
let rolled: Option<Version> = Migrator::from_dir(Path::new("migrations")).rollback(&db).await?;

// 运维入口
let pending = Migrator::from_dir(Path::new("migrations")).plan(&db).await?;   // 本次将应用哪些
let report = Migrator::from_dir(Path::new("migrations")).dry_run(&db).await?; // 预演（执行后回滚，不落库）
Migrator::from_dir(Path::new("migrations")).baseline(&db, Version::new("3")).await?; // 存量库跳过 <=3
Migrator::from_dir(Path::new("migrations")).repair(&db).await?;              // 修指纹 / 清理失效记录
// 开发期重建（须 MigratorOptions::is_destructive_allowed = true；启动路径恒为 false）
Migrator::from_dir(Path::new("migrations")).reset(&db).await?;             // 逆序回滚全部
Migrator::from_dir(Path::new("migrations")).fresh(&db).await?;             // reset 后重建

// 编译期嵌入：迁移目录被 include_str! 编入二进制，运行时不再依赖磁盘 SQL
let migrator = alun_db::migrate!("migrations");
```

- 执行顺序为 **基线 → 版本化（版本升序）→ 可重复 `R__` → 种子 `S__`（按名称升序）**
- 按 `Version` 升序应用未应用的迁移，每个迁移在独立事务中执行
- 自动创建并维护 `schema_migrations` 表（`(kind, name)` 为主键，另含
  `version` / `checksum` / `applied_via` / `applied_at` / `applied_seq` / `success`
  等列，`applied_at` 为 epoch 秒）
- 占位符替换：`MigratorOptions::placeholders` **非空时**才替换脚本中的 `${key}`，
  空表完全不扫描；替换在分段 / 拆分前完成，故指纹覆盖替换后的 SQL
- 重复版本或任一迁移失败均返回 `DbError`；失败迁移事务回滚且不记录
- 回退走 `Migrator::rollback`（按 `applied_seq` LIFO 回退最近一次）
- 版本化迁移的内容被改写 → `DbError::MigrationChecksum`（策略见 `MigratorOptions::divergent`）；
  可重复 / 种子迁移的内容变化是**预期行为**，触发重跑

CLI 形态（规划中）：

```text
alun migrate up
alun migrate down
alun migrate status
```

### 10.3 Schema 快照

在编译期，应用 crate 的 `build.rs` 调用 `alun_db_build::build_snapshot("migrations")`
解析所有迁移文件，合并 `CREATE TABLE` 与 `ALTER TABLE ... ADD COLUMN` 的增量演进，
写出一份规范化 DDL 快照（`$OUT_DIR/schema_snapshot.sql`），并注入
环境变量 `ALUN_DB_SCHEMA_SNAPSHOT` 与该目录的 `cargo:rerun-if-changed`。

```toml
# Cargo.toml
[build-dependencies]
alun-db-build = { path = "…/alun-db-build" }
```

```rust,ignore
// build.rs
fn main() -> Result<(), Box<dyn std::error::Error>> {
    alun_db_build::build_snapshot("migrations")?; // 生成快照并注入 env
    Ok(())
}
```

`#[model(strict)]` 展开时（未显式给 `schema = "…"` 时读环境变量指向的快照），
做**两层编译期校验**：

1. **列名存在性**：模型字段列确实存在于对应 `CREATE TABLE`（主键列也单独校验）；
2. **列类型兼容**：Rust 字段类型族与数据库列类型族匹配——`i64` 不应映射
   `TEXT` 列。类型族为方言无关的粗粒度归类（`int` / `text` / `bool` / `float` /
   `decimal` / `bytes` / `date` / `time` / `timestamp` / `timestamptz` / `uuid` /
   `ulid` / `json` / `inet` / `cidr`），兼容矩阵对跨方言合法实践放宽：
   SQLite 以 `INTEGER` 存布尔、以 `TEXT` 存 UUID/JSON，MySQL 以 `TINYINT(1)`
   存布尔均不误报；`VARCHAR(255)`、`DECIMAL(10,2)`、`TIMESTAMP WITH TIME ZONE`
   等带参数/多词类型自动归一化。未知或无法归类的类型（自定义枚举、
   `#[json]` 嵌套结构等）自动跳过类型比对，只保留列名校验。

拼错列名或类型漂移不是运行时 panic，而是带定位的编译错误——这是 alun-db
相对 aifei 与 Toasty 的核心增强：aifei 是运行时反射，Toasty 是结构校验，
alun-db 是 **schema 级编译期校验**。

```rust,ignore
#[derive(Model)]
#[table("users")]
#[model(strict)] // 未给 schema 时自动消费 build.rs 快照
struct User {
    #[key] id: i64,
    name: String,
    age: i64, // 来自 0002 迁移的 ALTER TABLE ADD COLUMN，快照已合并
}
```

```text
error: strict schema mismatch: column `qty` is `text` in Rust but `int` in table `widgets`
  --> src/models.rs:9:9
   |
 9 | pub struct Widget {
   |            ^^^^^^
```

---

## 十一、库亮点

### 亮点 1：aifei 调用风格 + Rust 类型安全

**问题**：aifei 的调用风格便捷，但 Java 的运行时反射无法编译期校验；Toasty 类型安全，但查询构建器的宏样板仍多。

**alun-db 方案**：以 Model 关联函数为统一入口（`User::find`、`User::query().paginate`），通过 `FieldExpr<T>` 和 `FieldValue<T>` 保证编译期类型安全。

```rust,ignore
User::age().eq(18)        // ✅
User::age().eq("18")      // ❌ 编译失败
```

### 亮点 2：三层类型解耦

Rust 类型 → 中间类型 → 数据库类型的三层映射，使得：
- Rust 类型↔中间类型映射固定一次定义
- 中间类型↔数据库类型由每个方言独立决定
- 新增数据库只需实现 `Dialect::render_type`

### 亮点 3：Capability 协商机制

驱动显式声明支持的能力，避免"某方言悄悄不支持但看起来能用"。可降级场景（如
`ILIKE` → `LOWER(...) LIKE LOWER(...)`）由渲染器自动降级并保留语义；不可降级
场景在调用点返回 `DbError::UnsupportedFeature`，由调用方明确处理（见 5.3）。

### 亮点 4：编译期 Schema 校验（**已实现**：`#[model(strict)]` + build.rs 快照）

`build.rs` 解析迁移文件生成 schema 快照（见 10.3），`#[model(strict)]` 展开时
校验列名存在性与列类型族兼容。拼错列名不是运行时 panic，而是带定位的编译错误：

```text
error: strict schema mismatch: column `emial` is not declared in `users`
  --> src/models.rs:9:9
   |
 9 |     pub emial: String,
   |              ^^^^^
```

> **规划中。** 拼错列名时的 `did you mean ...` 智能修正建议尚未实现
> （见 13 Phase 4），当前仅给出明确报错。

### 亮点 5：生态类型全支持

chrono / time / jiff / rust_decimal / bigdecimal / uuid / ulid / serde_json / ipnetwork 通过 feature 逐项启用。**核心层只依赖 `thiserror`**，不默认拖入任何生态类型或驱动。

### 亮点 6：JSON 与嵌套结构透明映射

`#[json]` 属性让字段在 `as_row` / `from_row` 时经 `serde_json` 与 `Value::Json`
互转（`#[json(nullable)]` 对应可空列），用户代码中 `user.profile.bio` 就是普通
字段访问。方言渲染时自动选择 `JSONB` / `JSON` / `TEXT`。

### 亮点 7：SQL 模板 + 链式 Builder 双风格

- **aifei `#` 指令**：`#where(status, '=', status)` 动态条件（`sql!` 宏，见 6.3）
- **链式查询脊柱**：`ModelQuery`（`User::query().filter(...).order_by(...)`）强类型组装，
  `User::status().eq(1)` 类型安全；动态 `Row` 侧用 `FindBuilder`
- **终态动词统一**：`find` / `find_first` / `find_one` / `exists` / `paginate` /
  `for_each` / `for_each_page` / `scalar` / `execute`——链式 API 只负责选择执行方式，
  语义一律由 SQL 表达

三种写法共享同一套 `Expr` 表达式树与渲染路径，避免双轨制的维护负担。

> 链式条件入口就是 `ModelQuery`（`User::query().filter(...)`，见 6.2）；
> 静态糖 `User::find_by` / `find_first_by` 覆盖单条件场景，两者共用同一份渲染实现。

### 亮点 8：Hook 生命周期 + RAII 事务

- **Hook**：before/after 拦截 CRUD，支持审计、时间戳、软删除
- **RAII 事务**：`db().transaction(|tx| async { ... })` 闭包式，返回 `Err` 自动回滚

### 亮点 9：RPITIT 异步零分配

`SqlExecutor` / `Dialect` 等 trait 使用 **RPITIT**（`-> impl Future<Output = _> + Send`）
返回 future，而不是 `#[async_trait]` 的 `Box<dyn Future>`——没有堆分配，future
类型由编译器推导；这也是 `CrudExt` / `PersistExt` 的关联函数都能直接跨线程
（`tokio::spawn` / axum handler）的原因。数据库层没有引入 `async-trait`；各驱动
自行封装底层约束（例如 SQLite 底层的 `rusqlite::Connection` 不是 `Sync`，
由驱动封装成可安全执行的形式）。

### 亮点 10：AI 友好

| 维度 | 说明 | 状态 |
|---|---|---|
| 低 Token | 日常 CRUD 一行表达，无宏样板 | 已实现 |
| 可预测 | 所有操作从 Model 或 Db 出发 | 已实现 |
| 编译期反馈 | `#[model(strict)]` 让列名/类型漂移在 `cargo build` 阶段暴露 | 已实现 |
| 修正建议 | 拼错列名时提示 `did you mean ...` | 规划中 |

---

## 十二、使用示例

### 12.1 入门：CRUD

```rust,ignore
// 需开启 `alun-db` 的 `chrono` feature
use alun_db::prelude::*;
use chrono::{DateTime, Utc};

#[derive(Model, Debug, Clone)]
pub struct User {                    // 表名自动推导为 `user`
    pub id: i64,                     // `id` → 主键；整型 → 自增
    #[unique]
    pub email: String,
    pub name: String,
    pub status: i32,
    pub created_at: DateTime<Utc>,   // 自动挂载 TimestampHook
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化
    alun_db::init("sqlite://./dev.db").await?;

    // 创建：insert 是纯 INSERT；主键为占位值（0）时由数据库生成并回填
    let mut user = User {
        id: 0,
        email: "alice@example.com".into(),
        name: "Alice".into(),
        status: 1,
        created_at: Utc::now(),
    };
    user.insert(db()).await?;
    println!("Created user with id: {}", user.id);

    // 查询（不存在 → DbError::NotFound；可空版本是 find_or_none）
    let fetched = User::find(db(), user.id).await?;
    println!("Found: {}", fetched.name);

    // 更新（整行）
    user.name = "Alice Updated".into();
    user.update(db()).await?;

    // 只改一列：变更集，无需先 SELECT
    User::of(user.id).set(User::name(), "Alice Again").update(db()).await?;

    // 分页：ModelQuery 终态动词
    let page: Page<User> = User::query().paginate(db(), 1, 20).await?;
    println!("Total: {}, Pages: {}", page.total, page.pages);

    // 删除（软删模型自动改写为标记删除）
    user.delete(db()).await?;
    Ok(())
}
```

### 12.2 进阶：事务 + Hook

```rust,ignore
// 需开启 `alun-db` 的 `rust_decimal`、`chrono` feature
#[derive(Model)]
#[hook(TimestampHook)]   // 显式声明只为固定 Hook 位置；含 created_at/updated_at 时本会自动挂载
pub struct Order {                // 表名自动推导为 `order`
    id: i64,                      // 主键 + 自增
    user_id: i64,   // 外键直接声明为普通列；#[belongs_to] 关系属性规划中
    amount: Decimal,
    status: OrderStatus,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// 单条写入：走 Model API，Hook（时间戳）与自增主键回填自动生效。
pub async fn create_order(user_id: i64, amount: Decimal) -> Result<Order> {
    let mut order = Order {
        id: 0,
        user_id,
        amount,
        status: OrderStatus::Pending,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    order.insert(db()).await?;   // 纯 INSERT，id 回填，before/after_insert 被调用
    Ok(order)
}

/// 跨表一致性：事务内 Model API 与 `sql!` 都能用——执行入口统一泛化于 `SqlExecutor`，
/// 把 `&tx` 传进去即可（见 9.2）。
pub async fn place_order(user_id: i64, amount: Decimal, sku: &str) -> Result<()> {
    db().transaction(|tx| async move {
        // Model API：Hook（时间戳）与自增主键回填在事务内同样生效
        let mut order = Order {
            id: 0,
            user_id,
            amount,
            status: OrderStatus::Pending,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        order.insert(&tx).await?;

        // 原始 SQL：`sql!` 的终态动词同样接受 `&tx`
        sql!("UPDATE stock SET qty = qty - ", { 1i64 }, " WHERE sku = ", { sku })
            .execute(&tx)
            .await?;

        Ok(())   // 返回 Err 则整体回滚
    })
    .await
}
```

### 12.3 高级：JSON 路径查询

> **规划中。** `FieldExpr::json_get` 字段级入口尚未开放（表达式树已具备
> `Expr::JsonGet` 节点），当前以 `find_by` + 方言 SQL 片段替代。
> 以下为目标写法：

```rust,ignore
// 需开启 `alun-db` 的 `json`、`chrono` feature
#[derive(Model)]
pub struct Event {                // 表名自动推导为 `event`
    id: i64,
    #[json] payload: EventPayload,
    created_at: DateTime<Utc>,
}

// 查询 payload.channel = "alipay" 的事件（目标写法，待实现）
let events = Event::query()
    .filter(Event::payload().json_get("channel").eq("alipay"))
    .find(db()).await?;

// 自动适配方言：
// PostgreSQL: payload->>'channel' = $1
// MySQL:      JSON_UNQUOTE(JSON_EXTRACT(payload, '$.channel')) = ?
// SQLite:     json_extract(payload, '$.channel') = ?
```

### 12.4 AI Coding 场景

给 AI 一句需求：

> "查一下 2026 年之后注册、状态为启用、名字包含'张'的用户，按注册时间倒序，取前 20 条"

AI 生成：

```rust,ignore
// 多参数 → 整段按 WHERE 片段处理（片段内自带占位符，可带 ORDER BY / LIMIT）
let users = User::find_by(
    db(),
    "created_at >= ? AND status = ? AND name LIKE ? ORDER BY created_at DESC LIMIT 20",
    (since, 1, "%张%"),
).await?;

// 也可以交给强类型链式：列名与值类型在编译期校验
let users = User::query()
    .filter(User::created_at().ge(since))
    .filter(User::status().eq(1))
    .filter(User::name().like("%张%"))
    .order_by(User::created_at().desc())
    .limit(20)
    .find(db()).await?;
```

**AI 只需生成 SQL 片段 + 少量 Rust 控制流**，不需要学习 `#where`、`#orderBy`
这类非标准指令。`find_by` 的 SQL 片段直接拼在 `WHERE` 之后，占位符按位置
与 `Params` 元组（0~12 元）对应，元素类型只需 `Into<Value>`；单参数且是合法
标识符时则按「字段 = ?」处理（`where_or_field` 双态签名，见 6.1）；配合
`#[model(strict)]` 的编译期列名/类型校验（见 10.3），拼错的列名在 `cargo build`
阶段就会暴露。

---

## 十三、实施路线

### Phase 1：核心可用（约 10 周）

| 里程碑 | 内容 | 工期 |
|---|---|---|
| M1 | `alun-db-core` 核心类型 | 2 周 |
| M2 | SQLite 驱动 + 基础查询 | 2 周 |
| M3 | 宏系统（derive Model/Row/TypeMapper） | 2 周 |
| M4 | `#` 指令与 `sql!` | 1 周 |
| M5 | 迁移系统 | 1 周 |
| M6 | Hook 与事务 | 1 周 |
| M7 | 文档与示例 | 1 周 |

### Phase 2：多后端

- PostgreSQL 与 MySQL 驱动（**已实现**）
- Capability 协商（**已实现**）
- `#[view]` 只读视图模型（**已实现**：视图可查询、写操作返回 `Readonly`）
- `#[model(strict)]` 编译期列名与类型校验（**已实现**）
- 进程内迁移 runner `Db::migrate` / `Migration`（**已实现**）
- JSON 路径查询（规划中：`Expr::JsonGet` 节点已就绪，用户链式入口未开放）
- 连接池调优（**已实现**：`PoolConfig` 可配置池化连接）

### Phase 3：生态融合

- 生态类型全量支持（time / jiff / bigdecimal / ulid / ipnetwork）（**已实现**）
- `build.rs` schema 快照（**已实现**：`alun-db-build`，见 10.3）
- CLI 工具（规划中：`alun migrate up/down/status` 尚未落地）
- alun 框架集成（规划中）

### Phase 4：智能化

- `sql_template!()` 对 AI 生成 SQL 的编译期校验（规划中）
- 拼错列名时的智能修正建议（规划中：当前仅报错，无 `did you mean`）
- 编译错误消息针对 AI 生成场景优化（规划中）

---

## 十四、附录：Crate 清单

| Crate | 职责 | 依赖 |
|---|---|---|
| `alun-db` | facade，用户唯一直接依赖 | core, sql, macro, driver-* |
| `alun-db-core` | trait、AST、错误、Page、Row | thiserror（+ 可选生态类型 feature） |
| `alun-db-macro` | proc-macro：derive(Model)、derive(Row)、derive(TypeMapper)、sql! | syn, quote, proc-macro2 |
| `alun-db-sql` | SQL 渲染器、方言 | core |
| `alun-db-migrate` | 迁移执行器（规划中，尚未建立；runner 当前内置在 facade） | sql |
| `alun-db-build` | build.rs 辅助：迁移文件→schema 快照 | 零依赖（std） |
| `alun-db-driver-sqlite` | SQLite 驱动 | core, sql, rusqlite |
| `alun-db-driver-postgres` | PostgreSQL 驱动 | core, sql, tokio-postgres, bytes |
| `alun-db-driver-mysql` | MySQL 驱动 | core, sql, mysql_async |

facade `alun-db` 的默认 feature 为 `["sqlite", "async"]`；`postgres` / `mysql`
与生态类型（`chrono` / `time` / `jiff` / `rust_decimal` / `bigdecimal` / `uuid` /
`ulid` / `json` / `ipnetwork`）逐个按需开启。

---

## 与现有框架对比

| 维度 | aifei-db | Toasty | SeaORM | alun-db |
|---|---|---|---|---|
| 查询入口 | `sql()` 统一链式 | 编译期查询构建器 | `Entity::find()` Builder | `Model::find` + `ModelQuery` 终态动词 |
| SQL 模板 | `#where` 运行时解析 | 无 | 无 | `#where` 编译期校验 |
| 类型映射 | 运行时反射 | stmt::Type 双层 | ColumnType 枚举 | stmt::Type 三层 + 方言 |
| 编译期校验 | 无 | 结构级 | 无 | schema 级 + 类型级 |
| 事务 | 显式控制 | 闭包 | 闭包 | 闭包 + RAII |
| Hook | 支持 | 无 | 无 | 内置 `TimestampHook` + `deleted_at` 自动软删 |
| 多数据库 | JDBC 统一 | Capability 协商 | 内置多驱动后端 | Capability 协商 + 渲染期降级/报错 |
| 生态类型 | Java 标准 | chrono/uuid | chrono/time/decimal/uuid/json | 同 SeaORM + ulid + ipnetwork |
| AI 友好度 | 极优 | 良好 | 中等 | 极优（统一入口 + 编译期反馈） |

---

## 总结

alun-db 的设计可以概括为一句话：

> **以 Model 关联函数为统一入口继承 aifei 的调用风格，以三层类型映射和编译期 schema 校验继承 Toasty 的类型安全，以方言渲染器和 Capability 协商处理多数据库差异，以 Hook 和 RAII 事务提供框架级横切能力，以生态类型 feature 开关融入 Rust 生态。**

核心差异化在于：**aifei 的调用形式 + Toasty 的编译期校验 + alun-db 独有的 schema 快照验证**，在保持 AI 友好度的同时提供 Rust 生态最强的编译期安全保障。

---

*文档版本：v0.2.0（随 crate 0.2.0 破坏性重写更新）· 最后更新：2026-09-26*

---
id: guides.db.getting-started
title: 数据库入门：五分钟跑通 + 让数据库连上
summary: 加依赖、配库（配置驱动 / 代码初始化两种方式）、db() 与 try_db() 的区别；5 分钟跑通第一个接口。
tags: [database, getting-started, config, connect]
status: active
owner: alun-core-team
source_of_truth: [code, alun-db/src, alun-db-core/src]
depends_on: [guides.database]
last_reviewed: 2026-10-06
---

# 数据库入门：五分钟跑通 + 让数据库连上

> 本文件是 [`database.md`](../database.md) 的 `getting-started` 分册；总目录与阅读顺序见 [数据库指南](../database.md)。


## 1.1 三步走

**第 1 步：加依赖。** 一句话记住：**`db` 是能力，驱动必须单独挑一个。**

```toml
[dependencies]
# 5 分钟跑通用 SQLite（零部署）；生产换成 "postgres"
alun = { version = "0.3", features = [
    "sqlite",                   # 驱动：sqlite / postgres / mysql 三选一（各自隐含 db）
    "chrono", "rust_decimal",   # 模型字段用到的生态类型
] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
```

| 你要什么 | feature |
| ---- | ---- |
| 只要数据库能力、先不定驱动 | `db`（**不含任何驱动**） |
| SQLite（学习 / 单机 / 测试，文件即库） | `sqlite` |
| PostgreSQL（生产推荐） | `postgres` |
| MySQL | `mysql` |
| 字段用 `DateTime` / `Decimal` / `Uuid` / `#[json]` | 再加 `chrono` / `rust_decimal` / `uuid` / `json` |

> `db` **只带能力、不带任何驱动** —— `alun-db` 自身 `default = ["async"]`，门面以
> `default-features = false` 引入它。所以只开 `db` 就去建池，会得到
> `DbError::UnsupportedFeature { feature: "driver", dialect: "postgres" }`。
> 写 Model **不需要**额外声明 `alun-db`：`Model` / `CrudExt` / `sql!` 全由 `alun::prelude::*` 提供；
> 生态类型缺了是**编译期**错误（字段类型不满足 `TypeMapper`）。

**第 2 步：写配置。** 生成一份默认配置，然后改 `[database]` 段：

```bash
mkdir -p config && cargo run -- gen-config
```

```toml
# config/config.toml
[database]
enabled = true
type = "sqlite"        # 新手先用 SQLite，零部署：文件即数据库
name = "app.db"        # SQLite 的 name 就是数据库文件路径
```

**第 3 步：写一个接口。** `App::start()` 会自动建连接池、跑一次连接自检（`SELECT 1`）、
注册全局单例 `db()`，你在 handler 里直接取用，**不需要把连接透传**。

写接口有**两条路**，先看清区别再选（[总目录 §1.2](../database.md) 有完整对照）：

**路 A · Model 模式**（业务表增删改查的默认选择）：

```rust,no_run
use alun::prelude::*;

#[derive(Model, Debug, Clone, serde::Serialize)]
#[table("sys_user")]
pub struct SysUser {
    pub id: i64,
    pub name: String,
}

#[alun::get("/api/users")]
async fn list_users() -> ResResult<Vec<SysUser>> {
    // 返回的就是 Vec<SysUser>：列名、类型编译期校验，不用手写 SQL、不用手工映射
    let users = SysUser::query().limit(50).find(db()).await?;
    Ok(Res::ok(users))
}

#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?.scan().start().await
}
```

**路 B · Row 模式**（动态表名 / 联表聚合 / 报表 / 一次性脚本）：

```rust,no_run
use alun::prelude::*;

#[derive(serde::Serialize)]
struct UserRes {
    id: String,
    name: String,
}

#[alun::get("/api/users")]
async fn list_users() -> ResResult<Vec<UserRes>> {
    // 手写 SQL，拿到的是动态行 Vec<Row>
    let rows: Vec<Row> = db()
        .find("SELECT id, name FROM sys_user ORDER BY id LIMIT 50")
        .await?;

    // Row 不实现 Serialize：返回给前端前映射成 DTO
    let users = rows
        .iter()
        .map(|r| UserRes {
            id: r.get_typed::<String>("id").unwrap_or_default(),
            name: r.get_typed::<String>("name").unwrap_or_default(),
        })
        .collect();

    Ok(Res::ok(users))
}
```

> 两条路的 handler 都返回 [`ResResult<T>`](ops.md) §5.10 ——
> 数据访问层的 `DbError` 可以直接 `?` 上来，不用手写 `map_err`。

---

## 1.2 两条路：Model 模式 vs Row 模式

这是全文最重要的一张表。两种模式**共享同一个连接池**（都是 `db()` / `&tx`），
差别在于「谁来保证列名与类型是对的」：

| | **Model 模式** `#[derive(Model)]` | **Row 模式** `Row` + 原生 SQL |
| --- | --- | --- |
| 写法 | `SysUser::query().filter(..).find(db())` | `db().find("SELECT ..")` |
| 拿到什么 | `Vec<SysUser>`（强类型结构体） | `Vec<Row>`（动态行，按列名取值） |
| 列名 / 类型谁保证 | **编译期**：`SysUser::status()` 是访问器，写错字段编译不过 | **运行时**：`get_typed::<T>("col")`，列名错了才报错 |
| 表名 | 结构体名推导或 `#[table("..")]` 固定 | 运行时字符串，**可以动态拼** |
| 时间戳自动填充 | ✅ `TimestampHook` 免费 | ❌ 自己写 |
| 软删除自动过滤 | ✅ 自动加 `deleted_at IS NULL` | ❌ 自己带条件 |
| Hook（审计 / 加密 / 校验） | ✅ 同步 + 异步 Hook | ❌ 不走 |
| 多租户自动隔离 | ✅ 自动补 `tenant_id = ?` | ❌ 不注入（fail-closed，会直接报错） |
| 分页一致性 | ✅ `Page<T>` + `into_res()` | 需自己算 `COUNT` |
| 适合 | **业务表的增删改查**（绝大多数场景） | 报表 / 联表聚合 / 动态表名 / 迁移脚本 / 一次性导数据 |

**选路规则（一句话）**：**能走 Model 就走 Model**。
只有「编译期不知道表名/列名」或「SQL 复杂到无法用 `filter` 表达」时，才退回 `Row` + 原生 SQL。

| 你现在要做的事 | 选哪条路 |
| ---- | ---- |
| 业务表增删改查、列表分页 | Model |
| 按主键/唯一列查单条 | Model（[4.1](crud.md) §4.1） |
| 多表联查 + 聚合 + 窗口函数 | Row（[5.2](recipes.md) §5.2） |
| 表名运行时才知道（分表 / 租户分表） | Row（[5.5](recipes.md) §5.5） |
| 报表、导出、对账 | Row |
| 读写一张表但要绕过软删/时间戳 | Row（并自己带上条件） |

---

## 1.3 文档的读法

真实项目里，「查列表」「改状态」「做报表」「导数据」会同时存在于同一个服务里。
所以本文按**你手头正在做什么**来组织（不按场景 A/B/C 切分）：

| 你正在做的事 | 看哪一章 | 一句话建议 |
| ---- | ---- | ---- |
| 第一次把库接上 | [第 2 章](getting-started.md) | 先用配置驱动，零代码 |
| 定义一张业务表 | [第 3 章](modeling.md) | `#[derive(Model)]`，约定能替你写掉大部分 |
| 写增删改查 | [第 4 章](crud.md) | 照着抄，六件事覆盖 90% 需求 |
| 遇到具体难题（联表、事务、导数据…） | [第 5 章](recipes.md) | 按需求标题直接跳 |
| 要做多租户隔离 | [第 6 章](multi-tenant.md) | 独立章节，含与普通操作的对照表 |
| 出错了、结果不对 | [第 7 章](ops.md) | 先看避坑清单 |
| 只想查某个 API 怎么写 | [附录 A](appendix.md) | |

**贯穿全文的主线**：能走 Model 层就不要手写 SQL。Model 层免费送你类型检查、字段访问器、
时间戳、软删除、Hook、分页一致性与多租户隔离；只有「运行时才知道表名」「联表聚合」
这类动态需求，才退回 `Row` + 原生 SQL。

---


## 2.1 先选数据库：三种类型怎么选

| 你的情况 | 选什么 | `type` 填 |
| ---- | ---- | ---- |
| 学习、演示、单机小工具、单元测试 | SQLite（文件即库，零部署） | `sqlite` |
| 正式项目、需要并发与丰富类型 | PostgreSQL | `postgres` |
| 团队/运维体系已是 MySQL、或要接存量库 | MySQL | `mysql` |

> 能不能一套代码都跑？能，见 [5.9 一套代码跑多种数据库](ops.md) §5.9。
> 但方言能力有差异（例如 MySQL 没有 `ON CONFLICT`），开发期用 SQLite、上线换 PG 时要提前避开这些差异。

## 2.2 方式一：配置驱动（推荐，零代码）

`App::start()` 会读 `[database]` 段 → 拼出连接 URL → 建连接池 → 执行 `SELECT 1` 自检 → 注册全局 `db()`。

三种数据库要填的字段不同，**只填该类型用得到的那几个**：

```toml
# ---------- SQLite：只要 type + name ----------
[database]
enabled = true
type = "sqlite"
name = "app.db"               # name 就是数据库文件路径（如 "./data/app.db"）
                              # host / port / user / password 对 sqlite 无效，不用填

# ---------- PostgreSQL ----------
[database]
enabled = true
type = "postgres"             # 写 "postgresql" 也行
host = "localhost"
name = "mydb"
user = "app_user"
password = "secret"
ssl_mode = "prefer"           # postgres 专用；port 省略时默认 5432

# ---------- MySQL ----------
[database]
enabled = true
type = "mysql"
host = "localhost"
name = "mydb"
user = "app_user"
password = "secret"           # port 省略时默认 3306
```

完整可调项（含连接池与迁移）：

```toml
[database]
enabled = true
type = "postgres"
host = "localhost"
port = 5432                   # 可省略，按类型取默认端口
name = "mydb"
user = "app_user"
password = ""
password_encrypted = false    # true 时 password 写 AES-GCM 密文，用 ALUN_SECRET_KEY 解密
max_connections = 10          # 连接池上限
min_connections = 2           # 连接池下限
connect_timeout = 10          # 建连超时（秒）
ssl_mode = "prefer"           # disable | prefer | require | verify-ca | verify-full
sql_logging = false           # 打印 SQL
slow_query_ms = 0             # 慢查询阈值（毫秒），0 = 关闭

[database.migration]
enabled = false               # 是否启用迁移模块
path = "migrations"           # 迁移目录；留空则只建 schema_migrations 表
auto_migrate = false          # 启动时自动执行未应用的迁移

# —— 来源（可选）——
# sources = ["migrations/system", "migrations/support"]  # 非空时取代 path
# seed_path = "seed"                                    # 种子目录；空 = 不跑

# —— 文件协议（可选，保持默认即可兼容三种真实项目风格）——
# statement_split = true      # 单文件多语句（默认开）
# up_down_sections = true     # 单文件 -- +migrate Up/Down 分段（默认开）
# version_prefix = ""         # 空 = 自动识别 V 前缀或无前缀
# name_separator = ""         # 空 = 自动判定（优先 __，回退 _）
# repeatable_prefix = "R"     # 可重复迁移前缀；"" = 关闭该种类
# seed_prefix = "S"           # 种子迁移前缀；"" = 关闭该种类
# baseline_prefix = "B"       # 基线标记前缀；"" = 关闭该种类

# —— 一致性（可选）——
# validate_checksum = true    # 校验已应用迁移的内容指纹
# divergent_policy = "abort"  # abort | warn | ignore
# out_of_order = false        # 乱序补迁是否免告警
# transaction_default = true  # 迁移默认包裹事务

# —— 存量库接入（可选）——
# import_legacy = false       # 读旧框架 _sqlx_migrations 还原已应用记录
# baseline_on_migrate = false # 迁移前先做基线标记
# baseline_version = ""       # 基线版本，如 "3"（<=3 视为已应用）
```

启动时到底发生了什么（顺序很重要，排查问题按这个顺序看日志）：

1. `db_url` 模块按 `type` 拼 URL（`sqlite://./{name}`、`postgres://user:pwd@host:port/db`、`mysql://...`）；
2. 若 `password_encrypted = true`，先用 `ALUN_SECRET_KEY`（32 字节 hex）解密密码；
3. `Db::connect_with(url, config)` 建池并执行 `SELECT 1`；
4. 成功打 `数据库连接成功`；若 `migration.enabled && auto_migrate` 为真则跑迁移（打 `数据库迁移完成: 应用 N 个迁移`）；
5. 最后 `set_db(db)` 注册全局单例，之后 handler 里的 `db()` 才可用。

**任一步失败都会让进程直接启动失败**（返回 `Error::Config`），不会带病运行 —— 这是有意的设计：
配置错、库连不上、迁移失败，都应该在第一秒暴露，而不是等你调接口时才发现。

## 2.3 方式二：代码内初始化（测试、多数据源、精细控制池）

不想走配置文件时（例如集成测试、临时脚本、连第二个库）：

```rust,ignore
// 注册成全局单例：之后 handler 里可以直接用 db()
alun_db::init("sqlite://./app.db").await?;
alun_db::init_with("postgres://user:pwd@localhost:5432/mydb", pool_config).await?;

// 或者自己持有句柄（测试、多数据源）：不注册全局，随用随传
let db = alun_db::Db::connect("sqlite://./app.db").await?;
let db = alun_db::Db::connect_with("mysql://user:pwd@localhost:3306/mydb", pool_config).await?;
```

用哪种？

| 需求 | 选择 |
| ---- | ---- |
| 生产服务、只有一个库 | 配置驱动（2.2），零代码 |
| 单元/集成测试、临时验证 | `Db::connect("sqlite://:memory:")` 自己持有句柄 |
| 一个进程连多个库（读写分离、多租户） | 各自 `Db::connect(...)`，按业务传句柄 |
| 要精细控制池的 `idle_timeout` / `max_lifetime` | 代码初始化：**这两个字段配置文件映射不到**，走 `[database]` 时它们是 `None`（不回收连接） |

## 2.4 关于 `db()` 与 `try_db()`

- `db()` → `&'static Db`。**未初始化直接 panic**。业务 handler 里可以放心用（启动阶段已保证）。
- `try_db()` → `Option<&'static Db>`。用于「不确定有没有库」的场景：库是可选的工具方法、启动钩子里探测、测试代码。

```rust,ignore
// 库是可选项时，不要 unwrap，显式降级
match try_db() {
    Some(db) => { /* 走数据库 */ }
    None => { /* 降级：返回缓存/空结果 */ }
}
```

> **导入提醒**：本文示例统一用 `use alun::prelude::*;`——它一次给出框架类型（`db()` / `Res` /
> `ApiError`）与数据库 API（`Model` / `CrudExt` / `sql!` / `DbError` …），只依赖 `alun` 即可。
> 若你另外直接依赖了 `alun-db` 并 glob 了它的 prelude，`db` / `try_db` / `Result` 会变成歧义名字
> （两个 prelude 各有其一）；此时显式导入 `use alun::prelude::db;` 优先于 glob，可消除歧义。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [返回总目录](../database.md) | [② 定义模型](modeling.md) → |

<sub>本分册属于「数据库实操（7 分册）」，共 7 篇。首篇 / 末篇可点「返回总目录」。</sub>

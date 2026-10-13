# 进阶数据库用法：联查 / 排序白名单 / 原生 SQL 租户安全 / 命名数据源

> 前置：`references/database.md`（基础 CRUD / 事务 / 迁移）+ 本文（进阶）。
> 全部 API 都从 `alun::prelude::*` 出 —— **不需要**在 `Cargo.toml` 里声明 `alun-db` / `alun-db-core`。

## 0. 先记住一条分层结论

| 需求 | 首选 | 何时才用原生 SQL |
|---|---|---|
| 单表查改删、分页 | `Model` / `ModelQuery` | — |
| 多表关联 | `ModelQuery::join::<U>()`（结构化） | 需要窗口函数 / 复杂报表时 |
| 报表、跨库聚合 | `sql!` + 终态动词 | — |
| 多数据源（主库 / 报表库 / 只读副本） | `set_db_named` + `db_named` | — |

`Model` 通道**自动注入租户条件、自动走 Hook、类型安全**；原生 SQL 通道**不会自动注入**，
所以框架对它还加了额外约束（`raw_tenant_annotated`，见下文第 3 节）。

## 1. 结构化关联查询 `ModelQuery::join::<U>(JoinKind, Expr)`

```rust,no_run
use alun::prelude::*;

// 主表模型
#[derive(Model)]
#[table("biz_order")]
struct Order { /* … */ }

// 从表模型（它的 #[tenant] 列由框架自动注入条件 —— 不要手写）
#[derive(Model)]
#[table("biz_customer")]
struct Customer { /* … */ }

async fn list() -> DbResult<()> {          // DB 层签名用 DbResult（E = DbError）；
                                           // handler 恒为 ResResult<T>，`?` 直接跨过去
    let rows = Order::query()
        .join::<Customer>(
            JoinKind::Inner,
            Expr::Binary {
                left:  Box::new(Expr::Column { table: Customer::TABLE, column: "id" }),
                op:    BinaryOp::Eq,
                right: Box::new(Expr::Column { table: Order::TABLE, column: "customer_id" }),
            },
        )
        .find(db())
        .await?;
    Ok(())
}
```

- `JoinKind::{Inner, Left, Right, Full}`；
- 条件用 `Expr::Binary { left, op, right }`（`BinaryOp::Eq` 等），`Expr::Column { table, column }` 指名列；
- 从表的租户条件**由框架按 `#[tenant]` 自动注入**。⚠️ 别手写 `tenant_id = ?` —— 手写就会绕过
  校验，且报表型 JOIN 容易漏列；
- 关联子查询 `Expr::Subquery` **框架未提供**，报表类需求用 `join` 或占位原生 SQL。

## 2. 排序白名单 `SortQuery` + `order_by_whitelisted`

用户传 `?sortBy=amount&order=desc` 是常态。直接拼字符串进 SQL = SQL 注入 + 任意列排序。
框架的解法：请求侧用 `SortQuery`，查询侧显式给白名单。

```rust,no_run
use alun::prelude::*;

#[derive(Model)]
#[table("biz_order")]
struct Order { /* … */ }

const SORTABLE: &[&str] = &["created_at", "amount", "status"];

async fn list(q: SortQuery) -> DbResult<()> {
    // 白名单外的字段名 → 直接报错（不是静默忽略）
    Order::query().order_by_whitelisted(q.field().unwrap_or("created_at"), q.is_desc(), SORTABLE)?;
    Ok(())
}
```

- `SortQuery { sort_by: Option<String>, order: SortOrder }`，`?sortBy=`（也有 `sortBy` 别名）与 `?order=asc|desc`；
- `.field() -> Option<&str>`、`.is_desc()`、`SortQuery::none()`；
- ⚠️ 白名单类型是 `&[&'static str]`，不能拿 `Vec<String>` 直接传；
- ⚠️ 越界是**报错**（400/422）而不是静默忽略 —— 这是刻意的，避免「白名单形同虚设」。

## 3. 原生 SQL 的租户安全通道 `/*tenant:o*/` + `raw_tenant_annotated()`

`ModelQuery::from_sql` / `T::sql(..)` 绕过了 AST，框架**没法**自动追加租户条件 ——
所以它改成「显式承诺」模型：

```rust,no_run
use alun::prelude::*;

const SQL: &str = r#"
  SELECT o.id, c.name
    FROM biz_order o
    JOIN biz_customer c ON c.id = o.customer_id
   WHERE o.created_at >= $1
     /*tenant:o*/  AND /*tenant:c*/"#;

async fn report(since: i64) -> DbResult<()> {
    let list = Order::sql(SQL)
        .raw_tenant_annotated()          // 必须写：声明「我已经标了占位」
        .find(db())
        .await?;
    Ok(())
}
```

规则：

| 情况 | 结果 |
|---|---|
| 没写 `raw_tenant_annotated()` 却用了 `/*tenant:* */` 风格的 SQL | ❌ 直接拒绝并提示 |
| 写了 `raw_tenant_annotated()` 但 SQL 里**漏标**某个租户表 | ❌ 编译/运行期报错，不会静默跑成全量 |
| 张表都标了 `/*tenant:别名*/` | ✅ 正常注入 |
| 模型本身无 `#[tenant]` / 上下文是 `Cross` / `[tenant].enabled=false` | ✅ 放行（无租户语义） |

配套指标：`alun_raw_sql_tenant_unbound_total`（安全指标，非 0 就该查）。

## 4. 命名数据源（主库 + 报表库 / 只读副本）

不做「自动读写分离」，只提供**显式命名数据源** —— 语义清晰、行为可预测：

```rust,no_run
use alun::prelude::*;

// 启动钩子 / 初始化时注册（同名重复注册返回 Err）
// Db 的构造入口是 connect / connect_with，没有 with_pool
let report_db = Db::connect_with(&cfg_report.url, cfg_report.clone()).await?;
set_db_named("report", report_db)?;

// 使用时点名（未注册会 panic，语义与 db() 一致的 fail-fast）
let rows = db_named("report")
    .find_with_params(
        "SELECT * FROM dwd_order_day WHERE dt = $1",
        vec![today.into()],
    )
    .await?;
```

| API | 行为 |
|---|---|
| `db()` | 默认（主库） |
| `set_db_named(name, db)` | 注册；空名 / 重名返回 `Err` |
| `db_named(name)` | 取用，未注册直接 **panic**（fail-fast） |
| `try_db_named(name) -> Option<&'static Db>` | 安全取值 |

> ⚠️ **事务必须走主库 `db()`**：事务独占一条连接，用 `db_named` 的只读源开事务会造成跨源写或不一致。
> 报表库账号建议给只读权限，从配置层面兜住误写。

## 5. 多租户的 fail-closed（回顾 + 排障）

| 防线 | 行为 |
|---|---|
| 读 | 租户模型自动追加 `tenant_id = ?`；**需要隔离但无上下文 → `DbError::InvalidValue`**（绝不静默全量） |
| 写 | INSERT 自动补租户值；行内值与上下文不一致 → **报错**（防越权写入） |
| 原生 SQL | 见第 3 节 |

定时任务 / CLI / MQ 消费 / 测试里没有 HTTP 上下文，必须显式包一层：

```rust,ignore
use alun::prelude::*;

tenant_scope(TenantContext::Scoped(Value::Int(1)), async {
    // 此处可正常读写该租户的模型与包了 /*tenant:* */ 的原生 SQL
    Ok::<_, DbError>(())
}).await?;
```

平台端 / 运维后台要跨全部租户：

```rust,ignore
tenant_scope(TenantContext::Cross, async { /* 放开所有 */ }).await?;
```

> 指标 `alun_tenant_scope_missing_total` 上涨 = 有代码需要租户上下文却没包 —— 直接当告警处理。

## 6. `Expr` 的 Raw 收口

`Expr::Raw` 变体已私有，只能通过两个构造函数拿到，避免随手拼字符串：

```rust,no_run
use alun::prelude::*;

// 无参数片段：必须是常量字符串，不能是可变输入
Expr::raw_trusted("LOWER(code) = 'abc'");

// 有参数：值走绑定，绝不进 SQL 文本
Expr::raw_trusted_bound("code = ?", vec![Expr::Value(Value::Text("A-1".into()))]);
```

- `raw_trusted` 只适合**写死的片段**；
- `raw_trusted_bound` 的第二个参数是 `Vec<Expr>`（绑定值）—— 用户输入必须走这条；
- `Value` 变体用 `Value::Text`（不是 `Value::Str`）。

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [数据库 CRUD / 事务 / 迁移](database.md) | [认证与鉴权](auth.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

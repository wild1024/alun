---
id: reference.crates.observability
title: 可观测 · 实时推送 · 导出导入 · 扩展点
summary: 可观测 · 实时推送 · 导出导入 · 扩展点——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [reference, observability, sse, metrics, xlsx, extension]
status: active
owner: alun-core-team
source_of_truth: [alun-web/src, alun-utils/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# 可观测 · 实时推送 · 导出导入 · 扩展点

> 本文件是 [`crates.md`](../crates.md) 的 `observability` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


这一节收口 0.3 里与「生产可用性」直接相关的几块能力：指标埋点、SSE 推送、Excel 读写、
插件化路由与行级异步 Hook、多数据源与排序白名单。它们都通过门面 feature 引入（`alun` 门面
自动转发到 `alun-web` / `alun-db` / `alun-cache` / `alun-utils`）。

## 2.15.1 `alun-web` 可观测性（`metrics` / `metrics-prometheus`）

| API / 配置 | 位置 | 说明 |
|---|---|---|
| `metrics` | `alun-web` feature；门面 `metrics = ["alun-web/metrics", "alun-db?/metrics", "alun-cache?/metrics"]` | 埋点。无 recorder 时全量 no-op，**可安全常开** |
| `metrics-prometheus` | 门面 feature | 额外装 Prometheus recorder 并挂 `/metrics` 端点 |
| `[metrics].enabled` | `config/config.toml` | 默认 `false` —— 关时端点不存在 |
| `[metrics].path` | 同上 | 绝对路径，**不拼** `[router].prefix` |
| `[metrics].allow_ips` | 同上 | 空 = 仅本机回环；抓取器在外网必须显式列出 IP |
| `[metrics].http` | 同上 | 是否埋 HTTP 层；关掉只留 DB / 缓存 / 安全指标 |
| `install_prometheus(cfg)` | `alun_web::metrics` | 建 handle → 装全局 recorder → 起 upkeep → 注册 `describe_*` |

```toml
# config/config.toml
[metrics]
enabled   = true
path      = "/metrics"
allow_ips = ["10.0.0.9"]   # 空 = 仅回环
http      = true
```

自动产出的指标（业务零改动）：

| 指标 | 标签 | 说明 |
|---|---|---|
| `alun_http_requests_total` / `alun_http_request_duration_seconds` / `alun_http_requests_in_flight` | `method` `route` `status` | `route` 是归一化路径（`/users/1` → `/users/:id`）；`status` 记 `2xx` 类别 —— 刻意压低基数 |
| `alun_db_queries_total` / `alun_db_query_duration_seconds` | `op` `table` `status` | ORM 读路径按 `Model::TABLE` 归类；原生 SQL 通道 `table = "raw"` |
| `alun_cache_operations_total` / `alun_cache_operation_duration_seconds` | `op` `result` | `result` ∈ `hit` / `miss` / `ok` / `error` |
| `alun_tenant_scope_missing_total` / `alun_raw_sql_tenant_unbound_total` | `table` | ⚠️ **安全指标**：租户 fail-closed 被触发 / 未绑定租户的原生 SQL 被拦截的次数 |

手动埋点用门面再导出的 `metrics` crate（`alun::metrics`）：`counter!(..)` / `histogram!(..)` / `gauge!(..)`。
想换 OpenTelemetry 等后端，自行 `metrics::set_global_recorder(..)` 即可，业务代码不用改。

> 三个坑：① 全局 recorder 只能装一次（并行测试里各装各的会静默失败，统一走 `App` 装配路径）；
> ② `describe_*` 在没有 recorder 时会被**静默丢弃** —— 注册必须排在 `install_prometheus()` 之后，
> 否则产物里没有 `# HELP` 说明行；③ `/metrics` 要排除在 HTTP 计数之外（`exclude_paths` 已默认含它），
> 否则自抓取污染 QPS。端点挂在中间件链**之外**（不穿鉴权/限流），访问控制完全由 `allow_ips` 负责。

## 2.15.2 `alun-web` 实时推送（feature `sse`）

```rust,ignore
use std::sync::OnceLock;
use alun::prelude::*;

static NOTIFY: OnceLock<Broadcaster<String>> = OnceLock::new();
fn notify() -> &'static Broadcaster<String> {
    NOTIFY.get_or_init(|| Broadcaster::new(1024))     // 广播器是进程级单例，自己持有
}

#[alun::get("/events")]
async fn events() -> impl axum::response::IntoResponse {
    sse_json(notify().subscribe(), "message")
}
```

| API | 位置 | 说明 |
|---|---|---|
| `Broadcaster::new(capacity)` | `alun_web::sse` | 基于 `tokio::sync::broadcast`，无锁有界；每个订阅端独立缓冲 |
| `Broadcaster::send(msg) -> usize` | 同上 | 扇出，返回送达连接数；订阅端消费不及只丢**最旧**消息、连接不断 |
| `Broadcaster::subscribe()` | 同上 | 取订阅端交给 `sse_json` |
| `Broadcaster::receiver_count()` | 同上 | 当前连接数（健康检查 / 限流可用） |
| `sse_json(rx, event_name)` | 同上 | 构造 SSE 响应，内置 15 秒保活注释行 |

适用场景：站内信、待办红点、导入导出进度、监控大屏 —— 只需「服务端 → 客户端」单向推送。
SSE 走普通 HTTP，浏览器自动带 Cookie / 可写 `Authorization` 头，因此**天然复用 `AuthLayer`**；
浏览器 `EventSource` 自带重连。双向通信场景没有内置 WebSocket，直接用 `axum::extract::ws`。
广播器是**进程级**的，多实例部署需换成 Redis Pub/Sub 或 `alun-kafka` 做跨进程扇出。

## 2.15.3 `WebPlugin` 与扩展点（`alun-web`）

`WebPlugin` 是 `Plugin` 的**子 trait**，在生命周期之上再贡献 Web 装配，三个方法都有默认空实现：

| 方法 | 默认 | 合并对象 |
|---|---|---|
| `routes(&self, &mut AlunRouter)` | 空 | 与 `#[alun::get]` 收集的路由合并 |
| `permission_rules() -> Vec<PermissionDef>` | `vec![]` | 与 `#[alun::permission]` 合并 |
| `no_auth_paths() -> Vec<NoAuthDef>` | `vec![]` | 与 `#[alun::no_auth]` 合并 |

`.web_plugin(p)` 一行同时完成生命周期注册与 Web 装配注册（内部包 `WebPluginAdapter`），
所以既有的 `impl Plugin for X` 改成 `impl WebPlugin for X` 即可升级。它解决的是**库形态分发**：
一个 crate 无法指望使用方的编译期收集拿到自己的 handler。

其余装配口子：`App::on_startup(|| async {})`（可调多次）、`App::layer(L)` / `App::layer_from_fn(f)`
（**链外**：覆盖整站含 401/403/429 与 404，但读不到身份；与 `on_startup` 共用同一条有序序列）、
`App::route_layer(L)`（**链内**：只覆盖匹配路由，能读 `AuthClaims`）、
`App::module(f)`（`f: fn(Self) -> Self`）、`App::routes(f)`（`f: fn(&mut AlunRouter)`）、
`App::into_router()`（只构建 `Router`，测试/内嵌用）。

行级异步 Hook 写在模型上（`#[hook_async(H)]` + `impl AsyncHook`），三个可选方法
`after_insert(&Row)` / `after_update(&Row)` / `after_delete(&Value)`，默认 `Ok(())`：
在**事务提交之后**触发，因此适合审计、缓存失效、消息投递、SSE 通知；
「和主业务同生共死」的逻辑要用同步 `Hook`。

## 2.15.4 命名数据源（`alun-web` + `alun-db`）

框架**不做**自动读写分离，只提供显式命名数据源：

| API | 行为 |
|---|---|
| `db()` | 默认（主库） |
| `set_db_named(name, db)` | 注册；空名 / 重名返回 `Err` |
| `db_named(name)` | 取用，未注册直接 panic（fail-fast） |
| `try_db_named(name) -> Option<&'static Db>` | 安全取值 |

```rust,ignore
// Db 的构造入口是 connect / connect_with，没有 with_pool
let report_db = Db::connect_with(&cfg_report.url, cfg_report.clone()).await?;
set_db_named("report", report_db)?;      // 启动钩子 / 初始化里注册
let rows = db_named("report").find("SELECT * FROM dwd_order_day WHERE dt = '2026-10-07'").await?;
```

> 事务必须走主库 `db()`（事务独占一条连接）；报表库账号建议给只读权限，从配置层兜住误写。

## 2.15.5 排序白名单与结构化联查（`alun-db`）

```rust,ignore
use alun::prelude::*;

const SORTABLE: &[&str] = &["created_at", "amount", "status"];

// 请求侧：SortQuery { sort_by: Option<String>, order: SortOrder }
//   ?sortBy=amount&order=desc
Order::query()
    .order_by_whitelisted(q.field().unwrap_or("created_at"), q.is_desc(), SORTABLE)?
    .paginate(db(), pq.page, pq.page_size)
    .await?;

// 联查：从表租户条件由框架按 #[tenant] 自动注入，勿手写
Order::query().join::<Customer>(
    JoinKind::Inner,
    Expr::Binary {
        left:  Box::new(Expr::Column { table: Customer::TABLE, column: "id" }),
        op:    BinaryOp::Eq,
        right: Box::new(Expr::Column { table: Order::TABLE, column: "customer_id" }),
    },
);
```

- `JoinKind::{Inner, Left, Right, Full}`；关联子查询 `Expr::Subquery` 未实现，报表类需求用 `join` 或占位原生 SQL；
- 白名单类型是 `&[&'static str]`，越界**直接报错**（400/422）而非静默忽略 —— 这是刻意的。

## 2.15.6 导出与导入（`alun-utils`，门面 feature `xlsx`）

| 能力 | API | feature |
|---|---|---|
| CSV（带 BOM） | `Export::to_csv(columns, rows)` / `to_csv_bytes(..)` | 默认 |
| JSON | `Export::to_json(&[T])` | 默认 |
| 读 CSV / JSON | `Import::from_csv(str)` / `Import::from_json::<T>(str)` | 默认 |
| 写 XLSX | `Export::to_xlsx_bytes(..)` / `to_xlsx_typed(..)` / `to_xlsx_workbook(&[XlsxSheet])` | `xlsx` |
| 读 XLSX | `Import::xlsx_sheet_names(bytes)` / `from_xlsx(..)` / `from_xlsx_sheet(..)` / `from_xlsx_typed::<T>(..)` | `xlsx` |

```rust,ignore
use std::collections::HashMap;
use alun::prelude::*;

let rows: Vec<HashMap<&str, String>> = vec![HashMap::from([("code", "A-001"), ("qty", "3")])];
let bytes = Export::to_xlsx_bytes(&["code", "qty"], &rows)?;      // 真 OOXML（PK\x03\x04）
let sheets = vec![XlsxSheet::new("明细", &["code", "qty"], &rows)];
let workbook = Export::to_xlsx_workbook(&sheets)?;                // 多 sheet，列宽自适应 + 冻结首行
let names = Import::xlsx_sheet_names(&workbook)?;
```

> 两条硬约束：① **不开 `xlsx` feature 时这些 API 不存在**（宁可编译失败，也不给「扩展名是 xlsx、
> 内容其实是 CSV」的错产物）；② **类型原样往返** —— 单元格类型由「字符串是不是纯数字」推断，
> 单号 `00123` 保持文本就存字符串，要参与求和的 `19999` 存数字串即可。
> 二进制导出不要塞进 `Res<T>`（那是 JSON 包装），直接返回 `impl IntoResponse`。

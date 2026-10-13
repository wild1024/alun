# 可观测性（metrics / Prometheus）

> 适用于：`features = ["metrics"]`（埋点）与 `features = ["metrics-prometheus"]`（导出端点）。
> 核心观念：**埋点可以无脑常开**（没有 recorder 时是 no-op），导出端点默认关闭。

## 两档 feature 的区别

| feature | 作用 | 代价 |
|---|---|---|
| `metrics` | 在 HTTP / DB / 缓存 / 租户安全关键路径**埋点**；未装 recorder 时全部 no-op | 极小（一次原子自增） |
| `metrics-prometheus` | 额外装 Prometheus recorder，并挂 `/metrics` 端点 | 需 `metrics-exporter-prometheus` |

门面 `alun` 的 `metrics` 会**弱下传**到已启用的 `alun-db` / `alun-cache`
（`metrics = ["alun-web/metrics", "alun-db?/metrics", "alun-cache?/metrics"]`），
不会出现「HTTP 有指标、DB 没指标」的半套。

## 配置

```toml
# config/config.toml
[metrics]
enabled   = false          # 默认关闭：enabled=false 时端点不存在
path      = "/metrics"     # 绝对路径，**不参与** [router].prefix 拼接
allow_ips = []             # 空 = 仅允许本机回环；Prometheus 在外网抓取必须显式列出其 IP
http      = true           # HTTP 层自动埋点；关掉只留 DB / 缓存 / 安全指标
```

```toml
# Cargo.toml
alun = { version = "0.3", features = ["postgres", "cache", "metrics-prometheus"] }
```

> `path` 是绝对路径：配 `/metrics` 就是 `/metrics`，不会变成 `/api/metrics`。

## 自动产出的指标

| 指标 | 标签 | 说明 |
|---|---|---|
| `alun_http_requests_total` | `method` `route` `status` | 请求总数 |
| `alun_http_request_duration_seconds` | 同上 | 耗时直方图 |
| `alun_http_requests_in_flight` | — | 在途请求 gauge |
| `alun_db_queries_total` / `alun_db_query_duration_seconds` | `op` `table` `status` | `table` 取 `Model::TABLE`，原生 SQL 通道为 `raw` |
| `alun_cache_operations_total` / `alun_cache_operation_duration_seconds` | `op` `result` | `result` ∈ `hit` / `miss` / `ok` / `error` |
| `alun_tenant_scope_missing_total` | `table` | ⚠️ **安全指标**：多租户 fail-closed 被触发次数 |
| `alun_raw_sql_tenant_unbound_total` | `table` | ⚠️ **安全指标**：未绑定租户的原生 SQL 被拦截次数 |

**标签基数是刻意压低的**：`route` 会把 `/users/1` 归一化成 `/users/:id`（数字 / UUID / ≥24 token 的
路径段分别归一为 `:id` / `:uuid` / `:token`），单条路径上限 512 字符、超出记 `__other__`；
`status` 只记 `2xx` 类别。这样 QPS 不会随数据爆炸。

两个安全指标**持续增长 = 有 bug**：说明某处该用 `tenant_scope` 却没包，或该标 `cross_tenant` 没标。
上线后建议直接把它们做成告警。

## 手动埋点

```rust,ignore
use alun::prelude::*;   // 已重导出 alun_web::metrics

#[alun::get("/api/startup")]
async fn startup() -> ResResult<()> {
    let started = std::time::Instant::now();
    // …初始化…
    metrics::histogram!("app_startup_duration_seconds", started.elapsed().as_secs_f64() as f64);
    metrics::counter!("app_cold_start_total").increment(1);
    Ok(Res::ok_empty())
}
```

`metrics` 是**门面直接再导出**的 crate（`alun::metrics`），`counter!` / `histogram!` / `gauge!`
与上游 `metrics` 完全一致。想换成 OpenTelemetry 等后端，自行
`metrics::set_global_recorder(..)` 即可，业务代码不用改。

## Prometheus 抓取

```yaml
# prometheus.yml
scrape_configs:
  - job_name: alun
    scrape_interval: 15s
    static_configs:
      - targets: ["127.0.0.1:8023"]
```

抓取前确认：`[metrics].enabled = true` 且 `allow_ips` 里含抓取机 IP —— 否则会拿到 **403**
（端点在中间件链**之外**，不穿鉴权/限流，访问控制完全由 `allow_ips` 负责）。

## 四个坑

1. **全局 recorder 只能装一次**：并行测试里多个用例各自 `set_global_recorder`，只有一个能成功，
   第二个静默失败 → 别的用例看到「没指标」。统一走 `App` 的装配路径（或 `install_prometheus()`）。
2. **`describe_*` 在没有 recorder 时会被静默丢弃**：注册 describes 必须**排在** `install_prometheus()`
   之后，否则产物里没有 `# HELP` 说明行（Prometheus 里就是裸指标名，可读性大打折扣）。
3. **`/metrics` 自抓取会污染自己的 QPS**：`exclude_paths` 已默认把 `[metrics].path` 排除在外；
   自己再加一个导出端点时记得同步加进去。
4. **`describe_*` 与 `install_prometheus` 的调用顺序不能反**（见 2），但 `describe_*` 本身可以多次调用、
   幂等覆盖。

## 自测

```bash
curl -s http://127.0.0.1:8023/metrics | head -50
# 应看到 # HELP 说明行、以 alun_ 开头的指标、以及 alun_tenant_scope_missing_total
```

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [SSE 实时推送](realtime-sse.md) | [导出导入（CSV / JSON / XLSX）](export-and-import.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

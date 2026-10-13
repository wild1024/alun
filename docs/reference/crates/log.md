---
id: reference.crates.log
title: `alun-log` — 日志初始化
summary: `alun-log` — 日志初始化——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [log, reference, tracing]
status: active
owner: alun-core-team
source_of_truth: [alun-log/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-log` — 日志初始化

> 本文件是 [`crates.md`](../crates.md) 的 `log` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun-log/src/lib.rs`

`alun_log::init(config: &LogConfig)` — 根据 `LogConfig` 配置 tracing（**无返回值，不返回 `Result`**）：

- `format = "text"`：彩色文本
- `format = "json"`：JSON 结构化日志（对接 ELK/Loki）
- `dir` 非空：日滚文件输出（按天轮转）；`dir` 为空：标准输出
- 过滤级别优先取 `RUST_LOG` 环境变量；未设置时使用默认过滤器——用户代码 `debug`、`alun-*` 框架 crate 用 `LogConfig.level`、第三方喧闹 crate 固定 `warn`

## 使用示例

```rust,ignore
// ── 自动初始化（App 启动时自动调用） ──
// App::new() 内部调用 alun_log::init(&config.log)
// 无需手动初始化

// ── 手动初始化（返回 ()，不要加 ?） ──
use alun_log;
alun_log::init(&config.log);

// ── 在代码中使用 tracing ──
use tracing::{info, warn, error, debug, trace};

trace!("进入函数: process_order({})", order_id);
debug!("数据库查询耗时: {}ms", duration_ms);
info!("用户登录成功: user_id={}", user_id);
warn!("缓存即将过期: key={}", cache_key);
error!("支付回调异常: order_id={}, 原因={}", order_id, reason);

// ── 结构化字段 ──
info!(method = "POST", path = "/api/order", status = 200, duration_ms = 5, "请求完成");

// ── Span（分布式追踪上下文） ──
use tracing::instrument;
#[instrument(skip(db))]
async fn create_order(db: &Db, req: CreateOrderReq) -> Result<Order, Error> {
    info!("开始创建订单");
    // ... span 自动记录函数名、参数、耗时
    Ok(order)
}

// ── 配置文件示例 ──
// [log]
// level = "debug"         # 开发环境
// format = "json"         # 生产环境（对接 ELK/Loki）
// dir = "logs"            # 同时输出到文件
// file_prefix = "alun"
```

***

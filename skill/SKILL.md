---
name: alun-project
description: 项目级速查——Alun 框架（v0.3.0，21 crate Cargo workspace）的 AI 操作要点：feature 分层、handler 单一出口、路由 scan()、命令、高频踩坑与文件指针。加载即用，详细参考见 ../skill-alun/SKILL.md。
agent_created: true
---

# Alun Framework — 项目级 AI 速查

首个面向 AI Coding 的 Rust Web 框架（配置驱动 + 插件架构）。版本 0.3.0（Edition 2024，rust-version 1.94.1），
Cargo workspace 含 21 个 crate + 5 个示例工程。分层：`用户代码 → alun（门面）→ 功能 crate → alun-core（纯骨架）`。

## 命令
- 检查：`cargo check --workspace`（单包 `cargo check -p alun`）
- 测试：`cargo test --workspace`；doctest：`cargo test -p alun-docs-tests --doc`
- 跑示例：`cd examples/00-quick-start && cargo run`（首次 `cargo run -- gen-config`）
- 文档：`cargo doc --no-deps -p alun`
- 注：README 提到的 `./scripts/quick-test.py` 在本 checkout 不存在，用上面 `cargo test`。

## 铁律
1. 唯一框架依赖 `alun`；全部 API 来自 `use alun::prelude::*;`，不引子 crate。
2. handler 唯一签名 `ResResult<T> = Result<Res<T>, ApiError>`；响应 `{code,msg,data}`。
3. 数据层 `DbResult<T>`（= `Result<T,DbError>`），`?` 自动上转 `ApiError`，handler 不写 `map_err`。
4. 路由 `scan()` 自动收集 `#[alun::get/post/...]`；分组 `#[alun::controller]`；库 `#[alun::routes]`+`.module()`。
5. feature：`default=["core"]`；数据库必选驱动 `sqlite/postgres/mysql`；预设 `mini⊂lite⊂full`；`fs`=`fs-local`。
6. `ApiError` 是唯一错误出口（`WebError` 已删除）；5xx 对前端模糊化。
7. 文档 Rust 块被 rustdoc 编译：可编译 `rust,no_run`，纯示意 `rust,ignore`。

## 高频踩坑
- 只写 `db` 不选驱动 → 建池 `UnsupportedFeature`；必须选 `sqlite/postgres/mysql`。
- 业务 Cargo.toml 直接引 `alun-db` → 应走 `alun` 门面 + feature。
- 改宏路由后 404 → 确认 `scan()` 在 main 且 handler 同编译单元。
- `lite` 不含 `template`，要模板写 `features=["lite","template"]`。
- 不要串 `alun-core/db`（已不存在）。

## 关键文件
- 门面 + feature：[../alun/Cargo.toml](../alun/Cargo.toml)
- 错误收敛：`../alun-web/src/api.rs`、`../alun-web/src/response.rs`
- 路由收集：`../alun-web/src/router.rs`、App：`../alun-web/src/app.rs`、JWT：`../alun-web/src/jwt.rs`
- DB 门面：`../alun-db/src/`、`../alun-db-core/src/`
- 示例：`../examples/`（00-quick-start…04-test-suite）
- 文档：[`../docs/index.md`](../docs/index.md)（总索引）+ `../docs/guides/`（入门/数据库/全栈/推荐架构）+ `../docs/reference/`（crates/dependencies）+ `../docs/architecture/`（overview + adr/）+ `../docs/migrations/v0.3.0-breaking.md`；机器可读 [`../docs/index.json`](../docs/index.json)
- 详细参考手册（人与 AI 通用）：[../skill-alun/SKILL.md](../skill-alun/SKILL.md) + `../skill-alun/references/`

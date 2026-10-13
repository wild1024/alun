# AGENTS.md — Alun Framework

给编码 Agent（Claude Code / Cursor / Codex / WorkBuddy Agent）的操作指令。
本文件是「指令」，详细参考与可运行示例见 `README.md`、`docs/`、`skill-alun/SKILL.md`、`examples/`。

## 项目是什么
Alun 是**面向 AI Coding 的 Rust Web 框架**：配置驱动 + 插件架构，一个接口 = 一个 `async fn` + 一个宏注解。
版本 `0.3.0`（Edition 2024，rust-version 1.94.1），Cargo workspace 含 **21 个 crate + 5 个示例工程**。
分层：`用户代码 → alun（门面）→ 功能 crate（web / db / cache / template …）→ alun-core（纯骨架：Error / Plugin）`。

设计理念承接 **[Aifei](https://github.com/jfinal/aifei)**（[官网 aifei.cn](https://aifei.cn)，世界首个 AI Coding 框架）的
Just Service 范式——Alun 是它在 Rust 生态的表达（Just Config），不是代码翻译。
回答「这个 API 为什么这样设计」时，必要时读 `docs/architecture/aifei-tribute.md`（含 aifei → alun 的逐项映射与取舍）。

## 铁律（动手前必读）
1. **唯一框架依赖是 `alun`**。业务 `Cargo.toml` 不写 `alun-db` / `alun-db-core` 等子 crate；
   路由 / 配置 / 数据库 / 响应 / 错误全部从 `use alun::prelude::*;` 来。
2. **handler 只有一种签名**：`async fn -> ResResult<T>`，其中 `ResResult<T> = Result<Res<T>, ApiError>`。
   响应统一包成 `{ "code": 0, "msg": "ok", "data": .. }`。出错：`Err(ApiError::not_found(".."))` 等。
3. **数据访问层用 `DbResult<T>`**（即 `Result<T, DbError>`）。`DbError` 通过 `From<DbError> for ApiError`
   （在 `alun-web/src/api.rs`）自动上转，**handler 里一行 `?` 都不用写**；404 / 409 / 500 语义留在 web 层。
4. **路由自动发现**：`App::new()?.scan().start()` 会扫整个 crate 内所有 `#[alun::get/post/put/patch/delete]`，
   无需手写注册代码。分组用 `#[alun::controller("/prefix")]`；探针用 Builder 链式；库形态用 `#[alun::routes]` + `.module()`。
5. **feature 分层（见 `alun/Cargo.toml`）**：`default = ["core"]`（最小 HTTP 服务）。
   - 数据库**必选一个驱动**：`sqlite` / `postgres` / `mysql`（驱动隐含 `db`）；只写 `db` 建池会报 `UnsupportedFeature`。
   - 生态类型按需开：`chrono` / `rust_decimal` / `uuid` / `ulid` / `json` / `ipnetwork` / `time` / `jiff` / `bigdecimal`（模型字段用到才开）。
   - 预设严格递增：`mini ⊂ lite ⊂ full`。`lite`（推荐单库生产）= postgres + cache + plugin + fs-local。
   - `fs` = `fs-local`；要 MinIO 写 `fs-minio`。`template` / `sse` / `metrics` / `metrics-prometheus` / `xlsx` / `xss` 各自独立。
6. **错误出口只有 `ApiError`**。`WebError` 已删除，不要再用；5xx 对前端只显示「服务器内部错误」，详情仅进日志。
7. **不要串 `alun-core/db`**（该 feature 已不存在）——alun-core 与数据库内核互不依赖是硬约束。
8. **文档代码块会被 rustdoc 真实编译**：可编译示例用 ` ```rust,no_run `，纯示意用 ` ```rust,ignore `。
9. **宏路由已被 `scan()` 收集**，改路由后无需手动登记；新增示例工程需在根 `Cargo.toml` 的 `members` 里登记。
10. **密码哈希**：生成用 argon2 0.6，验证兼容 bcrypt——不要自行实现哈希。

## 目录结构（要点）
```
alun/                门面：prelude + 统一再导出 + 宏（feature 定义在这里）
alun-core/           纯骨架：Error、Plugin（零 HTTP / 零 DB 依赖）
alun-macros/         过程宏：get/post/put/patch/delete/controller/permission/no_auth/layer/plugin/task_handler
alun-web/            Web：App、Router、中间件、JWT、全局资源、SSE、可观测（Res/ApiError/PageQuery 在此）
alun-db/             DB 门面：Db/Row/Model/CrudExt/PersistExt/ModelQuery/ChangeSet/sql!/事务/迁移
alun-db-core/        DB 内核：Row/Value/Expr/Dialect/Hook/DbError/Page
alun-db-sql/         SQL 渲染器与方言
alun-db-macro/       Model/Row/TypeMapper 派生宏 + sql! 宏
alun-db-driver-*/    驱动：sqlite(rusqlite)/postgres(tokio-postgres)/mysql(mysql_async)
alun-config/         配置：TOML 加载、多环境 Profile、动态配置
alun-log/            tracing 初始化（text/json/按天滚动）
alun-cache/          LocalCache + Redis + 命名空间
alun-template/       Jinja2（minijinja）
alun-utils/          字符串/日期/脱敏/ID/校验/加密/导出导入/单号
alun-plugin/         定时任务/邮件/单号/异步任务池
alun-kafka/ alun-task/ alun-fs/   消息/任务/文件存储抽象
examples/            5 个可运行示例（00-quick-start…04-test-suite）
docs/                文档体系：index.md（人类 TOC）+ index.json（机器索引）+ glossary.md（术语表）；guides/（入门/数据库/全栈/推荐架构）· reference/（crates/dependencies）· architecture/（overview + adr/）· migrations/（破坏性变更迁移指南）
skill-alun/          详细 AI 参考手册（SKILL.md + 10 份 references/）
```

## 命令
- 类型/编译检查：`cargo check --workspace`（或单包 `cargo check -p alun`）
- 构建：`cargo build -p alun`（开发 profile 已 `debug=false` 提速，需调试信息临时恢复）
- 测试：`cargo test --workspace`（完整门禁）；文档 doctest：`cargo test -p alun-docs-tests --doc`
- 跑示例：`cd examples/00-quick-start && cargo run`（首次 `cargo run -- gen-config` 生成 `config/config.toml`）
- 文档：`cargo doc --no-deps -p alun`
- 格式化/剪裁：`cargo fmt` / `cargo clippy --workspace`

> 注意：根 `README.md` 提到的 `./scripts/quick-test.py` 在本 checkout 不存在，按上面 `cargo test` 命令即可。

## 高频踩坑速查
- 只写 `features=["db"]` 不选驱动 → 建池报 `UnsupportedFeature { feature: "driver" }`，必须选 `sqlite/postgres/mysql`。
- 在业务 `Cargo.toml` 直接引 `alun-db` → 应通过 `alun` 门面 + feature，不要引子 crate。
- 修改宏路由后 404 → 确认 `scan()` 在 `main` 里、handler 用了 `#[alun::get]` 且在同一编译单元。
- 文档示例编译失败 → 可编译片段用 `rust,no_run`，纯示意用 `rust,ignore`，否则 rustdoc 会真编译报错。
- 想用 `render_template` 但没开 `template` feature → `lite` 不含 template，需显式 `features=["lite","template"]`。

## 文档检索与变更（AI 必读）

- **检索**：先读 [`docs/index.json`](./docs/index.json)（机器可读索引：id / 摘要 / 标签 / token 数 / 依赖）；人类可读目录见 [`docs/index.md`](./docs/index.md)。按标签或场景定位文档，再用稳定 **id** 引用（如 `guides.database#多租户三通道`），**不要引用行号**。
- **引用文档**：用 `docs/index.json` 里的 `id` 或 `path#锚点`，例如「见 `reference.crates` 的 `alun-web` 条目」。行号会变，id 稳定。
- **更新文档**：改动文档时同步更新其顶部 YAML front matter（`last_reviewed` / `status`）；新增概念补 [`docs/glossary.md`](./docs/glossary.md)。
- **架构 / 破坏性变更走流程**：新决策写 [`docs/architecture/adr/ADR-00XX-*.md`](./docs/architecture/adr/)（不修改历史，只追加）；版本变更记 [`CHANGELOG.md`](./CHANGELOG.md)；破坏性变更必须配套 [`docs/migrations/`](./docs/migrations/) 下的迁移指南。

## 详读指针
- 入门 / 架构 / 全栈选型：[`README.md`](./README.md)、[`docs/guides/getting-started.md`](./docs/guides/getting-started.md)（总目录，实体在 `docs/guides/startup/`：setup / config-basics / first-endpoint / middleware / hooks-plugins / custom-config / config-reference / security-model）、[`docs/architecture/overview.md`](./docs/architecture/overview.md)、[`docs/guides/fullstack.md`](./docs/guides/fullstack.md)
- 数据库：`alun-db/src/`、`alun-db-core/src/`、`docs/guides/database.md`、`skill-alun/references/database.md`
- 认证 / 安全：`alun-web/src/jwt.rs`、`alun-web/src/middleware/`、`docs/guides/startup/security-model.md`（认证、鉴权与隔离安全模型）
- 数据库：`docs/guides/database.md`（总目录）→ 分册 `docs/guides/db/`：`modeling` / `crud` / `recipes`（联表·批量·动态表名）/ `ops`（视图·Hook·**迁移**·多库·错误映射·踩坑）/ `multi-tenant`（多租户对照 + §6.7 设计要点）/ `appendix`（API 速查）
- 破坏性变更：`docs/migrations/v0.3.0-breaking.md`
- 速查手册（人与 AI 通用）：[`skill-alun/SKILL.md`](./skill-alun/SKILL.md) + `skill-alun/references/`

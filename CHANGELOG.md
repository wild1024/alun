# Changelog

本文件记录 alun 的**全局版本变更**（破坏性变更、新增能力、安全修复）。

约定：

- 每个破坏性 / 重大变更都对应 `docs/migrations/` 下的一份迁移指南，以及（若涉及架构决策）`docs/architecture/adr/` 下的一份 ADR。
- 文档总索引见 `docs/index.md`；AI 检索入口见 `AGENTS.md` 与 `llms.txt`。
- 格式遵循 [Keep a Changelog](https://keepachangelog.com/)：`Added` / `Changed` / `Deprecated` / `Removed` / `Fixed` / `Security`。

---

## [Unreleased]

（暂无）

---

## [0.4.1] — 2026-10-15

主题：**自定义中间件的粒度与身份可见性**（让「中间件放哪」这件事从隐式契约变成显式 API）。

### Added

- **`Principal`**（进 prelude）：身份视图 `user_id` / `username` / `roles` / `claims`，实现 `FromRequestParts` + `OptionalFromRequestParts`；`has_role` / `has_any_role` / `has_permission` / `has_any_permission` 透传通配语义。用于在 handler 里按业务字段访问当前用户，而不是直接碰内部 `TokenClaims`。
  - `user_id` 的类型是 **`UserId`（`sub` 的原样字符串）**，不是 `Option<Uuid>`：主键形态（UUID / 自增整型 / 复合作用域键 / 外部 IdP 主体）由业务决定，强类型一律在**调用点**声明 —— `p.user_id.parse::<Uuid>()`。把 `user_id` 定成 `Option<Uuid>` 会让上述非 UUID 系统整体退化为 `None`、拿不到身份。
- **`UserId` 增强**：`as_str()` / `Display` / `AsRef<str>` / `parse::<T>()`（任意 `FromStr` 类型，失败得 `None`）。
- **`AuthClaims` 双通道提取器**：① 优先读请求扩展（认证层已注入）；② 否则**自行验签并查黑名单**。因此 `#[no_auth]` 路由、以及 `App::into_router()` 的测试 / 嵌入路径同样能提取身份，且不依赖「提取器 vs 中间件」的先后顺序。
- **`Option<AuthClaims>` / `Option<Principal>`**：可选身份（无 token 得 `None`，不报 401），用于「登录与否都能访问」的接口。
- **`App::layer_from_fn(fn(Router) -> Router)`**：`with_middleware_hook` 的正名（语义、行为完全一致）。
- **`App::route_layer(L)`**：**链内**中间件 —— 在建链之前应用，因此位于 Auth / 权限 / 限流**之内**，只作用于匹配路由（不含 404 与 `/metrics`），**能**读 `AuthClaims` / 租户上下文。
- **`AlunRouter::merge_with_layer(prefix, sub, wrap)`**：子树级 layer（只给某前缀下的整组路由挂）。
- **`#[alun::layer(L)]`**：路由级 layer 属性宏，写在路由注解**上方**；会摘掉该路由的编译期自注册、改生成「带 layer 的注册」，因此不会重复挂载。
- `AlunRouter::add_route_with_layer`（未知动词回退 `any`）。
- **请求体上限自动装配**：`UploadConfig::max_request_body_bytes()`（= `max_size_mb` × 1 MiB + 32 MiB multipart 分帧余量），`App::start()` / `App::into_router()` 会据此自动挂载 `DefaultBodyLimit` 并在启动日志打印生效值，业务侧无需再手写请求体上限中间件。

### Changed

- `App::layer(L)` 的位置与能力边界现在是**显式文档**：位于内置链**之外**、整站最外层，覆盖 404 / 静态文件 / `/metrics`，能记到 401 / 403 / 429 与真实端到端耗时，但**读不到身份**。
- `App::into_router()` 的装配顺序改为与 `App::start()` **完全一致**（此前会静默丢弃全部自定义层，导致「测试通过、线上两套行为」）；唯一保留的差别是静态文件服务不在此挂载。
- 自定义层序列的注册语义：多个注册**按顺序全部生效**（基线为 `Option`，后注册者会静默吃掉先注册者）。
- `AuthClaims` 提取器成为「认证的唯一真值实现」的复用点：`AuthLayer` 与提取器通道 2 共用同一个校验函数。

### Deprecated

- `App::with_middleware_hook(f)` → 请迁移到 `App::layer_from_fn(f)`（保留为转发，行为一致）。

### Fixed

- `AuthClaims` 提取器在**无 token** 且未调用 `set_config` 时返回 **401**，不再因取不到配置而 500（新增回归测试 `channel2_without_token_is_401_not_500`）。
- `App::handle_not_found()` 不再依赖全局配置已初始化（未初始化时降级为默认文案，不再 panic），使 `into_router()` 在测试 / 嵌入路径下可用。

---

## [0.4.0] — 2026-10-15

主题：**迁移器工程化 + 启动装配热路径优化，并修掉一批「配置写了却不生效 / 值根本解不出来」的静默缺陷**。

### Changed

- **破坏性 · `PermissionCheckLayer.rules` 类型变更**：`Vec<PermissionRule>` → **`Arc<Vec<PermissionRule>>`**，权限层与 `App` 共享同一份规则，消除启动时「每条路由深拷贝整份规则」的热点（实测为装配耗时大头）。新增 `PermissionCheckLayer::push_rule()` 作为唯一的追加入口；直接构造该字段或用 `.rules.push(..)` 的代码需改为 `push_rule()` 或 `Arc::new(vec![..])`。
- **破坏性 · 配置迁移 overlay 语义修正**：迁移相关配置在 profile 文件中的覆盖此前只合并 3 个字段，其余被**静默丢弃**；现改为逐字段覆盖。此前「profile 里写了却不生效」的迁移配置现在会真正生效——若曾依赖该丢弃行为，请复核配置。
- `[database].sql_logging = true` 从此**真正生效**：此前该开关不产生效果；现在会把 `tokio_postgres` 日志级别提升到 `debug`，输出完整 SQL 与参数。
- 迁移器 `MigratorOptions` / `LoadedMigration` 新增公开字段（均带 `Default`），以字面量全字段构造的代码需补齐。

### Added

- `DbExecutor::supports_script()` 与 `Transaction::{supports_script, execute_script}`：支持整段脚本的驱动可一次往返跑完一次迁移（PG 走 simple query，往返 O(2n)→O(1)）；默认实现回退 `UnsupportedFeature`。
- 迁移器新选项 `synchronous_commit` 与 `single_transaction`（后者把整批 pending 放进同一事务，失败整批回滚）。
- `App::migrate_embedded()` / `Migrator::source_embedded`：直接注入 `migrate!` 编译期迁移器，运行期不再依赖 `migrations/` 目录。
- `alun_log::init_with_sql_logging()`。
- `alun-utils/examples/hash_password.rs`：用 `Crypto::hash_password` 预生成 Argon2 口令哈希的 CLI 示例。

### Fixed

- **PostgreSQL `inet` / `cidr` 解码**：线格式补回缺失的长度字节，修复「所有真实 inet 值解码失败」。
- 迁移历史记录由「先删后插」改为单条 UPSERT（PG/SQLite `ON CONFLICT`、MySQL `ON DUPLICATE`），并加内联 simple-query 快路径。
- 迁移指纹计算由手写 SHA-256 改用 `sha2`，并加 `OnceLock` 缓存。
- PG 连接池预热改为并发建连。

### Performance

- 迁移文件加载：≥8 个文件时并行读取；语句拆分器加零分配快速路径。
- metrics recorder 与启动钩子 / 插件并行安装，并补分段耗时日志。

---

## [0.3.0] — 2026-10-06

首个公开的破坏性大版本。**2.0 从未提交至本仓库**，0.3.0 直接承接 2.0 的设计意图并落地。

### Added

- 结构化多租户三条通道：A. `ModelQuery::join::<U>()`（从表自动注入租户，机制上封死泄漏）/ B. 原生 SQL `/*tenant:别名*/` + `raw_tenant_annotated()` 显式承诺 / C. `tenant_scope(TenantContext::Cross, ..)` 跨租户作用域。
- v2 迁移引擎：`Migrator` 单一 API、方言感知 `split_statements`、`(kind, name)` 记录表防重跑、`import_legacy` / `baseline`、`migrate!` 编译期嵌入、`reset` / `fresh` 安全门。
- 安全可观测性指标：`alun_tenant_scope_missing_total`、`alun_raw_sql_tenant_unbound_total`。
- 更多能力见 `docs/migrations/v0.3.0-breaking.md` §3（F1–F20 落地状态）。

### Changed

- 默认 feature 由 `full` 改为 `core`；`db` 不再隐含数据库驱动（需显式开启 `alun-db` + 驱动 feature）。
- 路由注册由 `linkme` 切换为 `inventory` + `#[alun::routes]` / `App::module()` 双通道（决策见 `docs/architecture/adr/ADR-0001-inventory-over-linkme.md`）。
- handler 统一出口 `ResResult<T>`；`DbError` 实现 `From` 进 `ApiError`；`WebError` 删除。
- 导出由 CSV 改为真 XLSX（`xlsx` feature）；`Expr::Raw` 私有化。
- feature 预设收敛为 `mini ⊂ lite ⊂ full`；版本号统一。
- 多租户改为 **fail-closed** 安全模型（无上下文即报错 / 原生 SQL 需显式承诺 / 鉴权不匹配即拒绝），决策见 `docs/architecture/adr/ADR-0002-fail-closed-tenant-isolation.md`。

### Removed

- `WebError`、`linkme` 依赖、CSV 导出、顶层 `Option<TenantId>` 式租户穿透写法。
- 旧的 `07`–`16` 演进 / 报告类文档（2.0→3.0 变更记录）已合并归档至 `docs/migrations/v0.3.0-breaking.md`，不再单独维护。

### Security

- 封住 F2 租户泄漏通道（结构化联查从机制上阻断从表泄漏）。
- 三条 fail-closed 底线：无租户上下文报错 / 原生 SQL 必须显式承诺租户 / 鉴权作用域不匹配即拒绝。

完整破坏性变更清单（B1–B10）、迁移对照表、F1–F20 落地状态、验证矩阵见 `docs/migrations/v0.3.0-breaking.md`。

---

## [0.2.0] 及更早

未提交至本仓库（设计停留在 2.0 分支，未入库）。本套文档体系从 0.3.0 起正式建立。

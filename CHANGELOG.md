# Changelog

本文件记录 alun 的**全局版本变更**（破坏性变更、新增能力、安全修复）。

约定：

- 每个破坏性 / 重大变更都对应 `docs/migrations/` 下的一份迁移指南，以及（若涉及架构决策）`docs/architecture/adr/` 下的一份 ADR。
- 文档总索引见 `docs/index.md`；AI 检索入口见 `AGENTS.md` 与 `llms.txt`。
- 格式遵循 [Keep a Changelog](https://keepachangelog.com/)：`Added` / `Changed` / `Deprecated` / `Removed` / `Fixed` / `Security`。

---

## [Unreleased]

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

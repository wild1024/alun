//! # Alun —— 快、简、美的 Rust Web 框架
//!
//! Alun 是一个**配置驱动、插件架构**的 Rust Web 框架，基于 [axum] 0.8 构建。
//! 核心理念：**一行启动，零成本抽象，约定优于配置**。
//!
//! ## 设计原则
//!
//! - **配置驱动**：数据库、缓存、认证、CORS、限流、压缩等基础设施行为全部在 `config.toml` 中声明
//! - **全局资源单例**：通过 `db()`、`cache()`、`cfg()` 访问资源，无需 `State` 注入
//! - **编译期路由发现**：`#[get]`/`#[post]` 宏通过 `inventory` 在编译期收集处理器，无运行时反射
//! - **插件拓扑排序**：插件声明依赖关系，按依赖顺序启动，反向停止
//! - **Row+Model 双模式 CRUD**：动态 `Row`（表名 + 主键列 + 有序字段表）承载数据，**不做变更追踪**（写回时整行提交）
//! - **RAII 事务**：Rust `Drop` + `?` 保证事务回滚，编译器强制安全检查
//!
//! ## 快速开始
//!
//! 在 `Cargo.toml` 中添加依赖：
//!
//! ```toml
//! [dependencies]
//! # 默认 = core：能起 HTTP 服务的最小集（路由 + 中间件 + 配置 + 日志）
//! alun = "0.3"
//!
//! # 按场景叠加（推荐用预设）
//! alun = { version = "0.3", features = ["mini"] }   # SQLite + 本地缓存，零外部服务
//! alun = { version = "0.3", features = ["lite"] }   # PostgreSQL 单库生产（推荐）
//! alun = { version = "0.3", features = ["full"] }   # 全部能力（含 MySQL / Kafka / MinIO）
//! ```
//!
//! > **默认不引入任何数据库驱动、消息队列与对象存储后端。**
//! > 能力按 `core → L1 能力 → L2 驱动/后端 → L3 重型后端` 逐层叠加，
//! > 因此也不需要为用不到的依赖付出编译时间与系统构建链（CMake / C 工具链）。
//!
//! 最小可运行应用（3 步）：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! #[alun::get("/")]
//! async fn hello() -> Res<String> {
//!     Res::ok("Hello, Alun!".into())
//! }
//!
//! #[tokio::main]
//! async fn main() {
//!     App::new()
//!         .expect("初始化失败")
//!         .scan()        // 自动发现 #[get]/#[post] 处理器
//!         .start()       // 读取 config/config.toml，初始化资源，启动服务器
//!         .await
//!         .unwrap();
//! }
//! ```
//!
//! 生成默认配置文件：
//!
//! ```bash
//! cargo run -- gen-config    # 在 config/ 目录生成 config.toml
//! ```
//!
//! ## 功能开关 (Feature Flags)
//!
//! 采用**分层**设计：默认最小，能力逐步叠加。默认集是 `core`。
//!
//! ```toml
//! alun = { version = "0.3" }                              # core（默认）
//! alun = { version = "0.3", features = ["cache"] }         # core + 缓存
//! alun = { version = "0.3", features = ["sse"] }           # core + 实时推送辅助
//! alun = { version = "0.3", features = ["metrics"] }       # core + 埋点（no-op 安全）
//! alun = { version = "0.3", features = ["xlsx"] }          # core + 真 XLSX 导出/解析
//! alun = { version = "0.3", features = ["mini"] }          # 预设：单机自足
//! alun = { version = "0.3", features = ["lite"] }          # 预设：单库生产
//! ```
//!
//! ### L0 · 核心
//!
//! | 功能 | 启用模块 | 描述 |
//! |------|---------|------|
//! | `core` | 全部基础设施 | **默认**。HTTP 服务最小集：路由 + 中间件 + 配置 + 日志，无外部重量级依赖 |
//! | `config` | `alun_web/config` | 配置加载与全局配置访问器（`core` 已包含）|
//!
//! ### L1 · 基础设施能力（相互独立）
//!
//! | 功能 | 启用模块 | 描述 |
//! |------|---------|------|
//! | `cache` | `alun_cache` | 缓存抽象（本地内存 / Redis）|
//! | `template` | `alun_template` | Jinja2 模板引擎（minijinja）|
//! | `plugin` | `alun_plugin` | 插件基础设施（定时任务、通知、单号）|
//! | `fs` | `alun_fs` | 文件存储抽象能力（等价 `fs-local`）|
//! | `sse` | `alun_web/sse` | 单向实时推送辅助：`Broadcaster` + `sse_json`（仅 `futures-util`，无重量依赖）|
//! | `metrics` | `alun_web/metrics` | 埋点：HTTP / DB / 缓存指标。未装 recorder 时是 **no-op，可安全常开** |
//! | `metrics-prometheus` | `alun_web/metrics-prometheus` | 在 `metrics` 之上再加 Prometheus recorder 与受保护的 `/metrics` 端点 |
//! | `xlsx` | `alun_utils/xlsx` | 真 XLSX：`Export::to_xlsx_*` 写、`Import::from_xlsx_*` 读（rust_xlsxwriter + calamine）|
//! | `xss` | `alun_utils/xss` | HTML 净化（`ammonia`）。**不开时这些 API 不存在**，不会退化成「什么都不做」|
//!
//! ### L2 · 数据库：**能力与驱动分离**
//!
//! | 功能 | 启用模块 | 描述 |
//! |------|---------|------|
//! | `db` | `alun_db` | 数据库**能力**（CRUD / 事务 / 钩子 / 迁移）。**不含任何驱动** |
//! | `sqlite` | 同上 + SQLite 驱动 | 驱动 feature 各自隐含 `db`，通常只写驱动即可 |
//! | `postgres` | 同上 + PostgreSQL 驱动 | — |
//! | `mysql` | 同上 + MySQL 驱动 | — |
//!
//! > 只开 `db` 而不选驱动时，crate 可编译，但建池阶段返回
//! > `DbError::UnsupportedFeature { feature: "driver", dialect: ".." }`。
//!
//! ### L2 · 文件存储后端（正交于 `fs`）
//!
//! | 功能 | 描述 |
//! |------|------|
//! | `fs-local` | 本地磁盘后端（零外部依赖）|
//! | `fs-minio` | MinIO / S3 后端（引入 `aws-sdk-s3`）|
//!
//! ### L3 · 重型后端（默认全关）
//!
//! | 功能 | 启用模块 | 描述 |
//! |------|---------|------|
//! | `kafka` | `alun_kafka` | Kafka 生产者/消费者 |
//! | `task` | `alun_task`（`model`）| **任务模型**：`#[task_handler]` 收集、存储抽象、类型、重试策略。无 Kafka |
//! | `task-kafka` | `alun_task`（`kafka`）| 在 `task` 之上再加 Kafka worker 与 `TaskPlugin` |
//!
//! > `kafka` / `task-kafka` 传递依赖 `rdkafka`，**构建需要 CMake + C 工具链**
//! > （librdkafka 的 `configure` 是 Unix shell 脚本，纯 Windows 工具链下无法构建）。
//!
//! ### 生态类型映射
//!
//! | 功能 | 描述 |
//! |------|------|
//! | `chrono` / `time` / `jiff` | 时间类型映射 |
//! | `rust_decimal` / `bigdecimal` | 高精度小数映射 |
//! | `uuid` / `ulid` | 标识类型映射 |
//! | `json` / `ipnetwork` | JSON / 网段类型映射 |
//!
//! 以上各自隐含 `db`，模型字段用到哪个就开哪个。
//!
//! ⚠️ 开 feature ≠ 能在代码里 `use` 到 crate：门面只透传 feature，不会把 `chrono` /
//! `rust_decimal` 等放进你的 `extern prelude`。所以写 `use chrono::{DateTime, Utc}` 时
//! 业务工程还得自己声明 `chrono = "0.4"`；想省掉这一步就改走门面重导出
//! `use alun::prelude::chrono;`。
//!
//! ### 预设组合
//!
//! 命名力求最短，能力严格递增：**`mini` ⊂ `lite` ⊂ `full`**。
//!
//! | 预设 | 组成 | 适用场景 |
//! |------|------|---------|
//! | `mini` | `core` + `sqlite` + `cache` | 单机自足，**零外部服务**（原型 / 小工具 / 测试）|
//! | `lite` | `core` + `postgres` + `cache` + `plugin` + `fs-local` | 单库生产（推荐）；模板不在预设内，自行加 `template` |
//! | `full` | `lite` + `mysql` + `kafka` + `task-kafka` + `fs-minio` + `sse` + `metrics-prometheus` + `xlsx` + `template` | 全部能力，构建最重 |
//!
//! > 分层纪律由 `ci/check_feature_discipline.sh` 守卫：默认集一旦混入
//! > 数据库驱动 / Kafka / 对象存储，脚本即失败。
//!
//! > 不开 `xlsx` 时 `Export::to_xlsx_*` / `Import::from_xlsx_*` **根本不存在**——
//! > 不是「静默退化成 CSV」。同理不开 `xss` 时净化 API 不存在。
//! > `alun-core` 的 `web`（`Res`/`ApiError` 的 `IntoResponse` 实现）由 `alun-web` 恒定开启，无需手动指定。
//!
//! # 路由注册
//!
//! ## 方式一：Proc 宏 + scan()（推荐）
//!
//! 零手动注册，`scan()` 自动发现所有带 `#[alun::get]`/`#[alun::post]` 等注解的函数：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Serialize)]
//! struct UserRes { id: String, name: String }
//!
//! #[alun::get("/api/users")]
//! async fn list_users() -> Res<Vec<UserRes>> {
//!     Res::ok(vec![])
//! }
//!
//! #[alun::post("/api/users")]
//! async fn create_user(
//!     ValidatedJson(req): ValidatedJson<CreateUserReq>,
//! ) -> ResResult<UserRes> {
//!     Ok(Res::ok(UserRes { id: "1".into(), name: req.name }))
//! }
//!
//! #[alun::put("/api/users/{id}")]
//! async fn update_user(
//!     Path(id): Path<String>,
//!     ValidatedJson(req): ValidatedJson<UpdateUserReq>,
//! ) -> Res<()> {
//!     let _ = (id, req);
//!     Res::ok_empty()
//! }
//!
//! #[alun::delete("/api/users/{id}")]
//! async fn delete_user(Path(id): Path<String>) -> Res<()> {
//!     let _ = id;
//!     Res::ok_empty()
//! }
//!
//! # #[derive(Deserialize, validator::Validate)] struct CreateUserReq { name: String }
//! # #[derive(Deserialize, validator::Validate)] struct UpdateUserReq { name: String }
//! ```
//!
//! ## 方式二：Controller 分组
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! struct AdminController;
//!
//! // `#[controller("/前缀")]` 标注在 impl 块上，块内方法用 `#[alun::get]` 等标注路由
//! #[alun::controller("/api/admin")]
//! impl AdminController {
//!     #[alun::get("/dashboard")]
//!     async fn dashboard() -> Res<String> {
//!         Res::ok("admin dashboard".into())
//!     }
//!
//!     #[alun::delete("/users/{id}")]
//!     async fn delete_user(Path(id): Path<String>) -> ResResult<()> {
//!         let _ = id;
//!         Ok(Res::ok_empty())
//!     }
//! }
//! ```
//!
//! ## 方式三：构建器链式
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! # async fn demo() -> Result<()> {
//! App::new()?
//!     .get("/api/users", list_users)
//!     .post("/api/users", create_user)
//!     .group("/api/v2", |app| {
//!         app.get("/hello", hello_v2)
//!     })
//!     .start()
//!     .await
//! # }
//! # async fn list_users() -> Res<String> { Res::ok("".into()) }
//! # async fn create_user() -> Res<String> { Res::ok("".into()) }
//! # async fn hello_v2() -> Res<String> { Res::ok("".into()) }
//! ```
//!
//! ## 提取请求参数
//!
//! ```rust,no_run
//! use alun::prelude::*;
//! use std::collections::HashMap;
//!
//! // 路径参数
//! #[alun::get("/users/{id}")]
//! async fn get_user(Path(id): Path<String>) -> Res<JsonValue> { unimplemented!() }
//!
//! // 查询参数
//! #[alun::get("/users")]
//! async fn search(Query(params): Query<HashMap<String, String>>) -> Res<JsonValue> { unimplemented!() }
//!
//! // 认证用户
//! #[alun::get("/me")]
//! async fn me(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<JsonValue> { unimplemented!() }
//! ```
//!
//! # 统一响应
//!
//! ## `Res<T>` — 标准成功响应
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! # fn demo(user: String, list: Vec<String>) {
//! let a = Res::ok(user);                              // { code:0, msg:"ok", data:... }
//! let b = Res::ok_with_msg(1, "创建成功");             // 自定义消息
//! let c = Res::ok_empty();                            // 无 data 字段
//! let d = Res::ok_msg("操作成功");                     // 仅消息
//! let e: Res<()> = Res::fail(codes::BAD_REQUEST, "参数不能为空"); // 自定义错误码
//! let f = Res::page(list, 100, 1, 20);                // 分页响应（total=100, page=1, size=20）
//! # let _ = (a, b, c, d, e, f);
//! # }
//! ```
//!
//! ## `ApiError` — 结构化 HTTP 错误
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! // `ResResult<T>` 即 `Result<Res<T>, ApiError>`
//! async fn handler() -> ResResult<String> {
//!     Err(ApiError::bad_request("参数错误"))
//! }
//!
//! # fn demo() {
//! // 便捷构造器
//! let a = ApiError::bad_request("参数错误");              // 400
//! let b = ApiError::unauthorized("请先登录");              // 401
//! let c = ApiError::forbidden("权限不足");                 // 403
//! let d = ApiError::not_found("用户不存在");               // 404
//! let e = ApiError::conflict("用户名已存在");              // 409
//! let f = ApiError::unprocessable_entity("邮箱格式不正确"); // 422
//! let g = ApiError::too_many_requests("请求过于频繁");     // 429
//! let h = ApiError::internal("数据库连接失败");            // 500（前端看到"服务器内部错误"）
//! let i = ApiError::internal_masked("服务器内部错误", "..."); // 自定义前端消息 + 内部详情
//! let j = ApiError::service_unavailable("服务暂不可用");   // 503
//! # let _ = (a, b, c, d, e, f, g, h, i, j);
//! # }
//! ```
//!
//! > **v0.1.1+**: `ApiError::internal(msg)` 自动屏蔽错误详情——前端永远看到 `"服务器内部错误"`，真实错误写入日志。用 `internal_masked(public, detail)` 自定义前端提示。
//!
//! ## `PageQuery` — 分页参数
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! let pq = PageQuery::new(1, 20);   // page=1, page_size=20（自动规整到合法范围）
//! let offset = pq.offset();         // (page-1) * page_size = 0
//! let limit = pq.limit();           // 20
//! let pages = pq.calc_pages(95);    // ceil(95 / 20) = 5
//! # let _ = (offset, limit, pages);
//! ```
//!
//! 作为提取器使用时，参数缺失 / 为 0 / 越界都会被规整（既不会 400，也不会因
//! `page=0` 造成下溢）。数据库返回的 `Page` 可直接转成标准分页响应，
//! 无需手工搬运 `total` / `page` / `size` 字段：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! #[derive(serde::Serialize)]
//! struct UserRes { id: String, name: String }
//!
//! async fn list_users(Query(pq): Query<PageQuery>) -> ResResult<PageData<Vec<UserRes>>> {
//!     let page = db()
//!         .paginate("SELECT * FROM users ORDER BY id", pq.page, pq.page_size)
//!         .await
//!         .map_err(|e| ApiError::internal_masked("查询失败", e.to_string()))?;
//!
//!     // Row → DTO（Row 本身不实现 Serialize），映射后分页元数据原样保留
//!     let page = page.map(|r| UserRes {
//!         id: r.get_typed::<String>("id").unwrap_or_default(),
//!         name: r.get_typed::<String>("name").unwrap_or_default(),
//!     });
//!
//!     // data = { list, total, page, page_size }
//!     Ok(page.into_res())
//! }
//! ```
//!
//! # 输入验证
//!
//! ## `ValidatedJson` 提取器
//!
//! 使用 `validator` crate 的 `Validate` derive 宏：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//! use serde::Deserialize;
//! use validator::Validate;
//!
//! #[derive(Debug, Deserialize, Validate)]
//! struct CreateUserReq {
//!     #[validate(length(min = 2, max = 50, message = "名称需 2-50 个字符"))]
//!     name: String,
//!
//!     #[validate(email(message = "邮箱格式不正确"))]
//!     email: String,
//!
//!     #[validate(range(min = 18, max = 120))]
//!     age: u8,
//! }
//!
//! #[alun::post("/api/users")]
//! async fn create(
//!     ValidatedJson(req): ValidatedJson<CreateUserReq>,
//! ) -> ResResult<()> {
//!     // req 已自动校验，失败返回 422
//!     let _ = req;
//!     Ok(Res::ok_empty())
//! }
//! ```
//!
//! ## 内置自定义验证器
//!
//! ```rust,no_run
//! use alun::prelude::*;
//! use alun::{validate_uuid, validate_mobile, validate_password_strength,
//!            validate_id_card, validate_date, validate_datetime,
//!            validate_date_or_datetime, validate_email, validate_url};
//! use serde::Deserialize;
//! use validator::Validate;
//!
//! #[derive(Debug, Deserialize, Validate)]
//! struct ComplexReq {
//!     #[validate(custom(function = "validate_uuid"))]
//!     parent_id: String,          // UUID 格式（空则跳过）
//!
//!     #[validate(custom(function = "validate_mobile"))]
//!     mobile: String,             // 手机号（空则跳过）
//!
//!     #[validate(custom(function = "validate_password_strength"))]
//!     password: String,           // 强密码（始终校验）
//!
//!     #[validate(custom(function = "validate_date_or_datetime"))]
//!     release_date: Option<String>, // 纯日期 或 ISO 8601 时间戳
//! }
//! ```
//!
//! | 验证器 | 说明 | 空值处理 |
//! |--------|------|---------|
//! | `validate_uuid` | UUID v1~v7 格式 | 跳过 |
//! | `validate_mobile` | 中国大陆手机/固话 | 跳过 |
//! | `validate_password_strength` | 8+ 字符，含大小写+数字+特殊字符 | 始终校验 |
//! | `validate_id_card` | 18 位身份证（含校验位） | 跳过 |
//! | `validate_date` | YYYY-MM-DD 格式 | 跳过 |
//! | `validate_datetime` | ISO 8601 / RFC 3339 | 跳过 |
//! | `validate_date_or_datetime` | 纯日期或完整时间戳 | 跳过 |
//! | `validate_email` | 邮箱格式 | 跳过 |
//! | `validate_url` | HTTP/HTTPS URL | 跳过 |
//!
//! 手动调用验证：
//!
//! ```rust,no_run
//! use alun::{ApiError, ValidateExt};
//! use validator::Validate;
//!
//! #[derive(serde::Deserialize, Validate)]
//! struct ComplexReq {
//!     #[validate(length(min = 2, max = 50))]
//!     name: String,
//! }
//!
//! fn process(req: &ComplexReq) -> std::result::Result<(), ApiError> {
//!     req.validate_or_reject()?;   // 失败返回 ApiError(422)
//!     Ok(())
//! }
//! ```
//!
//! # 数据库 CRUD
//!
//! 需要 `features = ["db"]`。`Db` 是对 PostgreSQL/MySQL/SQLite 的统一门面。
//!
//! ## Row 模式
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! # async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
//! // 插入：Row 携带「表名 + 主键列」元数据
//! let mut row = Row::new().with_entity("users", "id");
//! row.insert("id", Sid::uuid7());               // UUID v7 主键（推荐）
//! row.insert("name", "张三");
//! row.insert("email", "zhangsan@example.com");
//! row.insert("age", 28);
//! let inserted: Row = db().insert_row(&row).await?;
//!
//! // 按主键查询（主键值从插入结果回读）
//! let user: Option<Row> = match inserted.pk_value().cloned() {
//!     Some(pk) => db().find_row_by_pk("users", "id", pk).await?,
//!     None => None,
//! };
//!
//! // 更新：整行写回（Row 无变更追踪，按需只改想改的字段）
//! if let Some(mut row) = user {
//!     row.insert("age", 29);
//!     db().update_row(&row).await?;
//!
//!     // 删除
//!     db().delete_row(&row).await?;
//! }
//!
//! // 原始 SQL 查询
//! let rows: Vec<Row> = db().find("SELECT * FROM users WHERE age > 18").await?;
//! let count: i64 = db().scalar("SELECT COUNT(*) FROM users").await?;
//!
//! // 分页查询（返回 Page<Row>：list / total / page / size / pages）
//! let page = db().paginate("SELECT * FROM users ORDER BY id", 1, 20).await?;
//! # let _ = (rows, count, page);
//! # Ok(())
//! # }
//! ```
//!
//! ## Row 字段访问
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! # fn demo(row: &Row) {
//! let a = row.get("name");                  // Option<&Value>
//! let b = row.get_typed::<String>("name");  // Result<String>
//! let c = row.get_typed::<i64>("age");      // Result<i64>
//! let d = row.pk_value();                   // Option<&Value> — 主键值
//! let e = row.contains("field");            // bool — 字段是否存在
//! let f = row.table();                      // Option<&str> — 表名
//! let g = row.primary_key();                // &str — 主键列名（默认 "id"）
//! # let _ = (a, b, c, d, e, f, g);
//! # }
//! ```
//!
//! 自定义主键：`Row::new().with_entity("products", "code")`，或事后 `row.set_primary_key("code")`。
//!
//! ## Model（强类型糖）
//!
//! `#[derive(Model)]` 带一组可覆盖的默认约定：表名由结构体名蛇形推导（`SysUser` → `sys_user`）、
//! 名为 `id` 的字段即主键、`id` 整型主键默认自增、含 `created_at` / `updated_at` 字段时自动挂载
//! `TimestampHook`、`Option<...>` 的 `deleted_at` 字段自动作为软删标记（`#[model(hard_delete)]` 关闭）。
//! 查询是关联函数（`CrudExt`），持久化是实例方法（`PersistExt`）：
//!
//! ```rust,ignore
//! #[derive(Model)]
//! pub struct SysUser { id: i64, email: String, status: i32 }
//!
//! let user = SysUser::find(&db, 1).await?;                     // 按主键；不存在 → DbError::NotFound
//! let users = SysUser::find_by(&db, "status", (1,)).await?;    // where_or_field：字段名 或 WHERE 片段
//! let users = SysUser::query().filter(SysUser::id().gt(10)).find(&db).await?;  // 链式强类型查询
//! SysUser::of(1).set(SysUser::email(), "a@b.com").update(&db).await?;          // 变更集：只更新被 set 的列
//! ```
//!
//! - `ModelQuery<T>`（`Model::query()` / `Model::sql(sql!(...))`）以**终态动词**选择执行方式：
//!   `find` / `find_first` / `find_one` / `find_one_or_null` / `exists` / `paginate` /
//!   `for_each` / `for_each_page` / `scalar` / `execute`；
//! - `ChangeSet<T>`（`Model::of(pk)`）免 `SELECT` 做部分更新 / 删除；
//! - `CrudExt` / `PersistExt` / `ModelQuery` / `ChangeSet` / `SqlQueryExt` / `RowBatchExt`
//!   的首参都是 `&impl SqlExecutor`——`&Db` 与事务内的 `&Transaction` 通用。
//!
//! ## 单一出口：handler 只需要 `ResResult<T>`
//!
//! handler 的返回类型**只有一种**：`ResResult<T>`（= `Result<Res<T>, ApiError>`）。
//! 成功写 `Ok(Res::ok(data))`，业务报错写 `Err(ApiError::not_found(".."))`；
//! 数据库、框架、IO 的错误**直接 `?`**，映射全部内置：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! #[derive(Model, Debug, Clone, serde::Serialize)]
//! #[table("users")]
//! pub struct User { pub id: i64, pub name: String }
//!
//! // 数据访问层：错误类型是 DbError，签名用 DbResult
//! async fn load_user(id: i64) -> DbResult<User> {
//!     User::find(db(), id).await
//! }
//!
//! // handler：DbError 直接 `?` 上来，一行 map_err 都不用写
//! #[alun::get("/users/{id}")]
//! async fn get_user(Path(id): Path<i64>) -> ResResult<User> {
//!     Ok(Res::ok(load_user(id).await?))
//! }
//! ```
//!
//! `DbError` → 状态码的默认语义：`NotFound` → 404、`UniqueViolation` /
//! `ForeignKeyViolation` / `CheckViolation` → 409、`Deadlock` / `SerializationFailure` /
//! `LockTimeout` → 500「服务繁忙，请重试」、其余 → 500 脱敏。
//! 原文（含 SQL）统一塞进 `ApiError::internal_detail`，只进日志不回显。
//! 想自定义文案就显式 `match`：`Err(DbError::UniqueViolation { .. }) =>
//! return Err(ApiError::conflict("邮箱已被占用"))`。
//!
//! ## RAII 事务
//!
//! 返回 `Ok` 自动提交，返回 `Err` 自动回滚。支持强制回滚：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! # async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
//! db().transaction(|tx| async move {
//!     tx.execute_with_params(
//!         "UPDATE accounts SET balance = balance - 100 WHERE id = $1", vec!["A".into()],
//!     ).await?;
//!     tx.execute_with_params(
//!         "UPDATE accounts SET balance = balance + 100 WHERE id = $1", vec!["B".into()],
//!     ).await?;
//!
//!     let rows = tx.find_with_params("SELECT balance FROM accounts WHERE id = $1", vec!["A".into()]).await?;
//!     if rows.first().and_then(|r| r.get("balance")).is_none() {
//!         return Err(alun_db::prelude::DbError::NotFound);   // 提前返回 → Drop 自动回滚
//!     }
//!     Ok(())  // 正常返回 → 自动提交
//! }).await?;
//! # Ok(())
//! # }
//! ```
//!
//! 需要显式回滚时用 `db().begin()` 拿到 `Transaction`，再调用 `tx.rollback().await?`
//! 或 `tx.commit().await?`（两者均幂等）。
//!
//! ## 钩子 (Hooks)
//!
//! ```rust,no_run
//! use alun::prelude::*;
//! use alun_db::prelude::{DbError, Hook};
//!
//! struct UppercaseNameHook;
//!
//! // Hook 是同步 trait：在写入前后就地修改 Row
//! impl Hook for UppercaseNameHook {
//!     fn before_insert(&self, row: &mut Row) -> std::result::Result<(), DbError> {
//!         let name = row.get_typed::<String>("name").unwrap_or_default();
//!         row.insert("name", name.to_uppercase());
//!         Ok(())
//!     }
//!     fn before_update(&self, row: &mut Row) -> std::result::Result<(), DbError> {
//!         let name = row.get_typed::<String>("name").unwrap_or_default();
//!         row.insert("name", name.to_uppercase());
//!         Ok(())
//!     }
//! }
//! ```
//!
//! 内置 `TimestampHook` 写入 `created_at` / `updated_at`：含这两个字段的 `Model` 会**自动挂载**它
//! （排在 Hook 链最前），无需手写 `#[hook(TimestampHook)]`。自定义 Hook 在 `Model` 上通过
//! `#[hook(UppercaseNameHook)]` 声明，插入/更新/删除时按声明顺序触发（`before_*` 正序、`after_*` 逆序）；
//! `TimestampHook` 定义在 `alun_db_core::hook`（prelude 已导出）。
//!
//! # 认证与授权
//!
//! ## 配置
//!
//! ```toml
//! [middleware.auth]
//! enabled = true
//! jwt_secret = "your-secret-minimum-32-characters-long"
//! ignore_paths = ["/api/login", "/api/public"]
//! access_token_expire_secs = 7200
//! ```
//!
//! > `ignore_paths` 支持精确匹配和**前缀匹配**：配置 `/api/public` 后，所有 `/api/public/...` 均免认证。
//!
//! ## JWT 令牌管理
//!
//! ```rust,no_run
//! use alun::prelude::*;
//! use alun::JWT;
//!
//! # async fn demo() -> Result<()> {
//! let jwt = JWT::from_config();  // 从 config.toml 读取 secret + 过期时间
//!
//! // 登录——生成令牌
//! let access_token = jwt.create_access_token(
//!     "user_123",                     // user_id
//!     Some("username"),               // 用户名
//!     &["admin".into()],              // 角色
//!     &["user:read".into()],          // 权限
//! )?;
//! let old_refresh = jwt.create_refresh_token("user_123")?;
//!
//! // 验证
//! let claims: TokenClaims = jwt.validate(&access_token)?;
//!
//! // 刷新（旧 refresh_token 自动加黑名单）
//! let (new_access, new_refresh) = jwt.refresh(&old_refresh).await?;
//!
//! // 登出（加黑名单）
//! jwt.logout(&claims).await;
//! # let _ = (new_access, new_refresh);
//! # Ok(())
//! # }
//! ```
//!
//! ## 获取当前用户
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! #[alun::get("/api/me")]
//! async fn me(Extension(AuthClaims(claims)): Extension<AuthClaims>) -> Res<JsonValue> {
//!     Res::ok(json!({
//!         "user_id": claims.sub,
//!         "username": claims.username,
//!         "roles": claims.roles,
//!         "permissions": claims.permissions,
//!     }))
//! }
//! ```
//!
//! ## TokenClaims 权限检查
//!
//! ```rust,no_run
//! # use alun::TokenClaims;
//! # fn check(claims: &TokenClaims) {
//! claims.has_role("admin");                                    // bool
//! claims.has_any_role(&["admin", "moderator"]);                // bool
//! claims.has_permission("user:write");                         // bool
//! claims.has_any_permission(&["user:read", "user:write"]);     // bool
//! claims.is_super_admin();                                     // 拥有 "*" 权限
//! # }
//! ```
//!
//! ## 权限守卫
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! # async fn demo() -> Result<()> {
//! // 构建器方式
//! App::new()?
//!     .with_permission("GET", "/api/admin/stats", admin_stats, "admin:access")
//!     .with_role("DELETE", "/api/users/{id}", delete_user, "admin")
//!     .start()
//!     .await
//! # }
//!
//! // 宏方式
//! #[alun::permission(path = "/api/admin/users", method = "GET", permission = "admin:read")]
//! #[alun::get("/api/admin/users")]
//! async fn list_users() -> Res<Vec<JsonValue>> { Res::ok(vec![]) }
//!
//! # async fn admin_stats() -> Res<String> { Res::ok("".into()) }
//! # async fn delete_user() -> ResResult<()> { Ok(Res::ok_empty()) }
//! ```
//!
//! ## 免认证路径
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! #[alun::no_auth("/api/public")]
//! #[alun::get("/api/public")]
//! async fn public_api() -> Res<String> {
//!     Res::ok("免认证".into())
//! }
//! ```
//!
//! # 全局资源单例
//!
//! 资源由 `App::start()` 初始化一次，通过全局函数访问：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//! use alun::{cache, cfg, config, db, render_template};
//!
//! # async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
//! // 数据库（需要 features = ["db"]）
//! let user = db().find_row_by_pk("users", "id", "u_1").await?;
//!
//! // 配置（静态只读）
//! let port = cfg().server.listen.clone();
//!
//! // 缓存（需要 features = ["cache"]）
//! cache().set_ex("key", &"value", 3600).await?;
//!
//! // 模板（需要 features = ["template"]）
//! let html = render_template("page.html", &json!({"title": "Home"}))?;
//! # let _ = (user, port, html);
//! # Ok(())
//! # }
//! ```
//!
//! 安全检查变体（未初始化时返回 `None`）：
//!
//! ```rust,no_run
//! use alun::{try_cache, try_config, try_db};
//!
//! if let Some(_db) = try_db() { /* 数据库可用 */ }
//! if let Some(_cache) = try_cache() { /* 缓存可用 */ }
//! if let Some(_config) = try_config() { /* 配置可用 */ }
//! ```
//!
//! # 插件系统
//!
//! `Plugin` trait 定义生命周期：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! struct MyPlugin;
//!
//! #[async_trait::async_trait]
//! impl Plugin for MyPlugin {
//!     fn name(&self) -> &str { "my-plugin" }
//!     async fn start(&self) -> alun::Result<()> {
//!         tracing::info!("插件启动");
//!         Ok(())
//!     }
//!     async fn stop(&self) -> alun::Result<()> { Ok(()) }
//!     fn depends_on(&self) -> &[&str] { &["db"] } // 依赖数据库先启动
//! }
//!
//! // 注册
//! # async fn demo() -> Result<()> {
//! App::new()?
//!     .plugin(MyPlugin {})
//!     .scan()
//!     .start()
//!     .await
//! # }
//! ```
//!
//! 插件按依赖拓扑排序启动，反向停止。循环依赖会在启动时报错。
//!
//! ## 内置插件
//!
//! ```toml
//! [plugins]
//! enabled = ["cache", "scheduler", "notification", "serial"]
//! ```
//!
//! | 插件 | 功能 | 配置段 |
//! |------|------|--------|
//! | `cache` | 缓存（Local/Redis） | `[cache]` |
//! | `scheduler` | Cron 定时任务 | `[plugins.scheduler]` |
//! | `notification` | SMTP 邮件通知 | `[plugins.notification]` |
//! | `serial` | 分布式单号生成器 | `[plugins.serial]` |
//!
//! # 单号生成器 (Serial)
//!
//! 将"格式模板 + 循环周期 + 计数策略"抽象为可配置规则，支持内存/Redis/PG 三种后端。
//!
//! ```toml
//! # config.toml
//! [plugins.serial]
//! backend = "memory"
//!
//! [[plugins.serial.rules]]
//! key = "order"
//! format = "ORD{YYYY}{MM}{DD}{SEQ:8}"
//! cycle = "daily"
//! step = "sequential"
//! ```
//!
//! ```rust,no_run
//! use alun::{CyclePeriod, MemorySerialBackend, SerialGenerator, SerialRule};
//!
//! # async fn demo() -> std::result::Result<(), alun::SerialError> {
//! // 编程方式
//! let backend = MemorySerialBackend::new();
//! backend.register_rule(
//!     SerialRule::new("order", "ORD{YYYY}{MM}{DD}{SEQ:8}")
//!         .with_cycle(CyclePeriod::Daily)
//!         .with_initial_value(1)
//! ).await?;
//!
//! let no = backend.generate("order").await?;  // → "ORD2026061100000001"
//! let nos = backend.batch_generate("order", 5).await?;
//! let next = backend.peek("order").await?;    // 预览，不消耗计数器
//!
//! // 运行时管理
//! backend.enable_rule("order").await?;
//! backend.disable_rule("order").await?;
//! backend.remove_rule("order").await?;
//! # let _ = (no, nos, next);
//! # Ok(())
//! # }
//! ```
//!
//! ## 格式占位符
//!
//! | 占位符 | 输出 | 示例 |
//! |--------|------|------|
//! | `{YYYY}` | 4 位年份 | 2026 |
//! | `{YY}` | 2 位年份 | 26 |
//! | `{MM}` | 2 位月份 | 06 |
//! | `{DD}` | 2 位日期 | 11 |
//! | `{SEQ:n}` | n 位补零序号 | `{SEQ:6}` → 000001 |
//! | `{RAND:n}` | n 位随机数 | `{RAND:4}` → 5821 |
//! | `{TS}` | Unix 时间戳（秒） | 1716537600 |
//! | `{TSMS}` | Unix 时间戳（毫秒） | 1716537600123 |
//!
//! ## 自定义后端
//!
//! 实现 `SerialGenerator` trait，通过 `Arc<dyn SerialGenerator>` 注入 `SerialPlugin`：
//!
//! ```rust,no_run
//! use alun::{cfg, App, SerialError, SerialGenerator, SerialRecord, SerialRule};
//! use std::sync::Arc;
//!
//! // 实现 SerialGenerator trait
//! struct MyRedisBackend;
//!
//! # #[async_trait::async_trait]
//! # impl SerialGenerator for MyRedisBackend {
//! #     async fn generate(&self, _rule_key: &str) -> std::result::Result<String, SerialError> { unimplemented!() }
//! #     async fn batch_generate(&self, _rule_key: &str, _count: u32) -> std::result::Result<Vec<String>, SerialError> { unimplemented!() }
//! #     async fn peek(&self, _rule_key: &str) -> std::result::Result<String, SerialError> { unimplemented!() }
//! #     async fn register_rule(&self, _rule: SerialRule) -> std::result::Result<(), SerialError> { unimplemented!() }
//! #     async fn remove_rule(&self, _rule_key: &str) -> std::result::Result<(), SerialError> { unimplemented!() }
//! #     async fn enable_rule(&self, _rule_key: &str) -> std::result::Result<(), SerialError> { unimplemented!() }
//! #     async fn disable_rule(&self, _rule_key: &str) -> std::result::Result<(), SerialError> { unimplemented!() }
//! #     async fn query_records(&self, _rule_key: &str, _page: u64, _page_size: u64) -> std::result::Result<(Vec<SerialRecord>, u64), SerialError> { unimplemented!() }
//! #     async fn list_rules(&self) -> std::result::Result<Vec<SerialRule>, SerialError> { unimplemented!() }
//! # }
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let backend: Arc<dyn SerialGenerator> = Arc::new(MyRedisBackend);
//! let plugin = alun::SerialPlugin::new(cfg().plugins.serial.clone(), backend);
//! App::new()?.plugin(plugin).scan().start().await?;
//! # Ok(())
//! # }
//! ```
//!
//! # 缓存
//!
//! 需要 `features = ["cache"]`。
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! # #[derive(serde::Serialize, serde::Deserialize)] struct User { name: String }
//! # async fn demo(user: User) -> Result<()> {
//! // 读写
//! cache().set("user:1", &user).await?;
//! cache().set_ex("session:abc", &user, 3600).await?;  // 3600 秒后过期
//! let cached: Option<User> = cache().get("user:1").await?;
//!
//! // 递增
//! let count = cache().incr("page:views", 1).await?;
//!
//! // 模式匹配
//! let keys = cache().keys("session:*").await?;
//! let deleted = cache().delete_pattern("temp:*").await?;
//!
//! // 删除 / 检查
//! cache().del("user:1").await?;
//! let exists = cache().exists("user:2").await?;
//! # let _ = (cached, count, keys, deleted, exists);
//! # Ok(())
//! # }
//! ```
//!
//! ```toml
//! [cache]
//! type = "local"        # local | redis
//! max_capacity = 10000
//! default_ttl = 3600
//! ```
//!
//! # 中间件链
//!
//! 以下中间件按固定顺序执行，全部通过 `config.toml` 配置：
//!
//! ```text
//! SecurityHeaders -> CORS -> RequestLog -> RequestId -> Auth -> PermissionCheck -> RateLimit -> Compression -> Nonce -> Idempotency
//! ```
//!
//! ```toml
//! [middleware]
//! request_id = true
//! request_log = true
//!
//! [middleware.cors]
//! enabled = true
//! allow_origins = ["http://localhost:3000"]
//!
//! [middleware.rate_limit]
//! enabled = true
//! requests_per_window = 100
//! window_secs = 60
//! ```
//!
//! 自定义中间件注入（`layer_from_fn` 为**链外**；要用当前身份请改用 `route_layer`）：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! # async fn demo() -> Result<()> {
//! App::new()?
//!     // 链外：整站最外层，能记到 401/403/429 与真实耗时，但读不到身份
//!     .layer_from_fn(|router| {
//!         router.layer(axum::middleware::from_fn(my_middleware))
//!     })
//!     .start()
//!     .await
//! # }
//! # async fn my_middleware(
//! #     req: axum::http::Request<axum::body::Body>,
//! #     next: axum::middleware::Next,
//! # ) -> axum::response::Response {
//! #     next.run(req).await
//! # }
//! ```
//!
//! # 配置系统
//!
//! 配置加载顺序：`config.toml` → `config-{profile}.toml` → `ALUN_*` 环境变量。
//!
//! ```rust,no_run
//! use alun::{cfg, config};
//! use serde_json::json;
//!
//! // 静态配置
//! let port = cfg().server.listen.clone();
//! let db_enabled = cfg().database.enabled;
//!
//! // 动态配置（运行时读写）
//! config().set_dynamic("feature_x", json!(true));
//! let val = config().get_dynamic("feature_x");
//! config().remove_dynamic("feature_x");
//! # let _ = (port, db_enabled, val);
//! ```
//!
//! ```bash
//! # CLI
//! cargo run -- gen-config           # 生成默认配置文件
//! cargo run -- profile=prod          # 激活 prod 配置
//!
//! # 环境变量覆盖
//! ALUN_SERVER_LISTEN=3000 cargo run
//! ALUN_DATABASE_HOST=10.0.0.1 cargo run
//! ALUN_LOG_LEVEL=debug cargo run
//! ```
//!
//! # 异步任务系统
//!
//! 需要 `features = ["task"]`。Kafka 驱动的分布式任务，支持重试和死信队列。
//!
//! 该示例依赖非默认 feature（`alun_task` 仅在 `task` 下再导出、宏展开也引用它），
//! 故按文档约定用 `ignore` 只作示意，不参与默认 feature 下的 doctest 编译。
//!
//! ```rust,ignore
//! use alun::alun_task::TaskHandler;
//! use serde_json::{json, Value};
//!
//! #[alun::task_handler(task_type = 1, topic = "export", timeout_seconds = 60, max_retries = 3)]
//! struct ExportHandler;
//!
//! #[async_trait::async_trait]
//! impl TaskHandler for ExportHandler {
//!     fn task_type(&self) -> i16 { 1 }
//!     async fn execute(&self, payload: Value) -> std::result::Result<Value, String> {
//!         let user_id = payload["user_id"].as_str().unwrap_or("");
//!         // ... 导出逻辑 ...
//!         let _ = user_id;
//!         Ok(json!({"file_url": "https://...", "status": "completed"}))
//!     }
//! }
//! ```
//!
//! 任务生命周期：`PENDING → PROCESSING → COMPLETED / FAILED → (重试) → DEAD_LETTER`
//!
//! # 工具函数
//!
//! ## 加密
//!
//! ```rust,no_run
//! use alun::Crypto;
//!
//! # fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
//! let hash = Crypto::hash_password("pass123")?;               // Argon2 密码哈希
//! let ok = Crypto::verify_password("pass123", &hash)?;        // 自动识别 Argon2/BCrypt
//!
//! let key = Crypto::random_key();                             // 32 字节随机密钥
//! let (ciphertext, nonce) = Crypto::aes_encrypt(&key, "data").ok_or("加密失败")?;  // AES-256-GCM
//! let plaintext = Crypto::aes_decrypt(&key, &ciphertext, &nonce).ok_or("解密失败")?;
//!
//! let token = Crypto::random_token(32);                       // 随机十六进制令牌
//! # let _ = (ok, plaintext, token);
//! # Ok(())
//! # }
//! ```
//!
//! ## 数据脱敏
//!
//! ```rust,no_run
//! use alun::Mask;
//!
//! Mask::mobile("13812345678");            // "138****5678"
//! Mask::email("a@b.com");                 // "a***@b.com"
//! Mask::id_card("320112199001011234");    // "3201****1234"
//! Mask::bank_card("6222021234567890");    // "6222****7890"
//! Mask::name("张三丰");                    // "张**"
//! Mask::address("北京市朝阳区XX路");        // "北京市朝****"
//! Mask::mask_by_type("mobile", "13812345678");  // 按类型自动选择脱敏方式
//! ```
//!
//! ## ID 生成
//!
//! ```rust,no_run
//! use alun::Sid;
//!
//! Sid::short();    // 16 位十六进制
//! Sid::tiny();     // 8 位十六进制
//! Sid::uuid();     // UUID v4（36 字符，含连字符）
//! Sid::uuid7();    // UUID v7（36 字符，时间排序，推荐作为数据库主键）
//! Sid::tsid();     // 时间戳 + 随机数
//! ```
//!
//! ## 验证
//!
//! ```rust,no_run
//! use alun::Valid;
//!
//! Valid::is_email("a@b.com");                                  // true
//! Valid::is_mobile("13812345678");                              // true
//! Valid::is_uuid("550e8400-e29b-41d4-a716-446655440000");      // true
//! Valid::is_id_card("110101199003077790");                      // 含校验位
//! Valid::is_date("2024-01-01");                                 // true
//! Valid::is_datetime("2024-01-01T00:00:00Z");                   // true
//! Valid::is_url("https://example.com");                         // true
//! Valid::is_strong_password("Abcdefg1!");                       // 大小写+数字+特殊字符
//! Valid::is_json(r#"{"key":"value"}"#);                         // true
//! Valid::is_file_extension("photo.jpg", &["jpg", "png"]);       // true
//! ```
//!
//! # 文件存储
//!
//! 需要 `features = ["fs"]`。统一 StorageBackend trait，支持本地和 MinIO/S3：
//!
//! ```rust,no_run
//! use alun_fs::registry::BackendRegistry;
//! use alun_fs::types::BackendConfig;
//! use alun_fs::LocalFs;
//!
//! # async fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
//! // 注册后端并指定默认后端
//! let mut registry = BackendRegistry::new();
//! registry.register(LocalFs::new("uploads"), BackendConfig::default());
//! registry.with_default("local");
//!
//! let backend = registry.default_backend().ok_or("未配置默认后端")?;
//! let meta = backend.write("photo.jpg", b"binary data").await?;
//! let bytes = backend.read(&meta.stored_path).await?;
//! backend.delete(&meta.stored_path).await?;
//! # let _ = bytes;
//! # Ok(())
//! # }
//! ```
//!
//! # 项目结构推荐
//!
//! ```text
//! my_project/
//! ├── Cargo.toml           # alun = "0.3"
//! ├── config/
//! │   └── config.toml      # 框架配置
//! ├── src/
//! │   ├── main.rs          # App::new()?.scan().start()
//! │   ├── controllers/     # #[alun::controller] impl 块
//! │   ├── models/          # DTO 结构体（*Req, *Res, *Model）
//! │   ├── services/        # 业务逻辑
//! │   └── plugins/         # 自定义 Plugin 实现
//! ├── templates/           # Jinja2 模板文件
//! ├── migrations/          # 数据库迁移（NNN_desc.up.sql）
//! ├── uploads/             # 文件上传目录
//! └── downloads/           # 文件下载目录
//! ```
//!
//! # 宏参考
//!
//! | 宏 | 用途 |
//! |----|------|
//! | `#[alun::get("/path")]` | 注册 GET 处理器，由 `scan()` 自动收集 |
//! | `#[alun::post("/path")]` | 注册 POST 处理器 |
//! | `#[alun::put("/path")]` | 注册 PUT 处理器 |
//! | `#[alun::delete("/path")]` | 注册 DELETE 处理器 |
//! | `#[alun::controller("/prefix")]` | 在路径前缀下分组方法 |
//! | `#[alun::permission(path, method, permission)]` | 方法级权限检查 |
//! | `#[alun::no_auth("/path")]` | 标记路径免认证 |
//! | `#[alun::task_handler(task_type, topic, ...)]` | 自动注册异步任务处理器 |
//! | `#[alun::plugin]` | 自动注册插件类型 |
//!
//! 所有宏通过 `inventory::submit!` 在编译期收集——无运行时反射开销，且不依赖链接器自定义段。
//!
//! # 命名约定
//!
//! - 布尔变量/函数以 `is_`/`has_`/`can_` 开头：`is_enabled`、`has_permission`、`can_notarize`
//! - 枚举值蛇形命名，枚举类型帕斯卡命名：`enum TokenType { Access, Refresh }`
//! - Model 后缀 `*Model`，请求 DTO 后缀 `*Req`，返回 DTO 后缀 `*Res`
//! - 公共 API 必须添加 `///` 文档注释
//! - 禁止 `unwrap()`/`expect()`——所有错误必须显式处理
//!
//! # 重要提示
//!
//! - `ApiError::internal(msg)` 自动屏蔽错误，前端看到 `"服务器内部错误"`，真实错误仅写入日志
//! - `Row` 不做变更追踪：`db().update_row(&row)` 按 `row.primary_key()` 定位并整行写回；
//!   只想更新部分列时用 `ChangeSet`（`Model::of(id).set(field, v).update(&db)`）
//! - `db().paginate(...)`（以及 `db().page(page, size).exec(&db)`）以 `SELECT COUNT(*) FROM ({sql}) AS _count` 统计总数，传入的 SQL 必须是合法子查询
//! - 使用 `ValidatedJson<T>` 时，`T` 需实现 `validator::Validate`——校验在处理器执行前完成
//! - 自定义 `validate_*` 函数空值默认跳过（除 `validate_password_strength`）
//! - 使用 `serialize` feature 的 crate 必须在 `Cargo.toml` 中显式声明 `serde` 依赖

pub mod prelude {
    /// 骨架层错误：启动流程 / 插件 / 定时任务等非 HTTP 场景用 `alun::Result<T>`
    pub use alun_core::{Error, Plugin, PluginManager, Result};
    /// 统一出口：handler 的签名、成功值、业务报错
    pub use alun_web::{
        DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, ApiError, PageData, PageQuery, Res, ResResult, SortOrder,
        SortQuery, codes,
    };
    pub use alun_web::resources::{cfg, config, set_config, try_config};

    #[cfg(feature = "db")]
    pub use alun_web::resources::{db, db_named, set_db, set_db_named, try_db, try_db_named};

    #[cfg(feature = "cache")]
    pub use alun_web::resources::{cache, set_cache, try_cache};
    #[cfg(all(feature = "cache", feature = "db"))]
    pub use alun_web::resources::cache_tenant;
    #[cfg(feature = "sse")]
    pub use alun_web::sse::{Broadcaster, sse_json};

    #[cfg(feature = "template")]
    pub use alun_web::resources::{render_template, set_template, try_template};

    pub use alun_config::AppConfig;
    #[cfg(feature = "xss")]
    pub use alun_utils::xss;
    pub use alun_utils::{Crypto, Mask, Sid, Valid};
    pub use alun_utils::export::{Export, Import};
    #[cfg(feature = "xlsx")]
    pub use alun_utils::export::XlsxSheet;
    #[cfg(feature = "cache")]
    pub use alun_web::middleware::{IdempotencyLayer, NonceLayer};
    pub use alun_web::{
        AlunRouter, App, AuthClaims, ClaimsBuilder, JWT, Principal, TokenClaims, TokenType, UserId,
        ValidatedJson,
    };

    /// `alun-db` 的完整公开 API：`Model` 派生宏、`CrudExt` / `PersistExt`、
    /// `ModelQuery` / `ChangeSet`、`sql!`、`Expr` / `Value`、事务与迁移……
    ///
    /// 只依赖 `alun` 即可写 Model，**不需要**在 `Cargo.toml` 里声明
    /// `alun-db` 或 `alun-db-core`。
    ///
    /// 例外：其中的 `init` / `init_with` 属于 `alun-db` 的独立用法（操作
    /// `alun-db` 自带的全局实例），与本框架 `db()` 指向的实例不是同一个，框架内请勿调用。
    #[cfg(feature = "db")]
    pub use alun_db::prelude::*;

    /// 数据访问层的返回类型：错误是 **`DbError`**（`alun-db-core` 的类型）。
    ///
    /// 它**只在数据访问层出现**：handler 用 `ResResult<T>` 时 `DbError` 可以直接 `?`
    /// 上来（映射内置），所以业务代码不需要把 `DbError` 写在签名里。
    /// 非 HTTP 场景（启动 / 插件 / 定时任务）的 `alun::Result<T>` **收不下**
    /// `DbError`——那里仍用 `DbResult<T>` 或显式处理。
    ///
    /// 与 `alun::Result` 一样是单参别名；开启 `db` 后自动可用，无需额外依赖。
    #[cfg(feature = "db")]
    pub use alun_db::__db_core::prelude::Result as DbResult;

    #[cfg(feature = "db")]
    pub use alun_web::PageResponseExt;

    pub use serde_json::{Value as JsonValue, json};

    #[cfg(feature = "template")]
    pub use alun_template::TemplateEngine;

    #[cfg(feature = "cache")]
    pub use alun_cache::{Cache, CacheStats, LocalCache};

    #[cfg(feature = "plugin")]
    pub use alun_plugin;

    pub use axum::Extension;
    pub use axum::extract::{Path, Query};
    pub use axum::response::Json as AxumJson;
}

pub use alun_macros::{
    controller, delete, get, layer, no_auth, patch, permission, plugin, post, put, routes,
    task_handler,
};

/// 工具类型（字符串/加密/脱敏/ID/校验/单号）
pub use alun_utils::{
    Crypto, CyclePeriod, FormatEngine, IncrementStrategy, Mask, MemorySerialBackend, SerialError,
    SerialGenerator, SerialRecord, SerialRule, Sid, Valid,
};

/// 导出导入（CSV / JSON / 真 XLSX）
///
/// `Export` / `Import` 一直存在；`XlsxSheet`（多 sheet 工作簿）与
/// `Export::to_xlsx_*` / `Import::from_xlsx*` 需要 `xlsx` feature。
pub use alun_utils::export::{Export, Import};
#[cfg(feature = "xlsx")]
pub use alun_utils::export::XlsxSheet;

/// HTML 净化（需 `xss` feature）；也可经 prelude 直接 `xss::sanitize_html(..)`。
#[cfg(feature = "xss")]
pub use alun_utils::xss;

pub use alun_core::{Error, Result};
pub use alun_web::{ApiError, PageData, PageQuery, Res, ResResult, codes};
#[cfg(feature = "plugin")]
pub use alun_plugin::serial_plugin::SerialPlugin;
pub use alun_web::resources::{cfg, config, set_config, try_config};
pub use alun_web::resources::{
    download_path, set_download_path, set_upload_path, try_download_path, try_upload_path,
    upload_path,
};

#[cfg(feature = "db")]
pub use alun_web::resources::{db, db_named, set_db, set_db_named, try_db, try_db_named};

#[cfg(feature = "cache")]
pub use alun_web::resources::{cache, set_cache, try_cache};
#[cfg(all(feature = "cache", feature = "db"))]
pub use alun_web::resources::cache_tenant;

#[cfg(feature = "sse")]
pub use alun_web::sse;
#[cfg(feature = "sse")]
pub use alun_web::sse::{Broadcaster, sse_json};
/// F10 可观测性：`metrics::MetricsLayer`、`metrics::install_prometheus` 等
#[cfg(feature = "metrics")]
pub use alun_web::metrics;

#[cfg(feature = "template")]
pub use alun_web::resources::{render_template, set_template, try_template};

#[cfg(feature = "db")]
pub use alun_db::{Db, Page, Row};
/// 重导出 `async_trait` 属性宏：实现 `AsyncHook` 时写 `#[alun::async_trait]`，
/// 无需自行依赖 `async-trait`（普通项目只需声明 `alun`）。
#[cfg(feature = "db")]
pub use alun_db::async_trait;

/// 数据库内核的内部转发，供过程宏生成代码解析 `::alun::__db_core::…`。
#[cfg(feature = "db")]
#[doc(hidden)]
pub use alun_db::__db_core;

/// `alun-db` 门面的内部转发，供过程宏生成代码解析 `::alun::__db::…`。
#[cfg(feature = "db")]
#[doc(hidden)]
pub use alun_db as __db;

pub use alun_config::AppConfig;
#[cfg(feature = "db")]
pub use alun_web::PageResponseExt;
pub use alun_web::ValidateExt;
pub use alun_web::{
    AlunRouter, App, AuthClaims, ClaimsBuilder, JWT, NoAuthDef, PermissionDef, RouteEntry,
    TokenClaims, TokenType, UserId, ValidatedJson, WebPlugin, discovered_no_auth,
    discovered_permissions, discovered_routes,
};
pub use alun_web::{
    validate_date, validate_date_or_datetime, validate_datetime, validate_email, validate_id_card,
    validate_mobile, validate_password_strength, validate_url, validate_uuid,
};
/// `serde_json` 的内部转发，供过程宏生成代码解析 `::alun::__serde_json::…`。
#[doc(hidden)]
pub use serde_json as __serde_json;

/// `inventory` 的内部转发：过程宏生成的路由 / 权限 / 免鉴权注册项写
/// `::alun::inventory::submit! { … }`，用户项目因此不必自己声明 `inventory`。
pub use inventory;
pub use uuid;

/// Web 子模块（完整导出）
pub use alun_web as web;

#[cfg(feature = "kafka")]
pub use alun_kafka;

#[cfg(feature = "task")]
pub use alun_task;

#[cfg(feature = "fs")]
pub use alun_fs;

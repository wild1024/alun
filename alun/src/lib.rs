//! # Alun —— 快、简、美的 Rust Web 框架
//!
//! Alun 是一个**配置驱动、插件架构**的 Rust Web 框架，基于 [axum] 0.8 构建。
//! 核心理念：**一行启动，零成本抽象，约定优于配置**。
//!
//! ## 设计原则
//!
//! - **配置驱动**：数据库、缓存、认证、CORS、限流、压缩等基础设施行为全部在 `config.toml` 中声明
//! - **全局资源单例**：通过 `db()`、`cache()`、`cfg()` 访问资源，无需 `State` 注入
//! - **编译期路由发现**：`#[get]`/`#[post]` 宏通过 `linkme` 在编译期收集处理器，无运行时反射
//! - **插件拓扑排序**：插件声明依赖关系，按依赖顺序启动，反向停止
//! - **Row 模式 CRUD**：无 ORM，用 `HashMap<String, Value>` 加变更追踪实现统一 CRUD
//! - **RAII 事务**：Rust `Drop` + `?` 保证事务回滚，编译器强制安全检查
//!
//! ## 快速开始
//!
//! 在 `Cargo.toml` 中添加依赖：
//!
//! ```toml
//! [dependencies]
//! alun = "0.1"            # 最小 web 功能
//! alun = { version = "0.1", features = ["full"] }  # 全功能
//! ```
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
//! ```toml
//! alun = { default-features = false, features = [] }              # 最小 web
//! alun = { default-features = false, features = ["db"] }          # + 数据库
//! alun = { default-features = false, features = ["db", "cache"] } # + 缓存
//! alun = { features = ["task"] }                                  # Kafka 异步任务
//! alun = { features = ["fs"] }                                    # 文件存储
//! alun = { features = ["xss"] }                                   # XSS 净化
//! alun = { features = ["full"] }                                  # 全部功能（默认）
//! ```
//!
//! | 功能 | 启用模块 | 描述 |
//! |------|---------|------|
//! | `db` | `alun_db` | 数据库 CRUD，Row 模式，事务，钩子，迁移 |
//! | `cache` | `alun_cache` | 缓存抽象（本地内存 / Redis） |
//! | `template` | `alun_template` | Jinja2 模板引擎（minijinja） |
//! | `plugin` | `alun_plugin` | 插件基础设施（定时任务、通知、单号） |
//! | `task` | `alun_task` | Kafka 分布式异步任务（重试 + 死信队列） |
//! | `kafka` | `alun_kafka` | Kafka 生产者/消费者 |
//! | `fs` | `alun_fs` | 文件存储抽象（本地 / MinIO/S3） |
//! | `xss` | `alun_utils::xss` | XSS 净化 |
//! | `web` | `alun_core::api` | `IntoResponse` 实现 |
//! | `full` | 以上全部 | 一键启用所有功能 |
//!
//! # 路由注册
//!
//! ## 方式一：Proc 宏 + scan()（推荐）
//!
//! 零手动注册，`scan()` 自动发现所有带 `#[alun::get]`/`#[alun::post]` 等注解的函数：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
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
//! ) -> Result<Res<UserRes>, ApiError> {
//!     Ok(Res::ok(UserRes { id: "1".into(), name: req.name }))
//! }
//!
//! #[alun::put("/api/users/{id}")]
//! async fn update_user(
//!     Path(id): Path<String>,
//!     ValidatedJson(req): ValidatedJson<UpdateUserReq>,
//! ) -> Res<()> {
//!     Res::ok_empty()
//! }
//!
//! #[alun::delete("/api/users/{id}")]
//! async fn delete_user(Path(id): Path<String>) -> Res<()> {
//!     Res::ok_empty()
//! }
//!
//! # use serde::Deserialize;
//! # #[derive(Deserialize)] struct CreateUserReq { name: String }
//! # #[derive(Deserialize)] struct UpdateUserReq { name: String }
//! ```
//!
//! ## 方式二：Controller 分组
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! #[alun::controller("/api/admin")]
//! struct AdminController;
//!
//! impl AdminController {
//!     #[get("/dashboard")]
//!     async fn dashboard() -> Res<String> {
//!         Res::ok("admin dashboard".into())
//!     }
//!
//!     #[delete("/users/{id}")]
//!     async fn delete_user(Path(id): Path<String>) -> Result<Res<()>, ApiError> {
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
//! App::new()?
//!     .get("/api/users", list_users)
//!     .post("/api/users", create_user)
//!     .group("/api/v2", |app| {
//!         app.get("/hello", hello_v2)
//!     })
//!     .start().await
//! # async fn list_users() -> Res<String> { Res::ok("".into()) }
//! # async fn create_user() -> Res<String> { Res::ok("".into()) }
//! # async fn hello_v2() -> Res<String> { Res::ok("".into()) }
//! # ;
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
//! Res::ok(user)                                      // { code:0, msg:"ok", data:user }
//! Res::ok_with_msg(user, "创建成功")                  // 自定义消息
//! Res::ok_empty()                                    // 无 data 字段
//! Res::ok_msg("操作成功")                             // 仅消息
//! Res::fail(codes::BAD_REQUEST, "参数不能为空")       // 自定义错误码
//! Res::page(list, total_count, page, page_size)       // 分页响应
//! ```
//!
//! ## `ApiError` — 结构化 HTTP 错误
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! // 返回 Result<Res<T>, ApiError>
//! async fn handler() -> Result<Res<String>, ApiError> {
//!     Err(ApiError::bad_request("参数错误"))
//! }
//!
//! // 便捷构造器
//! ApiError::bad_request("参数错误");              // 400
//! ApiError::unauthorized("请先登录");              // 401
//! ApiError::forbidden("权限不足");                 // 403
//! ApiError::not_found("用户不存在");               // 404
//! ApiError::conflict("用户名已存在");              // 409
//! ApiError::unprocessable_entity("邮箱格式不正确"); // 422
//! ApiError::too_many_requests("请求过于频繁");     // 429
//! ApiError::internal("数据库连接失败");            // 500（前端看到"服务器内部错误"）
//! ApiError::internal_masked("服务器内部错误", "..."); // 自定义前端消息 + 内部详情
//! ApiError::service_unavailable("服务暂不可用");   // 503
//! ```
//!
//! > **v0.1.1+**: `ApiError::internal(msg)` 自动屏蔽错误详情——前端永远看到 `"服务器内部错误"`，真实错误写入日志。用 `internal_masked(public, detail)` 自定义前端提示。
//!
//! ## `PageQuery` — 分页参数
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! let pq = PageQuery::new(1, 20);   // page=1, page_size=20（自动校验范围）
//! let offset = pq.offset();         // (page-1) * page_size = 0
//! let limit = pq.limit();           // 20
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
//! ) -> Result<Res<()>, ApiError> {
//!     // req 已自动校验，失败返回 422
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
//! use alun::ValidateExt;
//! use alun::ApiError;
//!
//! fn process(req: &ComplexReq) -> Result<(), ApiError> {
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
//! // 插入
//! let row = Row::table("users")
//!     .id(Sid::uuid7())                          // UUID v7 主键（推荐）
//!     .set("name", "张三")
//!     .set("email", "zhangsan@example.com")
//!     .set("age", 28);
//! let inserted: Row = db().insert(&row).await?;
//!
//! // 按 ID 查询（自动检测 ID 类型：uuid / i64 / 字符串）
//! let user: Option<Row> = db().find_by_id("users", "user_id").await?;
//!
//! // 更新（仅 tracked changes 发送到数据库）
//! let mut row = db().find_by_id("users", "id").await?.unwrap();
//! row.set("age", 29);   // 仅此字段进入 UPDATE SET
//! let updated = db().update(&row).await?;
//! row.clear_changes();  // 清除跟踪，后续 set() 只含新字段
//!
//! // 删除
//! db().delete_by_id("users", "id").await?;
//!
//! // 原始 SQL 查询
//! let rows: Vec<Row> = db().query("SELECT * FROM users WHERE age > $1", &["18"]).await?;
//! let count: u64 = db().count("SELECT COUNT(*) FROM users", &[]).await?;
//!
//! // 分页查询
//! let (rows, total) = db().query_page(
//!     "SELECT * FROM users ORDER BY created_at DESC",
//!     &[],
//!     &PageQuery::new(1, 20),
//! ).await?;
//!
//! // 批量插入
//! let rows = vec![
//!     Row::table("users").id(Sid::uuid7()).set("name", "A"),
//!     Row::table("users").id(Sid::uuid7()).set("name", "B"),
//! ];
//! db().batch_insert(&rows).await?;
//! # Ok::<(), alun::Error>(())
//! ```
//!
//! ## Row 字段访问
//!
//! ```rust,no_run
//! row.get("name")              // Option<&Value>
//! row.get_as::<String>("name") // Option<String>
//! row.get_as::<i64>("age")     // Option<i64>
//! row.get_id()                 // Option<&Value> — 主键值
//! row.has("field")             // bool — 字段是否存在
//! row.table                    // Option<String> — 表名
//! ```
//!
//! 自定义主键：`.primary_key("code").id("val")`；联合主键：`.primary_keys(&["k1", "k2"])`。
//!
//! ## RAII 事务
//!
//! 返回 `Ok` 自动提交，返回 `Err` 自动回滚。支持强制回滚：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! db().transaction(|mut tx| async move {
//!     tx.execute("UPDATE accounts SET balance = balance - 100 WHERE id = $1", &["A"]).await?;
//!     tx.execute("UPDATE accounts SET balance = balance + 100 WHERE id = $1", &["B"]).await?;
//!
//!     let row = tx.query_one("SELECT balance FROM accounts WHERE id = $1", &["A"]).await?;
//!     if row.and_then(|r| r.get_as::<i64>("balance")).unwrap_or(0) < 0 {
//!         tx.set_rollback_only();  // 强制回滚
//!     }
//!     Ok(())  // 提交（除非 set_rollback_only）
//! }).await?;
//! # Ok::<(), alun::Error>(())
//! ```
//!
//! ## 钩子 (Hooks)
//!
//! ```rust,no_run
//! use alun::prelude::*;
//! use alun::{Hook, HookChain};
//!
//! struct TimestampHook;
//! #[async_trait::async_trait]
//! impl Hook for TimestampHook {
//!     async fn before_insert(&self, row: &mut Row) -> DbResult<()> {
//!         row.set("created_at", chrono::Utc::now().to_rfc3339());
//!         Ok(())
//!     }
//!     async fn before_update(&self, row: &mut Row) -> DbResult<()> {
//!         row.set("updated_at", chrono::Utc::now().to_rfc3339());
//!         Ok(())
//!     }
//! }
//!
//! let hooks = HookChain::new()
//!     .add(TimestampHook {});
//! ```
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
//! use alun::JWT;
//!
//! let jwt = JWT::from_config();  // 从 config.toml 读取 secret + 过期时间
//!
//! // 登录——生成令牌
//! let access = jwt.create_access_token(
//!     "user_123",                     // user_id
//!     Some("username"),               // 用户名
//!     &["admin".into()],              // 角色
//!     &["user:read".into()],          // 权限
//! )?;
//! let refresh = jwt.create_refresh_token("user_123")?;
//!
//! // 验证
//! let claims: TokenClaims = jwt.validate(&access_token)?;
//!
//! // 刷新（旧 refresh_token 自动加黑名单）
//! let (new_access, new_refresh) = jwt.refresh(&old_refresh).await?;
//!
//! // 登出（加黑名单）
//! jwt.logout(&claims).await;
//! # Ok::<(), alun::Error>(())
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
//! claims.has_role("admin")                                     // bool
//! claims.has_any_role(&["admin", "moderator"])                 // bool
//! claims.has_permission("user:write")                          // bool
//! claims.has_any_permission(&["user:read", "user:write"])      // bool
//! claims.is_super_admin()                                      // 拥有 "*" 权限
//! # }
//! ```
//!
//! ## 权限守卫
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! // 构建器方式
//! App::new()?
//!     .with_permission("GET", "/api/admin/stats", admin_stats, "admin:access")
//!     .with_role("DELETE", "/api/users/{id}", delete_user, "admin")
//!     .start().await
//! # ;async fn admin_stats() -> Res<String> { Res::ok("".into()) }
//! # async fn delete_user() -> Result<Res<()>, ApiError> { Ok(Res::ok_empty()) }
//! # ;
//!
//! // 宏方式
//! #[alun::permission(path = "/api/admin/users", method = "GET", permission = "admin:read")]
//! #[alun::get("/api/admin/users")]
//! async fn list_users() -> Res<Vec<JsonValue>> { Res::ok(vec![]) }
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
//! use alun::{db, cache, cfg, config, render_template};
//!
//! // 数据库（需要 features = ["db"]）
//! let user = db().find_by_id("users", "id").await?;
//!
//! // 配置（静态只读）
//! let port = cfg().server.listen;
//!
//! // 缓存（需要 features = ["cache"]）
//! cache().set_ex("key", &value, 3600).await?;
//!
//! // 模板（需要 features = ["template"]）
//! let html = render_template("page.html", &json!({"title": "Home"}))?;
//! # Ok::<(), alun::Error>(())
//! ```
//!
//! 安全检查变体（未初始化时返回 `None`）：
//!
//! ```rust,no_run
//! use alun::{try_db, try_cache, try_config};
//!
//! if let Some(db) = try_db() { /* 数据库可用 */ }
//! if let Some(cache) = try_cache() { /* 缓存可用 */ }
//! # Ok::<(), alun::Error>(())
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
//! App::new()?
//!     .plugin(MyPlugin {})
//!     .scan()
//!     .start()
//!     .await
//! # ;
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
//! use alun::{SerialRule, SerialGenerator, MemorySerialBackend, CyclePeriod};
//!
//! // 编程方式
//! let gen = MemorySerialBackend::new();
//! gen.register_rule(
//!     SerialRule::new("order", "ORD{YYYY}{MM}{DD}{SEQ:8}")
//!         .with_cycle(CyclePeriod::Daily)
//!         .with_initial_value(1)
//! ).await?;
//!
//! let no = gen.generate("order").await?;  // → "ORD2026061100000001"
//! let nos = gen.batch_generate("order", 5).await?;
//! let next = gen.peek("order").await?;    // 预览，不消耗计数器
//!
//! // 运行时管理
//! gen.enable_rule("order").await?;
//! gen.disable_rule("order").await?;
//! gen.remove_rule("order").await?;
//! # Ok::<(), alun_utils::SerialError>(())
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
//! use alun::{SerialGenerator, SerialRule, SerialRecord, SerialError};
//! use std::sync::Arc;
//!
//! // 实现 SerialGenerator trait
//! struct MyRedisBackend { /* ... */ }
//!
//! # #[async_trait::async_trait]
//! # impl SerialGenerator for MyRedisBackend {
//! #     async fn generate(&self, _rule_key: &str) -> Result<String, SerialError> { unimplemented!() }
//! #     async fn batch_generate(&self, _rule_key: &str, _count: u32) -> Result<Vec<String>, SerialError> { unimplemented!() }
//! #     async fn peek(&self, _rule_key: &str) -> Result<String, SerialError> { unimplemented!() }
//! #     async fn register_rule(&self, _rule: SerialRule) -> Result<(), SerialError> { unimplemented!() }
//! #     async fn remove_rule(&self, _rule_key: &str) -> Result<(), SerialError> { unimplemented!() }
//! #     async fn enable_rule(&self, _rule_key: &str) -> Result<(), SerialError> { unimplemented!() }
//! #     async fn disable_rule(&self, _rule_key: &str) -> Result<(), SerialError> { unimplemented!() }
//! #     async fn query_records(&self, _rule_key: &str, _page: u64, _page_size: u64) -> Result<(Vec<SerialRecord>, u64), SerialError> { unimplemented!() }
//! #     async fn list_rules(&self) -> Result<Vec<SerialRule>, SerialError> { unimplemented!() }
//! # }
//!
//! let backend: Arc<dyn SerialGenerator> = Arc::new(MyRedisBackend { /* ... */ });
//! let plugin = alun::SerialPlugin::new(cfg().plugins.serial.clone(), backend);
//! App::new()?.plugin(plugin).scan().start().await;
//! # ;
//! ```
//!
//! # 缓存
//!
//! 需要 `features = ["cache"]`。
//!
//! ```rust,no_run
//! use alun::cache;
//!
//! // 读写
//! cache().set("user:1", &user).await?;
//! cache().set_ex("session:abc", &session, 3600).await?;  // 5 分钟过期
//! let user: Option<User> = cache().get("user:1").await?;
//!
//! // 递增
//! let count = cache().incr("page:views", 1).await?;
//!
//! // 模式匹配
//! let keys = cache().keys("session:*").await?;
//! cache().delete_pattern("temp:*").await?;
//!
//! // 删除 / 检查
//! cache().del("user:1").await?;
//! let exists = cache().exists("user:2").await?;
//! # #[derive(serde::Serialize, serde::Deserialize)] struct User { name: String }
//! # let user = User { name: "".into() };
//! # let session = user;
//! # Ok::<(), alun::Error>(())
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
//! SecurityHeaders -> RequestLog -> RequestId -> CORS -> Compression -> RateLimit -> Auth -> PermissionCheck
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
//! 自定义中间件注入：
//!
//! ```rust,no_run
//! use alun::prelude::*;
//!
//! App::new()?
//!     .with_middleware_hook(|router| {
//!         router.layer(axum::middleware::from_fn(my_middleware))
//!     })
//!     .start().await
//! # ;
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
//!
//! // 静态配置
//! let port = cfg().server.listen;
//! let db_enabled = cfg().database.enabled;
//!
//! // 动态配置（运行时读写）
//! config().set_dynamic("feature_x", json!(true));
//! let val = config().get_dynamic("feature_x");
//! config().remove_dynamic("feature_x");
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
//! ```rust,no_run
//! use alun::prelude::*;
//! use serde_json::Value;
//!
//! #[alun::task_handler(task_type = 1, topic = "export", timeout_seconds = 60, max_retries = 3)]
//! struct ExportHandler;
//!
//! #[async_trait::async_trait]
//! impl TaskHandler for ExportHandler {
//!     fn task_type(&self) -> i16 { 1 }
//!     async fn execute(&self, payload: Value) -> Result<Value, String> {
//!         let user_id = payload["user_id"].as_str().unwrap_or("");
//!         // ... 导出逻辑 ...
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
//! let hash = Crypto::hash_password("pass123")?;             // Argon2
//! let ok = Crypto::verify_password("pass123", &hash)?;       // 自动识别 Argon2/BCrypt
//! let encrypted = Crypto::aes_encrypt("data", &key)?;       // AES-256-GCM
//! let decrypted = Crypto::aes_decrypt(&encrypted, &key)?;
//! let token = Crypto::random_token(32);                     // 随机十六进制令牌
//! # Ok::<(), alun::Error>(())
//! ```
//!
//! ## 数据脱敏
//!
//! ```rust,no_run
//! use alun::Mask;
//!
//! Mask::phone("13812345678");             // "138****5678"
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
//! use alun_fs::{StorageBackend, LocalFs, FileMeta, BackendRegistry, BackendConfig};
//!
//! let mut registry = BackendRegistry::new()
//!     .register(LocalFs::new("uploads"), BackendConfig::default())
//!     .with_default("local");
//!
//! let backend = registry.default_backend().unwrap();
//! let meta = backend.write("photo.jpg", b"binary data").await?;
//! let bytes = backend.read(&meta.stored_path).await?;
//! backend.delete(&meta.stored_path).await?;
//! # Ok::<(), String>(())
//! ```
//!
//! # 项目结构推荐
//!
//! ```text
//! my_project/
//! ├── Cargo.toml           # alun = "0.1"
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
//! 所有宏通过 `linkme::distributed_slice` 在编译期收集——无运行时反射开销。
//!
//! # 命名约定
//!
//! - 布尔变量/函数以 `is_`/`has_`/`can_` 开头：`is_enabled`、`has_permission`、`can_notarize`
//! - 枚举值蛇形命名，枚举类型帕斯卡命名：`enum IdKind { Uuid, I64, F64 }`
//! - Model 后缀 `*Model`，请求 DTO 后缀 `*Req`，返回 DTO 后缀 `*Res`
//! - 公共 API 必须添加 `///` 文档注释
//! - 禁止 `unwrap()`/`expect()`——所有错误必须显式处理
//!
//! # 重要提示
//!
//! - `ApiError::internal(msg)` 自动屏蔽错误，前端看到 `"服务器内部错误"`，真实错误仅写入日志
//! - `Row.changes` 追踪被修改的字段，UPDATE 查询只发送变更的字段。`clear_changes()` 清除追踪
//! - `Db::query_page()` 将 SQL 包装为 `SELECT COUNT(*) FROM ({sql}) AS _count_sub`，确保 SQL 是合法子查询
//! - 使用 `ValidatedJson<T>` 时，`T` 需实现 `validator::Validate`——校验在处理器执行前完成
//! - 自定义 `validate_*` 函数空值默认跳过（除 `validate_password_strength`）
//! - 使用 `serialize` feature 的 crate 必须在 `Cargo.toml` 中显式声明 `serde` 依赖

pub mod prelude {
    pub use alun_core::{Result, Error, Plugin, PluginManager, Res, ResResult, ApiError, PageData, PageQuery, codes};
    pub use alun_web::resources::{cfg, config, try_config, set_config};

    #[cfg(feature = "db")]
    pub use alun_web::resources::{db, try_db, set_db};

    #[cfg(feature = "cache")]
    pub use alun_web::resources::{cache, try_cache, set_cache};

    #[cfg(feature = "template")]
    pub use alun_web::resources::{render_template, try_template, set_template};

    pub use alun_web::{App, AlunRouter, TokenClaims, TokenType, UserId, AuthClaims, ValidatedJson, JWT};
    pub use alun_web::middleware::{NonceLayer, IdempotencyLayer};
    pub use alun_config::AppConfig;

    #[cfg(feature = "db")]
    pub use alun_db::{Db, Row, ActiveTx, Isolation, Hook, NullHook, HookChain, factory};

    pub use serde_json::{json, Value as JsonValue};

    #[cfg(feature = "template")]
    pub use alun_template::TemplateEngine;

    #[cfg(feature = "cache")]
    pub use alun_cache::{Cache, CacheStats, LocalCache};

    #[cfg(feature = "plugin")]
    pub use alun_plugin;

    pub use axum::response::Json as AxumJson;
    pub use axum::extract::{Path, Query};
    pub use axum::Extension;
}

pub use alun_macros::{get, post, put, delete, controller, plugin, permission, task_handler};
pub use alun_core::{Result, Error, Res, ApiError};
pub use alun_web::resources::{cfg, config, try_config, set_config};

#[cfg(feature = "db")]
pub use alun_web::resources::{db, try_db, set_db};

#[cfg(feature = "cache")]
pub use alun_web::resources::{cache, try_cache, set_cache};

#[cfg(feature = "template")]
pub use alun_web::resources::{render_template, try_template, set_template};

pub use alun_db::{Db, Row, IdKind};
pub use alun_web::{App, AlunRouter, TokenClaims, TokenType, UserId, AuthClaims, ValidatedJson, JWT, ROUTES, PermissionDef, PERMISSION_ROUTES, NoAuthDef, NO_AUTH_ROUTES};
pub use alun_web::{validate_uuid, validate_mobile, validate_password_strength, validate_id_card, validate_date, validate_email, validate_url, validate_datetime, validate_date_or_datetime};
pub use alun_web::ValidateExt;
pub use alun_config::AppConfig;

pub use linkme::distributed_slice;
pub use uuid;

/// Web 子模块（完整导出）
pub use alun_web as web;

#[cfg(feature = "kafka")]
pub use alun_kafka;

#[cfg(feature = "task")]
pub use alun_task;

#[cfg(feature = "fs")]
pub use alun_fs;

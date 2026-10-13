//! 文档代码块编译校验（`cargo test -p alun-docs-tests --doc`）
//!
//! 把仓库里所有 Markdown 文档作为模块文档 `include_str!` 进来，rustdoc 会**真实编译**
//! 其中每个 ```` ```rust ```` 代码块（`ignore` 标记的除外），从而保证文档里的 API 与
//! 真实代码一致、示例不会漂移。
//!
//! - 新增文档时：在下面加一个 `#[doc = include_str!("../../<path>")]` 模块即可。
//! - 模块命名：编号文档 `docs/NN-*.md` 用 `dNN`（与文件编号一一对应，改编号时同步改）；
//!   非编号文档用描述名（`docs_index` / `recommended_layout` / `examples_report`）。
//! - 示例约定：可编译的用 `rust,no_run`（顶层 `.await` 放进隐藏行 `# async fn demo() { }`）；
//!   纯示意片段用 `rust,ignore`；其他语言必须显式打标记（不打标记会被当作 Rust 编译）。
//!
//! 本 crate 不发布（`publish = false`），只用于测试。
#![allow(rustdoc::broken_intra_doc_links, rustdoc::invalid_rust_codeblocks)]

/// 文档：`README.md`
#[doc = include_str!("../../README.md")]
pub mod d00 {}

/// 文档：`docs/guides/getting-started.md`（入门总目录）
#[doc = include_str!("../../docs/guides/getting-started.md")]
pub mod d01 {}

// 以下是 `docs/guides/startup/` 下按流程步骤拆分的入门分册，
// 承载原 `d01` 的真实代码块，必须继续纳入 rustdoc 编译以防示例漂移。
/// 文档：`docs/guides/startup/setup.md`（环境与依赖）
#[doc = include_str!("../../docs/guides/startup/setup.md")]
pub mod d01_setup {}

/// 文档：`docs/guides/startup/config-basics.md`（配置与启动）
#[doc = include_str!("../../docs/guides/startup/config-basics.md")]
pub mod d01_config_basics {}

/// 文档：`docs/guides/startup/first-endpoint.md`（接库与第一个接口）
#[doc = include_str!("../../docs/guides/startup/first-endpoint.md")]
pub mod d01_first_endpoint {}

/// 文档：`docs/guides/startup/middleware.md`（中间件装配）
#[doc = include_str!("../../docs/guides/startup/middleware.md")]
pub mod d01_middleware {}

/// 文档：`docs/guides/startup/hooks-plugins.md`（启动钩子与插件）
#[doc = include_str!("../../docs/guides/startup/hooks-plugins.md")]
pub mod d01_hooks_plugins {}

/// 文档：`docs/guides/startup/custom-config.md`（自定义配置与 FAQ）
#[doc = include_str!("../../docs/guides/startup/custom-config.md")]
pub mod d01_custom_config {}

/// 文档：`docs/guides/startup/config-reference.md`（配置全字段参考）
#[doc = include_str!("../../docs/guides/startup/config-reference.md")]
pub mod d01_config_reference {}

/// 文档：`docs/guides/startup/security-model.md`（认证鉴权与多租户安全模型）
#[doc = include_str!("../../docs/guides/startup/security-model.md")]
pub mod d01_security_model {}

/// 文档：`docs/guides/database.md`（数据库指南总目录）
#[doc = include_str!("../../docs/guides/database.md")]
pub mod d02 {}

// 以下是 `docs/guides/db/` 下按主题拆分的数据库分册，
// 承载原 `d02` 的真实代码块，必须继续纳入 rustdoc 编译以防示例漂移。
/// 文档：`docs/guides/db/getting-started.md`（配库 + 跑通第一个接口）
#[doc = include_str!("../../docs/guides/db/getting-started.md")]
pub mod d02_getting_started {}

/// 文档：`docs/guides/db/modeling.md`（定义模型）
#[doc = include_str!("../../docs/guides/db/modeling.md")]
pub mod d02_modeling {}

/// 文档：`docs/guides/db/crud.md`（六件最常用的事）
#[doc = include_str!("../../docs/guides/db/crud.md")]
pub mod d02_crud {}

/// 文档：`docs/guides/db/recipes.md`（查询与数据操作技巧）
#[doc = include_str!("../../docs/guides/db/recipes.md")]
pub mod d02_recipes {}

/// 文档：`docs/guides/db/ops.md`（视图/Hook/迁移/多库/错误映射/踩坑）
#[doc = include_str!("../../docs/guides/db/ops.md")]
pub mod d02_ops {}

/// 文档：`docs/guides/db/multi-tenant.md`（多租户隔离）
#[doc = include_str!("../../docs/guides/db/multi-tenant.md")]
pub mod d02_multi_tenant {}

/// 文档：`docs/guides/db/appendix.md`（API 速查/反模式/已删除 API）
#[doc = include_str!("../../docs/guides/db/appendix.md")]
pub mod d02_appendix {}

/// 文档：`docs/architecture/overview.md`
#[doc = include_str!("../../docs/architecture/overview.md")]
pub mod d03 {}

/// 文档：`docs/architecture/aifei-tribute.md`（致敬 Aifei：设计理念的 Rust 表达）
#[doc = include_str!("../../docs/architecture/aifei-tribute.md")]
pub mod d03_aifei_tribute {}

/// 文档：`docs/reference/crates.md`（Crate 参考总索引）
#[doc = include_str!("../../docs/reference/crates.md")]
pub mod d04 {}

// 以下是 `docs/reference/crates/` 下按 crate 拆分的 API 模块分册，
// 承载原 `d04` 的真实代码块，必须继续纳入 rustdoc 编译以防示例漂移。
/// 文档：`docs/reference/crates/core.md`（`alun-core`）
#[doc = include_str!("../../docs/reference/crates/core.md")]
pub mod d04_core {}

/// 文档：`docs/reference/crates/facade.md`（`alun` 门面）
#[doc = include_str!("../../docs/reference/crates/facade.md")]
pub mod d04_facade {}

/// 文档：`docs/reference/crates/web.md`（`alun-web`）
#[doc = include_str!("../../docs/reference/crates/web.md")]
pub mod d04_web {}

/// 文档：`docs/reference/crates/db.md`（`alun-db`）
#[doc = include_str!("../../docs/reference/crates/db.md")]
pub mod d04_db {}

/// 文档：`docs/reference/crates/config.md`（`alun-config`）
#[doc = include_str!("../../docs/reference/crates/config.md")]
pub mod d04_config {}

/// 文档：`docs/reference/crates/cache.md`（`alun-cache`）
#[doc = include_str!("../../docs/reference/crates/cache.md")]
pub mod d04_cache {}

/// 文档：`docs/reference/crates/template.md`（`alun-template`）
#[doc = include_str!("../../docs/reference/crates/template.md")]
pub mod d04_template {}

/// 文档：`docs/reference/crates/utils.md`（`alun-utils`）
#[doc = include_str!("../../docs/reference/crates/utils.md")]
pub mod d04_utils {}

/// 文档：`docs/reference/crates/macros.md`（`alun-macros`）
#[doc = include_str!("../../docs/reference/crates/macros.md")]
pub mod d04_macros {}

/// 文档：`docs/reference/crates/plugin.md`（`alun-plugin`）
#[doc = include_str!("../../docs/reference/crates/plugin.md")]
pub mod d04_plugin {}

/// 文档：`docs/reference/crates/log.md`（`alun-log`）
#[doc = include_str!("../../docs/reference/crates/log.md")]
pub mod d04_log {}

/// 文档：`docs/reference/crates/kafka.md`（`alun-kafka`）
#[doc = include_str!("../../docs/reference/crates/kafka.md")]
pub mod d04_kafka {}

/// 文档：`docs/reference/crates/fs.md`（`alun-fs`）
#[doc = include_str!("../../docs/reference/crates/fs.md")]
pub mod d04_fs {}

/// 文档：`docs/reference/crates/task.md`（`alun-task`）
#[doc = include_str!("../../docs/reference/crates/task.md")]
pub mod d04_task {}

/// 文档：`docs/reference/crates/db-family.md`（db 家族其余 crate）
#[doc = include_str!("../../docs/reference/crates/db-family.md")]
pub mod d04_db_family {}

/// 文档：`docs/reference/crates/observability.md`（可观测/实时/导出/扩展点）
#[doc = include_str!("../../docs/reference/crates/observability.md")]
pub mod d04_observability {}

/// 文档：`docs/reference/dependencies.md`
#[doc = include_str!("../../docs/reference/dependencies.md")]
pub mod d05 {}

/// 文档：`docs/guides/fullstack.md`
#[doc = include_str!("../../docs/guides/fullstack.md")]
pub mod d06 {}

/// 文档：`docs/index.md`（Code Wiki 索引）
#[doc = include_str!("../../docs/index.md")]
pub mod docs_index {}

/// 文档：`docs/guides/recommended-architecture.md`
#[doc = include_str!("../../docs/guides/recommended-architecture.md")]
pub mod recommended_layout {}

/// 文档：`skill-alun/SKILL.md`
#[doc = include_str!("../../skill-alun/SKILL.md")]
pub mod d08 {}

/// 文档：`skill-alun/references/auth.md`
#[doc = include_str!("../../skill-alun/references/auth.md")]
pub mod d09 {}

/// 文档：`skill-alun/references/config.md`
#[doc = include_str!("../../skill-alun/references/config.md")]
pub mod d10 {}

/// 文档：`skill-alun/references/database.md`
#[doc = include_str!("../../skill-alun/references/database.md")]
pub mod d11 {}

/// 文档：`skill-alun/references/plugins.md`
#[doc = include_str!("../../skill-alun/references/plugins.md")]
pub mod d12 {}

/// 文档：`skill-alun/references/routing.md`
#[doc = include_str!("../../skill-alun/references/routing.md")]
pub mod d13 {}

/// 文档：`skill-alun/references/task.md`
#[doc = include_str!("../../skill-alun/references/task.md")]
pub mod d14 {}

/// 文档：`skill-alun/references/utils.md`
#[doc = include_str!("../../skill-alun/references/utils.md")]
pub mod d15 {}

/// 文档：`skill-alun/references/advanced-db.md`
/// 联查 / 排序白名单 / 原生 SQL 租户注记 / 命名数据源
#[doc = include_str!("../../skill-alun/references/advanced-db.md")]
pub mod d16 {}

/// 文档：`skill-alun/references/extension-points.md`
/// on_startup / 中间件钩子 / layer / WebPlugin / 两条路由通道 / 行级异步 Hook
#[doc = include_str!("../../skill-alun/references/extension-points.md")]
pub mod d17 {}

/// 文档：`skill-alun/references/realtime-sse.md`
#[doc = include_str!("../../skill-alun/references/realtime-sse.md")]
pub mod d18 {}

/// 文档：`skill-alun/references/observability.md`
#[doc = include_str!("../../skill-alun/references/observability.md")]
pub mod d19 {}

/// 文档：`skill-alun/references/export-and-import.md`
#[doc = include_str!("../../skill-alun/references/export-and-import.md")]
pub mod d20 {}

/// 文档：`alun/README.md`
#[doc = include_str!("../../alun/README.md")]
pub mod d21 {}

/// 文档：`alun-cache/README.md`
#[doc = include_str!("../../alun-cache/README.md")]
pub mod d22 {}

/// 文档：`alun-config/README.md`
#[doc = include_str!("../../alun-config/README.md")]
pub mod d23 {}

/// 文档：`alun-core/README.md`
#[doc = include_str!("../../alun-core/README.md")]
pub mod d24 {}

/// 文档：`alun-db/README.md`
#[doc = include_str!("../../alun-db/README.md")]
pub mod d25 {}

/// 文档：`alun-fs/README.md`
#[doc = include_str!("../../alun-fs/README.md")]
pub mod d26 {}

/// 文档：`alun-kafka/README.md`
#[doc = include_str!("../../alun-kafka/README.md")]
pub mod d27 {}

/// 文档：`alun-log/README.md`
#[doc = include_str!("../../alun-log/README.md")]
pub mod d28 {}

/// 文档：`alun-macros/README.md`
#[doc = include_str!("../../alun-macros/README.md")]
pub mod d29 {}

/// 文档：`alun-plugin/README.md`
#[doc = include_str!("../../alun-plugin/README.md")]
pub mod d30 {}

/// 文档：`alun-task/README.md`
#[doc = include_str!("../../alun-task/README.md")]
pub mod d31 {}

/// 文档：`alun-template/README.md`
#[doc = include_str!("../../alun-template/README.md")]
pub mod d32 {}

/// 文档：`alun-utils/README.md`
#[doc = include_str!("../../alun-utils/README.md")]
pub mod d33 {}

/// 文档：`alun-web/README.md`
#[doc = include_str!("../../alun-web/README.md")]
pub mod d34 {}

/// 文档：`examples/04-test-suite/TEST_REPORT.md`
#[doc = include_str!("../../examples/04-test-suite/TEST_REPORT.md")]
pub mod examples_report {}

/// 「单一出口」契约：写**真编译的活代码**做校验。
///
/// 文档块只能证明「写下来能过 rustdoc」，这里直接进 lib 编译，
/// `cargo check -p alun-docs-tests --lib` 就能守住：handler 只需要
/// `ResResult<T>` —— 数据库、框架、IO 的错误全部 `?` 直通，
/// 业务代码里不需要出现 `DbError` / `WebError` / `alun::Error`。
#[allow(dead_code)]
mod single_exit {
    use alun::prelude::*;

    #[derive(Model, Debug, Clone, serde::Serialize)]
    #[table("users")]
    pub struct User {
        pub id: i64,
        pub name: String,
    }

    /// 数据访问层：错误类型是 `DbError`，签名用 `DbResult`。
    async fn load(id: i64) -> DbResult<User> {
        User::find(db(), id).await
    }

    /// handler 的唯一出口：`ResResult<T>`。`DbError` 直接 `?` 上来。
    #[alun::get("/users/{id}")]
    async fn detail(Path(id): Path<i64>) -> ResResult<User> {
        Ok(Res::ok(load(id).await?))
    }

    /// 业务主动报错：类型精确匹配，连 `.into()` 都不用写。
    async fn guarded(id: i64) -> ResResult<User> {
        if id <= 0 {
            return Err(ApiError::bad_request("id 必须为正数"));
        }
        let user = load(id).await.map_err(|e| match e {
            DbError::NotFound => ApiError::not_found("用户不存在"),
            DbError::UniqueViolation { .. } => ApiError::conflict("邮箱已被占用"),
            other => ApiError::internal(other.to_string()),
        })?;
        Ok(Res::ok(user))
    }

    /// 框架错误（配置 / 插件）与 IO 错误同样直通，统一脱敏成 500。
    async fn with_io() -> ResResult<Vec<User>> {
        let _ = std::fs::read_to_string("config.toml")?; // io::Error → ApiError(500 脱敏)
        Ok(Res::ok(User::query().limit(20).find(db()).await?))
    }

    /// 只在编译期出现：确保上面几个函数**真的被要求编译通过**。
    pub async fn assert_compiles() {
        let _ = (detail(Path(1)).await, guarded(1).await, with_io().await);
    }
}

//! 迁移系统。
//!
//! 以**版本号升序**驱动 schema 演进：支持单文件多语句（内置方言感知拆分器）、
//! 多种真实项目命名风格、单文件 `-- +migrate Up/Down` 双段式，以及
//! up/down 配对回滚。记录写入 `schema_migrations`（`(kind, name)` 主键），
//! 内容指纹与应用序号保证「已应用迁移不可被静默改写」与「回滚严格逆序」。
//!
//! ## 迁移种类
//!
//! | 种类 | 写法 | 语义 |
//! |---|---|---|
//! | 版本化 | `V1__x.sql` / `01_x.sql` | 按版本号执行一次，内容被改写即报错 |
//! | 可重复 | `R__view.sql` | 内容指纹变化即重跑（视图 / 函数） |
//! | 种子 | `S__dict.sql`、或 `source_seed()` 目录 | 同可重复，幂等重灌初始化数据 |
//! | 基线 | `B3__baseline.sql` | 声明 `<= 3` 视为已应用，本身不执行 SQL |
//!
//! `R` / `S` / `B` 三个前缀可用 [`MigratorOptions::repeatable_prefix`] /
//! [`MigratorOptions::seed_prefix`] / [`MigratorOptions::baseline_prefix`] 改写
//! （如 `Fn__` / `Dict__` / `Base{n}__`），设为空串即**关闭**该种类。三个前缀
//! 必须互不相同且不以数字开头，否则加载前显式报错。
//!
//! ## 并发保护
//!
//! 所有入口全程持有迁移互斥锁（`lock` 子模块）：PostgreSQL `pg_try_advisory_lock`、
//! MySQL `GET_LOCK`（会话级，跨进程有效）；SQLite 退化为**进程内** `Mutex`，
//! 跨进程并发迁移须由部署编排保证。
//!
//! ## 执行顺序
//!
//! **基线 → 版本化（版本升序）→ 可重复 → 种子（按名称升序）**。基线必须最先，
//! 否则它要标记为已应用的版本早已被真正执行过。
//!
//! 版本化迁移按 [`Version`] 数值分量升序应用；已应用（`success=true`）的迁移被
//! 跳过，上次失败的记录会被重跑。判定用**集合语义**而非 `MAX(version)`，
//! 因此 gap 补迁（已应用 `{1,5}` 时补 `{2,3,4}`）语义与 sqlx 一致：补迁默认
//! 允许，但 [`MigratorOptions::out_of_order`] 为 `false` 时会打 `WARN`。
//!
//! ## 内容指纹
//!
//! `checksum` 保存 `up` 脚本的 SHA-256，每次运行与库内记录比对。版本化迁移不一致
//! 即 [`DbError::MigrationChecksum`]（策略见 [`Divergent`]）；可重复 / 种子迁移的
//! 不一致是**预期行为**，触发重跑。旧框架导入与基线标记的历史行没有指纹，
//! 首次运行时补录（历史上无法回溯校验）。
//!
//! ## 运维入口
//!
//! - [`Migrator::status`] / [`Migrator::plan`]：`Applied` / `Pending` / `Divergent`
//!   / `Missing` / `Deleted` 全量视图与待应用清单；
//! - [`Migrator::baseline`]：存量库接入，跳过历史版本；
//! - [`Migrator::repair`]：修指纹、清理已删除的可重复 / 种子记录与失败行；
//! - [`Migrator::dry_run`]：整批共用单事务预演后回滚，不落库；
//! - [`Migrator::reset`] / [`Migrator::fresh`]：开发期重建库（须显式开启破坏性开关）。
//!
//! ## 编译期嵌入与占位符
//!
//! - `migrate!` 宏把迁移目录在编译期用 `include_str!` 编入二进制，运行时不再依赖
//!   磁盘上的 SQL 文件；它返回的仍是普通 [`Migrator`]，可与目录 / 种子来源链式叠加。
//! - [`MigratorOptions::placeholders`] 在分段与拆分**之前**替换脚本中的 `${key}`，
//!   因此内容指纹覆盖替换后的 SQL：改动取值会被指纹校验拦下，而非静默执行新 SQL。
//!
//! ## 模块划分
//!
//! - `loader`：目录 / 单文件加载与多风格文件名解析
//! - `splitter`：方言感知语句拆分器（公共工具，可独立复用）
//! - `exec`：事务 / 非事务两档执行器
//! - `history`：记录表建表 / 读取 / 写入与内容指纹
//! - `compat`：旧框架 `_sqlx_migrations` 一次性导入
//! - `kind`：迁移种类 / 状态 / 方言类别

mod compat;
mod exec;
mod history;
mod kind;
mod loader;
mod lock;
mod splitter;
mod version;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use alun_db_core::error::{DbError, Result};

use crate::db::Db;

pub use kind::{DialectKind, MigrationKind, MigrationState, MigrationStatus};
pub use splitter::{Directives, Sections, parse_directives, split_sections, split_statements};
pub use version::Version;

use kind::AppliedVia;
use loader::LoadedMigration;

// ==================== 公开类型 ====================

/// `dry_run` 的预演结果。
#[derive(Debug, Clone, Default)]
pub struct DryRunReport {
    /// 已成功预演（执行后回滚）的迁移数量。
    pub previewed: usize,
    /// 因声明 `-- +no-transaction` 而**跳过**预演的迁移名称。
    ///
    /// 这类迁移的语句会真实提交，无法回滚，故不参与预演。
    pub skipped_non_transactional: Vec<String>,
}

/// `repair` 的处理结果。
#[derive(Debug, Clone, Default)]
pub struct RepairReport {
    /// 接受当前内容、重写指纹的已应用迁移数量（`Divergent` 的人工裁决结果）。
    pub repaired_checksums: usize,
    /// 补录指纹的历史记录数量（旧框架导入行原本没有指纹）。
    pub backfilled_checksums: usize,
    /// 清理的「文件已删除」可重复 / 种子记录数量。
    pub deleted_records: usize,
    /// 清理的 `success = false` 失败记录数量。
    pub removed_failed_records: u64,
}

/// `reset` 的处理结果。
#[derive(Debug, Clone, Default)]
pub struct ResetReport {
    /// 执行了 `down` 脚本并回滚的迁移数量。
    pub rolled_back: usize,
    /// 只删记录、**未执行** `down` 的条目数量。
    ///
    /// 对应基线标记：框架从未执行过这些迁移（只是声明「视为已应用」），无从判断
    /// 库内对象是谁建的，因此绝不执行它们的 `down`。
    pub cleared_records: usize,
}

/// `fresh` 的处理结果。
#[derive(Debug, Clone, Default)]
pub struct FreshReport {
    /// 重置阶段的结果。
    pub reset: ResetReport,
    /// 重建阶段实际应用的迁移数量。
    pub applied: usize,
}

/// 一条迁移：版本号 + 名称 + 正向 / 回退 SQL。
///
/// 内容为**拥有型**（[`Version`] + `String`），因此既能承载磁盘加载的迁移，
/// 也能承载运行时构造的清单；编程式清单经 [`Migrator::from_migrations`] 注入。
///
/// ```no_run
/// # use alun_db::prelude::*;
/// # async fn demo(db: &Db) -> Result<()> {
/// let migrator = Migrator::from_migrations([Migration::new(
///     1,
///     "create_users",
///     "CREATE TABLE users (id INTEGER PRIMARY KEY)",
///     "DROP TABLE users",
/// )]);
/// migrator.run(db).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    /// 版本号（[`Version`] 支持 `1` / `1.2` / `20250101_120000` 等通用形态）。
    pub version: Version,
    /// 迁移名称（记录表自然键的一部分）。
    pub name: String,
    /// 正向 SQL。
    pub up: String,
    /// 回退 SQL；单向迁移为空串。
    pub down: String,
}

impl Migration {
    /// 构造一条迁移。
    ///
    /// # 参数
    ///
    /// - `version`：版本号，接受 `i64` / `&str` / [`Version`]。
    /// - `name`：迁移名称。
    /// - `up`：正向 SQL（可多语句，执行时按方言拆分）。
    /// - `down`：回退 SQL；单向迁移传空串。
    pub fn new(
        version: impl Into<Version>,
        name: impl Into<String>,
        up: impl Into<String>,
        down: impl Into<String>,
    ) -> Self {
        Migration {
            version: version.into(),
            name: name.into(),
            up: up.into(),
            down: down.into(),
        }
    }
}

/// 已应用迁移被改写（指纹不一致）时的处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Divergent {
    /// 报 [`DbError::MigrationChecksum`] 中止（默认，保留强校验语义）。
    #[default]
    Abort,
    /// 只告警，继续执行（运维逃生门）。
    Warn,
    /// 跳过校验（危险，仅在明确知情时使用）。
    Ignore,
}

impl Divergent {
    /// 策略在配置中的名称。
    pub fn as_str(self) -> &'static str {
        match self {
            Divergent::Abort => "abort",
            Divergent::Warn => "warn",
            Divergent::Ignore => "ignore",
        }
    }

    /// 由配置字符串解析（大小写不敏感）。
    ///
    /// # 错误
    ///
    /// 取值不在 `abort` / `warn` / `ignore` 之内时返回 [`DbError::InvalidValue`]，
    /// 而不是回退到默认值——拼错策略名绝不能静默降级成「中止」或「忽略」。
    pub fn from_policy(policy: &str) -> Result<Self> {
        match policy.trim().to_ascii_lowercase().as_str() {
            "abort" => Ok(Divergent::Abort),
            "warn" => Ok(Divergent::Warn),
            "ignore" => Ok(Divergent::Ignore),
            other => Err(DbError::InvalidValue(format!(
                "unknown divergent policy '{other}'; expected one of `abort`, `warn`, `ignore`"
            ))),
        }
    }
}

/// 迁移执行选项。
#[derive(Debug, Clone)]
pub struct MigratorOptions {
    /// 是否拆分单文件多语句（默认开；jinhu / facai 的单文件多语句必需）。
    pub split_statements: bool,
    /// 是否解析单文件 `-- +migrate Up/Down` 分段（默认开）。
    pub up_down_sections: bool,
    /// 版本前缀；空串表示任意（接受 `V` 前缀或无前缀）。
    pub version_prefix: String,
    /// 名称分隔符；空串表示自动（优先 `__`，回退 `_`）。
    pub name_separator: String,
    /// 可重复迁移前缀（默认 `R`，即 `R__view.sql`）；空串表示**关闭**该种类。
    pub repeatable_prefix: String,
    /// 种子迁移前缀（默认 `S`，即 `S__dict.sql`）；空串表示**关闭**该种类。
    pub seed_prefix: String,
    /// 基线标记前缀（默认 `B`，即 `B3__baseline.sql`）；空串表示**关闭**该种类。
    pub baseline_prefix: String,
    /// 迁移默认是否包裹事务（`-- +no-transaction` 可就地覆盖）。
    pub transaction_default: bool,
    /// 是否校验已应用迁移的内容指纹。
    pub validate_checksum: bool,
    /// 指纹不一致时的处理策略。
    pub divergent: Divergent,
    /// 是否允许乱序补迁（「版本号低于已应用最大值」的待应用迁移）。
    ///
    /// `false`（默认）：补迁可用，但会打 `WARN`——迟到的低版本在生产环境
    /// 通常意味着分支合并事故，需要人工确认；`true`：调用方已声明知情，
    /// 补迁不再告警。两种取值都不拒绝补迁（与 sqlx 的 gap 语义一致）。
    pub out_of_order: bool,
    /// 是否导入旧框架 `_sqlx_migrations` 的已应用记录（存量库接入）。
    ///
    /// 仅在记录表为空且旧表存在时生效一次；`false` 时完全不探测旧表。
    pub import_legacy: bool,
    /// 是否在迁移前自动执行基线标记（存量裸库接入）。
    ///
    /// 开启后，低于 [`MigratorOptions::baseline_version`] 的迁移会被直接标记为
    /// 已应用而不执行 SQL，用于「库结构已经存在、但没有任何迁移记录」的场景。
    pub baseline_on_migrate: bool,
    /// 基线版本：`<=` 该版本的迁移被视为已应用（仅 `baseline_on_migrate` 时生效）。
    pub baseline_version: Option<Version>,
    /// 迁移事务的 `synchronous_commit` 设置（PostgreSQL，`SET LOCAL`）。
    ///
    /// `None`（默认）不设置；`Some("off")` 时每个迁移事务在脚本执行前执行
    /// `SET LOCAL synchronous_commit = off`——提交不等待 WAL 落盘，首次全量迁移
    /// （数十个独立事务）的提交等待显著降低（本机实测最大单项）。取值仅允许
    /// ASCII 字母、数字与 `_`（防注入），非法值在执行时报错；仅事务模式生效。
    pub synchronous_commit: Option<String>,
    /// 是否把单次 run 中的**全部待应用迁移合并进一个事务**（默认 `false`）。
    ///
    /// 开启后仅当待应用集合全部为事务型迁移且超过 1 条时生效（含
    /// `-- +no-transaction` 的批次自动回退逐条模式）：省去每迁移一次
    /// BEGIN / COMMIT 往返。代价是失败时**整批回滚**——已应用的迁移不再保留，
    /// 重跑从零开始（断点续跑语义仅在关闭时保证）。适合「空库首次初始化」
    /// 这类可整体重试的场景，不适合增量补迁。
    pub single_transaction: bool,
    /// 脚本占位符表：替换迁移 SQL 中的 `${key}`。
    ///
    /// 空表（默认）表示**完全不扫描**，含 `${` 的字符串字面量因此不会被动到。
    /// 替换发生在分段 / 拆分之前，故内容指纹覆盖替换后的 SQL——占位符取值改变
    /// 会被 [`DbError::MigrationChecksum`] 检出，而不是静默执行不同的 SQL。
    pub placeholders: HashMap<String, String>,
    /// 是否允许**破坏性**操作（[`Migrator::reset`] / [`Migrator::fresh`]）。
    ///
    /// 默认 `false`：这两个入口会显式报 [`DbError::InvalidValue`]。框架的
    /// `auto_migrate` 启动路径**从不**打开此开关，避免配置失误导致生产库被重建。
    pub is_destructive_allowed: bool,
}

impl Default for MigratorOptions {
    fn default() -> Self {
        MigratorOptions {
            split_statements: true,
            up_down_sections: true,
            version_prefix: String::new(),
            name_separator: String::new(),
            repeatable_prefix: "R".to_string(),
            seed_prefix: "S".to_string(),
            baseline_prefix: "B".to_string(),
            transaction_default: true,
            validate_checksum: true,
            divergent: Divergent::Abort,
            out_of_order: false,
            import_legacy: false,
            baseline_on_migrate: false,
            baseline_version: None,
            synchronous_commit: None,
            single_transaction: false,
            placeholders: HashMap::new(),
            is_destructive_allowed: false,
        }
    }
}

/// 迁移执行器：来源（多目录 / 内存清单）+ 策略（[`MigratorOptions`]）。
///
/// ```no_run
/// # use alun_db::prelude::*;
/// # async fn demo(db: &Db) -> Result<()> {
/// let migrator = Migrator::new()
///     .source_dir("migrations/system")
///     .source_dir("migrations/support");
/// migrator.run(db).await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Default)]
pub struct Migrator {
    /// 版本化 / 基线迁移的来源目录（递归扫描，版本号跨目录全局唯一）。
    sources: Vec<PathBuf>,
    /// 种子数据目录（其中每个 `.sql` 按 `S__` 语义装载为种子迁移）。
    seed_sources: Vec<PathBuf>,
    /// 编译期嵌入的迁移 `(文件名, 内容)`；由 `migrate!` 宏生成。
    embedded: &'static [(&'static str, &'static str)],
    /// 内存迁移清单（编程式）。
    migrations: Vec<Migration>,
    /// 执行选项。
    options: MigratorOptions,
}

impl Migrator {
    /// 创建空执行器（无来源、默认选项）。
    pub fn new() -> Self {
        Migrator::default()
    }

    /// 追加一个来源目录（可多次调用，版本号须跨目录全局唯一）。
    pub fn source_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.sources.push(dir.into());
        self
    }

    /// 追加一个种子数据目录。
    ///
    /// 目录内的每个 `.sql` 都按 [`MigrationKind::Seed`] 装载，文件名即种子名
    /// （不要求 `S__` 前缀）。种子迁移在版本化迁移之后执行，内容指纹变化时重跑。
    pub fn source_seed(mut self, dir: impl Into<PathBuf>) -> Self {
        self.seed_sources.push(dir.into());
        self
    }

    /// 设置编译期嵌入的迁移来源（`migrate!` 宏产物）。
    ///
    /// 与目录来源**并存**（加载期合并两者，版本号冲突会显式报错）；嵌入文件与
    /// 磁盘文件共用同一套解析、分类与指纹逻辑，行为完全一致。
    pub fn source_embedded(mut self, pairs: &'static [(&'static str, &'static str)]) -> Self {
        self.embedded = pairs;
        self
    }

    /// 设置执行选项。
    pub fn options(mut self, options: MigratorOptions) -> Self {
        self.options = options;
        self
    }

    /// 由单个目录构造（等价于 `new().source_dir(dir)`）。
    pub fn from_dir(dir: impl Into<PathBuf>) -> Self {
        Migrator::new().source_dir(dir)
    }

    /// 由内存迁移清单构造。
    pub fn from_migrations(list: impl IntoIterator<Item = Migration>) -> Self {
        Migrator {
            sources: Vec::new(),
            seed_sources: Vec::new(),
            embedded: &[],
            migrations: list.into_iter().collect(),
            options: MigratorOptions::default(),
        }
    }

    /// 由编译期嵌入的 `(文件名, 内容)` 对构造。
    ///
    /// 通常不直接调用，而是通过 `migrate!` 宏生成：
    ///
    /// ```ignore
    /// # use alun_db::prelude::*;
    /// # async fn demo(db: &Db) -> Result<()> {
    /// // 迁移文件在编译期被 include_str! 编入二进制，部署时不依赖 `migrations/` 目录
    /// let migrator = alun_db::migrate!("migrations");
    /// migrator.run(db).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_embedded(pairs: &'static [(&'static str, &'static str)]) -> Self {
        Migrator {
            embedded: pairs,
            ..Migrator::default()
        }
    }

    /// 查看当前执行选项。
    pub fn options_ref(&self) -> &MigratorOptions {
        &self.options
    }

    /// 应用所有尚未记录的迁移。
    ///
    /// # 返回
    ///
    /// 本次实际应用的迁移数量。
    ///
    /// # 错误
    ///
    /// 来源加载失败、版本 / 名称冲突、拿不到迁移互斥锁、指纹校验失败、
    /// 建表失败或任一迁移执行失败时返回 [`DbError`]。
    pub async fn run(&self, db: &Db) -> Result<usize> {
        let started = Instant::now();
        let list = self.load()?;
        let load_ms = started.elapsed().as_millis();

        // 稳态快路径：记录表已存在且没有待应用迁移时，只读一次历史 + 校验指纹即可
        // 返回。不获取咨询锁、不做 CREATE TABLE / 索引探测、不重复读取历史——
        // 启动期最常见的「应用 0 个迁移」不再付出 4~6 次额外往返。
        //
        // `import_legacy` / `baseline_on_migrate` 会写记录表，必须走完整路径；
        // 有 pending 时同样走完整路径（在那里持锁后重新读历史，避免并发实例竞态）。
        if !self.options.import_legacy && !self.options.baseline_on_migrate {
            let history_started = Instant::now();
            match history::load_applied(db).await {
                Ok(applied) => {
                    let history_ms = history_started.elapsed().as_millis();
                    if !list.iter().any(|m| needs_apply(m, &applied)) {
                        // 无待应用迁移：仍需校验已应用迁移的指纹（内容被改写必须中止）
                        let verify_started = Instant::now();
                        self.verify_checksums(db, &list, &applied).await?;
                        let verify_ms = verify_started.elapsed().as_millis();
                        tracing::debug!(
                            migrations = list.len(),
                            load_ms,
                            history_ms,
                            verify_ms,
                            elapsed_ms = started.elapsed().as_millis(),
                            "无待应用迁移，跳过迁移锁与建表探测"
                        );
                        return Ok(0);
                    }
                }
                // 记录表不存在（裸库）：交给完整路径建表并应用
                Err(e) if history::is_missing_table(&e) => {}
                Err(e) => return Err(e),
            }
        }

        self.apply_with_lock(db, list).await
    }

    /// 本次运行将应用哪些迁移（不落库）。
    ///
    /// 仅返回状态为 [`MigrationState::Pending`] 的迁移，按版本升序。
    pub async fn plan(&self, db: &Db) -> Result<Vec<MigrationStatus>> {
        Ok(self
            .status(db)
            .await?
            .into_iter()
            .filter(|status| status.state == MigrationState::Pending)
            .collect())
    }

    /// 全量状态视图：清单内每条迁移的状态 + 库内清单外记录（`Missing` / `Deleted`）。
    ///
    /// 库中不存在记录表时（裸库）按「全部待应用」返回，而不是报错——便于
    /// 存量库接入前先 `status` 观测。
    ///
    /// 状态语义（设计文档 §6）：
    /// - 版本化迁移内容被改写 → [`MigrationState::Divergent`]（执行入口会中止）；
    /// - 可重复 / 种子迁移内容变化 → [`MigrationState::Pending`]（会被重跑）；
    /// - 库内记录的版本化迁移不在清单中 → [`MigrationState::Missing`]；
    /// - 库内记录的可重复 / 种子迁移已无文件 → [`MigrationState::Deleted`]（`repair` 可清理）。
    pub async fn status(&self, db: &Db) -> Result<Vec<MigrationStatus>> {
        let list = self.load()?;
        check_identity_conflicts(&list)?;
        let applied = load_applied_tolerant(db).await?;

        let mut statuses: Vec<MigrationStatus> = Vec::with_capacity(list.len());
        for m in &list {
            let current = m.checksum().to_string();
            let record = applied.get(&(m.kind, m.name.clone()));
            let (is_applied, applied_checksum, state) = match record {
                // 上次失败（success=false）→ 本次会重跑
                Some(record) if record.is_success => {
                    let state = if record.checksum.as_deref() == Some(current.as_str()) {
                        MigrationState::Applied
                    } else if m.kind.is_rerunnable() {
                        // 可重复 / 种子的内容变化是预期行为，重跑即可
                        MigrationState::Pending
                    } else {
                        MigrationState::Divergent
                    };
                    (true, record.checksum.clone(), state)
                }
                _ => (
                    false,
                    record.and_then(|r| r.checksum.clone()),
                    MigrationState::Pending,
                ),
            };
            statuses.push(MigrationStatus {
                kind: m.kind,
                version: m.version.clone(),
                name: m.name.clone(),
                source: m.source.clone(),
                is_applied,
                applied_checksum,
                current_checksum: current,
                state,
            });
        }

        // 库内有记录、清单里没有的迁移 → 版本化记 Missing，可重复 / 种子记 Deleted
        let in_list: HashMap<(MigrationKind, String), ()> = list
            .iter()
            .map(|m| ((m.kind, m.name.clone()), ()))
            .collect();
        for ((kind, name), record) in &applied {
            if in_list.contains_key(&(*kind, name.clone())) {
                continue;
            }
            statuses.push(MigrationStatus {
                kind: *kind,
                version: record.version.clone(),
                name: name.clone(),
                source: None,
                is_applied: record.is_success,
                applied_checksum: record.checksum.clone(),
                current_checksum: String::new(),
                state: if kind.is_named() {
                    MigrationState::Deleted
                } else {
                    MigrationState::Missing
                },
            });
        }

        Ok(statuses)
    }

    /// 回滚最近一次**成功应用**的迁移（LIFO，按 `applied_seq`）。
    ///
    /// # 返回
    ///
    /// `Ok(Some(v))`：已回滚的版本号；`Ok(None)`：无可回滚的迁移。
    ///
    /// # 错误
    ///
    /// 最近应用的迁移不属于本执行器的来源（[`DbError::Migration`]）、
    /// 该迁移无 `down` 脚本、指纹校验失败或执行失败时返回 [`DbError`]。
    pub async fn rollback(&self, db: &Db) -> Result<Option<Version>> {
        let list = self.load()?;
        let rolled = lock::with_migration_lock(db, || self.rollback_inner(db, &list)).await?;
        Ok(rolled.and_then(|m| m.version))
    }

    // ==================== 存量库接入与运维 ====================

    /// 把 `<= version` 的版本化迁移标记为已应用（**不执行其 SQL**）。
    ///
    /// 用于存量生产库首次接入：库结构已存在但没有任何迁移记录时，用基线跳过
    /// 历史版本，随后 `run` 只应用基线之后的新迁移。标记只在**清单中存在**的
    /// 迁移上落地，不存在的版本不会被凭空写入记录。
    ///
    /// 已在库中的版本不会被覆盖（保留原指纹与 `applied_seq`），因此本操作可重复执行。
    ///
    /// # 参数
    ///
    /// - `db`：数据库句柄。
    /// - `baseline`：基线版本，`<=` 该版本的迁移视为已应用。
    ///
    /// # 返回
    ///
    /// 本次新标记的迁移数量。
    ///
    /// # 错误
    ///
    /// 基线版本为空、加载失败、建表失败或写记录失败时返回 [`DbError`]。
    ///
    /// ```no_run
    /// # use alun_db::prelude::*;
    /// # async fn demo(db: &Db) -> Result<()> {
    /// // 存量库结构已到 v3，跳过 v1..=v3
    /// Migrator::from_dir("migrations")
    ///     .baseline(db, Version::new("3"))
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn baseline(&self, db: &Db, baseline: Version) -> Result<usize> {
        if baseline.is_empty() {
            return Err(DbError::InvalidValue(
                "baseline requires a non-empty version".into(),
            ));
        }
        let list = self.load()?;
        lock::with_migration_lock(db, || self.baseline_inner(db, &list, &baseline)).await
    }

    /// 修复记录表：补/改指纹、清理已删除的可重复与种子记录、删除失败行。
    ///
    /// 三件事（设计文档 §6）：
    /// 1. **修指纹**：已应用版本化迁移的内容与库内指纹不符时，接受当前内容
    ///    （`Divergent` 的人工裁决结果），只改 `checksum` 不动 `applied_seq`；
    /// 2. **清理 Deleted**：库内有记录、清单里已无文件的可重复 / 种子迁移，删记录；
    /// 3. **清理失败行**：删除 `success = false` 的残留记录（非事务模式失败留下）。
    ///
    /// 版本化迁移的 `Missing`（清单外版本）**不清理**——它属于其他来源，
    /// 强行删除会掩盖真实的漏加载问题。
    ///
    /// # 返回
    ///
    /// [`RepairReport`]：三类操作各自处理的数量。
    ///
    /// # 错误
    ///
    /// 加载失败、建表失败或写记录失败时返回 [`DbError`]。
    pub async fn repair(&self, db: &Db) -> Result<RepairReport> {
        let list = self.load()?;
        lock::with_migration_lock(db, || self.repair_inner(db, &list)).await
    }

    /// 预演：执行本次会应用的迁移，随后**全部回滚**，不落库。
    ///
    /// 整批迁移在**同一个事务**内依次执行（后一条能看到前一条建出的对象），
    /// 结束时一次性回滚，因此：
    /// - 库内 `schema_migrations` 与业务表都不发生变化；
    /// - 语句本身的语法 / 约束 / 依赖错误会被真实暴露（这是预演的价值）；
    /// - 标了 `-- +no-transaction` 的迁移**无法预演**（语句会真实提交），
    ///   被跳过并记入 [`DryRunReport::skipped_non_transactional`]；若后续迁移
    ///   依赖它建出的对象，预演会报错——这是预期现象，不是误报。
    ///
    /// # 注意
    ///
    /// 回滚能力取决于方言：PostgreSQL / SQLite 的 DDL 可事务回滚；**MySQL 的
    /// DDL 隐式提交**，故在 MySQL 上预演只对 DML 有效，DDL 会真实生效。
    ///
    /// # 返回
    ///
    /// [`DryRunReport`]：预演统计。
    ///
    /// # 错误
    ///
    /// 加载失败、建表失败或某条迁移执行失败时返回 [`DbError`]
    /// （失败即中止并回滚，不留痕迹）。
    pub async fn dry_run(&self, db: &Db) -> Result<DryRunReport> {
        let list = self.load()?;
        lock::with_migration_lock(db, || self.dry_run_inner(db, &list)).await
    }

    /// 回滚**全部**已应用迁移（开发期重建库用）。
    ///
    /// 按 `applied_seq` **逆序**逐条回滚，直到记录表清空。若某条迁移的回滚脚本
    /// 会破坏仍被依赖的对象，回滚本身即报错并中止（与单条 `rollback` 同语义）。
    ///
    /// # 安全门
    ///
    /// 必须显式设置 [`MigratorOptions::is_destructive_allowed`]，否则直接报
    /// [`DbError::InvalidValue`]。框架的 `auto_migrate` 启动路径从不打开该开关。
    ///
    /// # 未执行过的记录的处理
    ///
    /// 基线标记（从未执行任何 SQL，只是声明「视为已应用」）**只删记录、不执行
    /// `down`**：框架从未执行过这些迁移，无从判断库内对象由谁创建，贸然回滚会拆掉
    /// 本不该动的表。这类条目计入 [`ResetReport::cleared_records`]。
    ///
    /// 旧框架导入的记录（`_sqlx_migrations`）**会**执行 `down`——旧框架确实跑过
    /// 它们的 `up`，回滚语义与框架自己执行过的迁移一致。
    ///
    /// # 返回
    ///
    /// [`ResetReport`]：回滚条数与仅清记录的条数。
    ///
    /// # 错误
    ///
    /// 未开启安全门、加载失败、某条迁移无 `down` 脚本、指纹校验失败或执行失败时
    /// 返回 [`DbError`]。
    pub async fn reset(&self, db: &Db) -> Result<ResetReport> {
        self.require_destructive("reset")?;
        let list = self.load()?;
        lock::with_migration_lock(db, || self.reset_inner(db, &list)).await
    }

    /// 重建开发库：先 [`reset`](Migrator::reset)，再 `run` 应用全部迁移。
    ///
    /// # 安全门
    ///
    /// 同 [`reset`](Migrator::reset)，需要 [`MigratorOptions::is_destructive_allowed`]。
    ///
    /// # 注意
    ///
    /// `reset` 只删记录、不回滚的条目（见 [`ResetReport::cleared_records`]）所建的
    /// 对象**不会被删除**，因此 `fresh` 可能因「表已存在」而失败。这属于预期：
    /// 框架拒绝猜测陌生对象的归属。需要绝对干净的重建请在**全新的数据库**上运行。
    ///
    /// # 返回
    ///
    /// [`FreshReport`]：重置结果 + 本次应用的迁移数量。
    ///
    /// # 错误
    ///
    /// 未开启安全门，或重置 / 应用阶段任一步失败时返回 [`DbError`]。
    pub async fn fresh(&self, db: &Db) -> Result<FreshReport> {
        self.require_destructive("fresh")?;
        let list = self.load()?;
        let (reset, applied) = lock::with_migration_lock(db, || async {
            let reset = self.reset_inner(db, &list).await?;
            let applied = self.apply(db, &list).await?;
            Ok((reset, applied))
        })
        .await?;
        Ok(FreshReport { reset, applied })
    }

    /// 破坏性操作的安全门。
    fn require_destructive(&self, operation: &str) -> Result<()> {
        if self.options.is_destructive_allowed {
            return Ok(());
        }
        Err(DbError::InvalidValue(format!(
            "`{operation}` is a destructive operation; set \
             `MigratorOptions::is_destructive_allowed = true` to confirm it is intended"
        )))
    }

    /// 应用单个迁移文件（版本号从文件名推断）。
    ///
    /// # 返回
    ///
    /// `Ok(1)`：已应用；`Ok(0)`：该版本此前已成功应用（幂等跳过）。
    pub async fn run_file(&self, db: &Db, path: &Path) -> Result<usize> {
        let migration = loader::load_single_file(path, &self.options)?;
        self.apply_with_lock(db, vec![migration]).await
    }

    /// 应用单个迁移文件，并显式指定版本号与名称。
    ///
    /// 用于文件名不符合命名规范、但确需纳入版本管理的补丁脚本：版本与名称
    /// 由调用方给定，文件名不再参与解析。
    ///
    /// # 错误
    ///
    /// 版本号或名称为空返回 [`DbError::InvalidValue`]；文件读取失败、
    /// 拿不到迁移互斥锁或执行失败时返回 [`DbError`]。
    pub async fn run_file_as(
        &self,
        db: &Db,
        path: &Path,
        version: Version,
        name: &str,
    ) -> Result<usize> {
        if version.is_empty() {
            return Err(DbError::InvalidValue(
                "run_file_as requires a non-empty version".into(),
            ));
        }
        if name.trim().is_empty() {
            return Err(DbError::InvalidValue(
                "run_file_as requires a non-empty name".into(),
            ));
        }
        let migration = loader::load_single_file_as(
            path,
            &self.options,
            MigrationKind::Versioned,
            Some(version),
            name.to_string(),
        )?;
        self.apply_with_lock(db, vec![migration]).await
    }

    // ==================== 内部实现 ====================

    /// 加载全部来源为统一的迁移清单（按执行顺序）。
    fn load(&self) -> Result<Vec<LoadedMigration>> {
        let mut list = loader::load_from_sources(&self.sources, &self.seed_sources, &self.options)?;
        // 编译期嵌入的迁移与目录迁移共用解析逻辑；两者版本号冲突会在此报错
        list.extend(loader::load_from_embedded(self.embedded, &self.options)?);
        for m in &self.migrations {
            list.push(LoadedMigration {
                kind: MigrationKind::Versioned,
                version: Some(m.version.clone()),
                name: m.name.clone(),
                up: m.up.clone(),
                down: m.down.clone(),
                source: None,
                is_no_transaction: false,
                checksum_cache: std::sync::OnceLock::new(),
            });
        }
        check_identity_conflicts(&list)?;
        list.sort_by_key(|a| a.sort_key());
        Ok(list)
    }

    /// 持锁应用。
    async fn apply_with_lock(&self, db: &Db, list: Vec<LoadedMigration>) -> Result<usize> {
        lock::with_migration_lock(db, || self.apply(db, &list)).await
    }

    /// 应用清单中所有未成功记录的迁移。
    async fn apply(&self, db: &Db, list: &[LoadedMigration]) -> Result<usize> {
        history::ensure_ready(db, self.options.import_legacy).await?;

        // 基线先行：它要「标记为已应用」的版本必须先于真正的执行发生
        self.apply_configured_baseline(db, list).await?;
        // 基线刚落地的记录也参与后续判定，故重新读取
        let applied = history::load_applied(db).await?;

        // 版本化迁移的指纹变化必须中止；可重复 / 种子的变化是重跑信号，跳过校验
        self.verify_checksums(db, list, &applied).await?;

        let max_applied = max_applied_version(&applied);
        let mut seq = history::next_seq(&applied);

        // 先选出本次要应用的清单（跳过已成功记录与基线标记），再决定执行模式：
        // 单事务模式（`single_transaction`，仅当全部为事务型迁移且 >1 条时生效）
        // 省去每迁移一次 BEGIN / COMMIT 往返；否则逐条应用（失败隔离、断点续跑）。
        let mut pending: Vec<(&LoadedMigration, i64, String)> = Vec::new();
        for migration in list {
            if !needs_apply(migration, &applied) {
                continue;
            }
            let checksum = migration.checksum().to_string();
            let key = (migration.kind, migration.name.clone());
            // 记录重跑原因，便于排查「为什么每次都执行」
            match applied.get(&key) {
                Some(record) if record.is_success => {
                    tracing::info!(
                        migration = %migration.name,
                        kind = %migration.kind,
                        "可重复迁移内容已变化，重新执行"
                    );
                }
                Some(_) => {
                    // 上次失败的记录：重跑（失败行不视为已应用）
                    tracing::info!(migration = %migration.name, "上次未成功的迁移将重跑");
                }
                None => {}
            }

            // 迟到的低版本：允许补迁（与 sqlx 一致），但默认要让人看见
            if let (Some(version), Some(max_applied)) = (&migration.version, &max_applied)
                && migration.kind == MigrationKind::Versioned
                && version < max_applied
                && !self.options.out_of_order
            {
                tracing::warn!(
                    migration = %migration.name,
                    version = %version,
                    latest_applied = %max_applied,
                    "待应用迁移的版本低于已应用最大值（乱序补迁）；\
                     确认无误后可设 out_of_order = true 消除告警"
                );
            }

            pending.push((migration, seq, checksum));
            seq += 1;
        }
        if pending.is_empty() {
            return Ok(0);
        }

        let is_batch = self.options.single_transaction
            && pending.len() > 1
            && pending
                .iter()
                .all(|(m, _, _)| exec::is_transactional(m, &self.options, &m.up));
        if is_batch {
            // 单事务整批：全部迁移共用一个事务；失败整批回滚（重跑从零开始）。
            tracing::info!(
                count = pending.len(),
                "单事务模式：本次 {} 个迁移合并为一个事务执行",
                pending.len()
            );
            let tx = db.begin().await?;
            for (migration, migration_seq, checksum) in &pending {
                if let Err(e) =
                    exec::apply_up_in_tx(&tx, migration, &self.options, *migration_seq, checksum)
                        .await
                {
                    // 显式回滚后再上抛，避免带未提交事务的连接归还连接池
                    if let Err(rollback_err) = tx.rollback().await {
                        tracing::warn!("单事务模式回滚失败: {rollback_err}");
                    }
                    return Err(e);
                }
                tracing::debug!(migration = %migration.name, "迁移已应用（单事务模式）");
            }
            tx.commit().await?;
        } else {
            for (migration, migration_seq, checksum) in &pending {
                let outcome =
                    exec::apply_up(db, migration, &self.options, *migration_seq, checksum).await?;
                tracing::debug!(
                    migration = %migration.name,
                    statements = outcome.statement_count,
                    elapsed_ms = outcome.execution_ms,
                    "迁移已应用"
                );
            }
        }
        Ok(pending.len())
    }

    /// 执行配置化的基线（`baseline_on_migrate` + `baseline_version`）。
    async fn apply_configured_baseline(&self, db: &Db, list: &[LoadedMigration]) -> Result<()> {
        if !self.options.baseline_on_migrate {
            return Ok(());
        }
        let Some(version) = self.options.baseline_version.as_ref() else {
            return Err(DbError::InvalidValue(
                "baseline_on_migrate requires baseline_version".into(),
            ));
        };
        let marked = self.baseline_inner(db, list, version).await?;
        if marked > 0 {
            tracing::info!(version = %version, marked, "基线标记完成");
        }
        Ok(())
    }

    /// 基线标记的内部实现（调用方已持锁）。
    async fn baseline_inner(
        &self,
        db: &Db,
        list: &[LoadedMigration],
        baseline: &Version,
    ) -> Result<usize> {
        history::ensure_ready(db, self.options.import_legacy).await?;
        let applied = history::load_applied(db).await?;

        // 已存在的记录不覆盖：基线是「首次接入」的手段，重复调用必须无副作用
        let mut seq = history::next_seq(&applied);
        let mut marked = 0_usize;
        for migration in list {
            if migration.kind != MigrationKind::Versioned {
                continue;
            }
            let Some(version) = migration.version.as_ref() else {
                continue;
            };
            if version > baseline {
                continue;
            }
            if applied.contains_key(&(migration.kind, migration.name.clone())) {
                continue;
            }
            let record = history::RecordWrite {
                kind: migration.kind,
                name: migration.name.clone(),
                version: Some(version.clone()),
                // 基线是「假定已应用」，没有任何被执行的 SQL → 指纹留空
                checksum: None,
                applied_via: AppliedVia::Baseline,
                seq,
                execution_ms: None,
                is_success: true,
                source: migration
                    .source
                    .as_ref()
                    .map(|path| path.display().to_string()),
            };
            let tx = db.begin().await?;
            if let Err(e) = history::upsert_in_tx(&tx, &record).await {
                // 显式回滚后再上抛，避免带未提交事务的连接归还连接池
                if let Err(rollback_err) = tx.rollback().await {
                    tracing::warn!("回滚基线标记事务失败: {rollback_err}");
                }
                return Err(e);
            }
            tx.commit().await?;
            seq += 1;
            marked += 1;
        }
        Ok(marked)
    }

    /// `repair` 的内部实现（调用方已持锁）。
    async fn repair_inner(&self, db: &Db, list: &[LoadedMigration]) -> Result<RepairReport> {
        history::ensure_ready(db, self.options.import_legacy).await?;
        let applied = history::load_applied(db).await?;

        let in_list: HashMap<(MigrationKind, String), &LoadedMigration> =
            list.iter().map(|m| ((m.kind, m.name.clone()), m)).collect();

        let mut report = RepairReport::default();
        for ((kind, name), record) in &applied {
            let Some(migration) = in_list.get(&(*kind, name.clone())) else {
                // 清单外的可重复 / 种子迁移 → 文件已删除，清理记录
                if kind.is_named() {
                    history::delete_record(db, *kind, name).await?;
                    report.deleted_records += 1;
                    tracing::info!(migration = %name, kind = %kind, "已删除的迁移记录被清理");
                }
                continue;
            };
            // 已应用版本化迁移的指纹不符 → 接受当前内容
            if *kind == MigrationKind::Versioned && record.is_success {
                let current = migration.checksum().to_string();
                if record.checksum.as_deref() != Some(current.as_str()) {
                    match record.checksum.as_deref() {
                        // 历史行缺指纹 → 补录，不算「改指纹」
                        None => report.backfilled_checksums += 1,
                        Some(_) => report.repaired_checksums += 1,
                    }
                    history::set_checksum(db, *kind, name, &current).await?;
                }
            }
        }

        report.removed_failed_records = history::delete_unsuccessful(db).await?;
        Ok(report)
    }

    /// `dry_run` 的内部实现（调用方已持锁）。
    ///
    /// 整批预演共用**一个**事务：后一条迁移能看到前一条建出的表，与真实执行的
    /// 可见性一致；结束时一次性回滚，因此 `schema_migrations` 与业务表都无变化。
    async fn dry_run_inner(&self, db: &Db, list: &[LoadedMigration]) -> Result<DryRunReport> {
        history::ensure_ready(db, self.options.import_legacy).await?;
        let applied = history::load_applied(db).await?;

        // 先挑出本次真正会执行的迁移，避免为「全部已应用」的库白开事务
        let mut pending: Vec<&LoadedMigration> = Vec::new();
        let mut report = DryRunReport::default();
        for migration in list {
            if migration.kind == MigrationKind::Baseline {
                continue;
            }
            let checksum = migration.checksum().to_string();
            let key = (migration.kind, migration.name.clone());
            let is_pending = match applied.get(&key) {
                Some(record) if record.is_success => {
                    migration.kind.is_rerunnable() && record.checksum.as_deref() != Some(&checksum)
                }
                _ => true,
            };
            if !is_pending {
                continue;
            }
            if exec::is_transactional(migration, &self.options, &migration.up) {
                pending.push(migration);
            } else {
                // 非事务迁移的语句会真实提交，无法回滚，故不参与预演
                report
                    .skipped_non_transactional
                    .push(migration.name.clone());
                tracing::warn!(
                    migration = %migration.name,
                    "该迁移声明了 no-transaction，语句会真实提交，dry_run 跳过"
                );
            }
        }
        if pending.is_empty() {
            return Ok(report);
        }

        let tx = db.begin().await?;
        for migration in pending {
            if let Err(e) = exec::execute_up_in_tx(&tx, migration, &self.options).await {
                // 显式回滚后再上抛，避免带未提交事务的连接归还连接池
                if let Err(rollback_err) = tx.rollback().await {
                    tracing::warn!("回滚预演事务失败: {rollback_err}");
                }
                return Err(e);
            }
            report.previewed += 1;
        }
        tx.rollback().await?;
        Ok(report)
    }

    /// `reset` 的内部实现（调用方已持锁）。
    ///
    /// 每次取「应用顺序上最近的一条记录」处理：
    /// - 有指纹（我们执行过）→ 走 `rollback_inner` 执行 `down` 并删记录；
    /// - 无指纹（基线标记 / 旧框架导入）→ 只删记录，**不执行 `down`**。
    ///
    /// 两条分支都推进「最近一条」，因此循环必然收敛。
    async fn reset_inner(&self, db: &Db, list: &[LoadedMigration]) -> Result<ResetReport> {
        history::ensure_ready(db, self.options.import_legacy).await?;

        let mut report = ResetReport::default();
        loop {
            let applied = history::load_applied(db).await?;
            let Some(record) = latest_success(&applied) else {
                return Ok(report);
            };
            if !record.applied_via.can_roll_back() {
                // 未由任何真实执行者跑过（基线标记）：清记录即可，绝不猜它建过什么对象。
                // 判定依据是 `applied_via` 而非「checksum 为空」——历史行会在首次运行时
                // 补录指纹，那之后两者就再也分不清了。
                tracing::info!(
                    migration = %record.name,
                    kind = %record.kind,
                    "该记录未执行过 SQL（基线标记），仅清除记录而不执行 down"
                );
                history::delete_record(db, record.kind, &record.name).await?;
                report.cleared_records += 1;
                continue;
            }
            // 执行过的迁移必须能在清单里找到（否则无法安全回滚，显式报错）
            self.rollback_inner(db, list).await?;
            report.rolled_back += 1;
        }
    }

    /// 回滚内部实现：定位最近成功应用的迁移并执行其 `down`。
    async fn rollback_inner(
        &self,
        db: &Db,
        list: &[LoadedMigration],
    ) -> Result<Option<LoadedMigration>> {
        history::ensure_ready(db, self.options.import_legacy).await?;
        let applied = history::load_applied(db).await?;
        let Some(record) = latest_success(&applied) else {
            return Ok(None);
        };

        let migration = list
            .iter()
            .find(|m| m.kind == record.kind && m.name == record.name)
            .ok_or_else(|| out_of_scope_error(record, list.len()))?;

        self.verify_one(migration, record, migration.checksum())?;
        let outcome = exec::apply_down(db, migration, &self.options).await?;
        tracing::debug!(
            migration = %migration.name,
            statements = outcome.statement_count,
            elapsed_ms = outcome.execution_ms,
            "迁移已回滚"
        );
        Ok(Some(migration.clone()))
    }

    /// 校验清单中已应用迁移的内容指纹，并补录历史行缺失的指纹。
    ///
    /// 只校验「本次清单中出现」的记录：清单外的版本属于其他来源，由那个来源的
    /// 运行时校验。可重复 / 种子迁移的内容变化是**预期行为**（重跑信号），
    /// 因此不参与校验，也不会因内容变化而中止。
    async fn verify_checksums(
        &self,
        db: &Db,
        list: &[LoadedMigration],
        applied: &history::AppliedMap,
    ) -> Result<()> {
        for migration in list {
            if migration.kind.is_rerunnable() {
                continue;
            }
            let Some(record) = applied.get(&(migration.kind, migration.name.clone())) else {
                continue;
            };
            // 基线标记从未执行过 SQL，给它补指纹等于伪造「这段内容被应用过」的证据
            if record.applied_via == AppliedVia::Baseline {
                continue;
            }
            let current = migration.checksum().to_string();
            if record.checksum.is_none() {
                tracing::info!(
                    migration = %migration.name,
                    "历史迁移记录缺少内容指纹，首次运行补录"
                );
                history::set_checksum(db, record.kind, &record.name, &current).await?;
                continue;
            }
            self.verify_one(migration, record, &current)?;
        }
        Ok(())
    }

    /// 校验单条迁移的内容指纹（按 [`Divergent`] 策略处理不一致）。
    fn verify_one(
        &self,
        migration: &LoadedMigration,
        record: &history::AppliedRecord,
        current: &str,
    ) -> Result<()> {
        if !self.options.validate_checksum {
            return Ok(());
        }
        let Some(recorded) = record.checksum.as_deref() else {
            return Ok(());
        };
        if recorded == current {
            return Ok(());
        }
        match self.options.divergent {
            Divergent::Abort => Err(DbError::MigrationChecksum {
                version: migration
                    .version
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                name: migration.name.clone(),
                applied: recorded.to_string(),
                current: current.to_string(),
            }),
            Divergent::Warn => {
                tracing::warn!(
                    migration = %migration.name,
                    applied = %recorded,
                    current = %current,
                    "已应用迁移内容指纹不一致（divergent_policy=warn，继续执行）"
                );
                Ok(())
            }
            Divergent::Ignore => Ok(()),
        }
    }
}

// ==================== 辅助函数 ====================

/// 取应用顺序上最近的一条**成功**记录。
fn latest_success(applied: &history::AppliedMap) -> Option<&history::AppliedRecord> {
    applied
        .values()
        .filter(|record| record.is_success)
        .max_by_key(|record| record.seq)
}

/// 该迁移在本次运行中是否需要执行。
///
/// 与 [`Migrator::apply`] 的待应用判定共用同一份规则（启动快路径与正式执行
/// 不允许出现两套口径）：
///
/// - 基线只写标记、不执行 SQL；
/// - 已成功记录默认跳过；可重复 / 种子迁移内容变化时重跑；
/// - 上次失败的记录重跑；记录表里没有的迁移是新迁移。
fn needs_apply(migration: &LoadedMigration, applied: &history::AppliedMap) -> bool {
    if migration.kind == MigrationKind::Baseline {
        return false;
    }
    match applied.get(&(migration.kind, migration.name.clone())) {
        Some(record) if record.is_success => {
            migration.kind.is_rerunnable()
                && record.checksum.as_deref() != Some(migration.checksum())
        }
        Some(_) => true,
        None => true,
    }
}

/// 已成功应用的版本化迁移中的最大版本号（乱序补迁判定用）。
///
/// 只看 [`MigrationKind::Versioned`]：基线标记的版本不参与比较，否则一次
/// 高版本基线就会把所有正常的顺序迁移判成「乱序」。
fn max_applied_version(applied: &history::AppliedMap) -> Option<Version> {
    applied
        .values()
        .filter(|record| record.is_success && record.kind == MigrationKind::Versioned)
        .filter_map(|record| record.version.clone())
        .max()
}

/// 读取已应用记录；记录表不存在（裸库）时视为空表。
async fn load_applied_tolerant(db: &Db) -> Result<history::AppliedMap> {
    match history::load_applied(db).await {
        Ok(applied) => Ok(applied),
        Err(e) if history::is_missing_table(&e) => Ok(history::AppliedMap::new()),
        Err(e) => Err(e),
    }
}

/// 校验迁移清单的内部一致性。
///
/// 两类冲突都会破坏记录表语义，故在加载阶段即拒绝：
///
/// 1. **同名不同版本**：记录表以 `(kind, name)` 为主键，同名迁移只能有一个版本；
/// 2. **同版本不同名**：同一 `kind` 下版本号必须唯一，否则应用顺序不可判定。
///
/// # 错误
///
/// 命中「同名不同版本」返回 [`DbError::Migration`]（属于清单编排问题），
/// 命中「同版本不同名」返回 [`DbError::InvalidValue`]（属于调用方输入的版本号重复）。
fn check_identity_conflicts(list: &[LoadedMigration]) -> Result<()> {
    // 名称 → 版本：查「同名不同版本」
    let mut names: HashMap<(MigrationKind, String), Option<Version>> = HashMap::new();
    // (种类, 版本) → 名称：查「同版本不同名」
    let mut versions: HashMap<(MigrationKind, Option<Version>), String> = HashMap::new();

    for migration in list {
        let name_key = (migration.kind, migration.name.clone());
        match names.get(&name_key) {
            Some(previous) if *previous != migration.version => {
                return Err(DbError::Migration(format!(
                    "migration name '{}' is used by two versions ({} and {}); \
                     the record table keys on (kind, name), so names must be unique",
                    migration.name,
                    previous
                        .as_ref()
                        .map(Version::to_string)
                        .unwrap_or_default(),
                    migration
                        .version
                        .as_ref()
                        .map(Version::to_string)
                        .unwrap_or_default(),
                )));
            }
            Some(_) => {}
            None => {
                names.insert(name_key, migration.version.clone());
            }
        }

        // 只有带版本的迁移参与版本唯一性检查（可重复 / 种子类迁移无版本）
        let Some(version) = migration.version.as_ref() else {
            continue;
        };
        let version_key = (migration.kind, Some(version.clone()));
        match versions.get(&version_key) {
            Some(existing_name) if existing_name != &migration.name => {
                return Err(DbError::InvalidValue(format!(
                    "duplicate migration version {version}: used by both '{existing_name}' \
                     and '{}'; versions must be unique within a migration kind",
                    migration.name
                )));
            }
            Some(_) => {}
            None => {
                versions.insert(version_key, migration.name.clone());
            }
        }
    }
    Ok(())
}

/// 构造「最近应用的迁移不属于本次来源」错误。
fn out_of_scope_error(record: &history::AppliedRecord, list_len: usize) -> DbError {
    DbError::Migration(format!(
        "latest applied migration version {} (`{}`) is not in this migrator's sources \
         ({} migrations loaded); roll it back from its own migration list first \
         (down must be the exact reverse of up)",
        record
            .version
            .as_ref()
            .map(Version::to_string)
            .unwrap_or_default(),
        record.name,
        list_len
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一条内存迁移（供冲突检测测试）。
    fn loaded(kind: MigrationKind, name: &str, version: &str) -> LoadedMigration {
        LoadedMigration {
            kind,
            version: Some(Version::new(version)),
            name: name.to_string(),
            up: String::new(),
            down: String::new(),
            source: None,
            is_no_transaction: false,
            checksum_cache: std::sync::OnceLock::new(),
        }
    }

    /// 同名不同版本必须报错（记录表以 `(kind, name)` 为主键）。
    #[test]
    fn rejects_same_name_with_two_versions() {
        let list = vec![
            loaded(MigrationKind::Versioned, "system", "1"),
            loaded(MigrationKind::Versioned, "system", "2"),
        ];
        assert!(check_identity_conflicts(&list).is_err());
    }

    /// 同版本不同名必须报错（版本号在迁移种类内唯一）。
    #[test]
    fn rejects_two_names_with_same_version() {
        let list = vec![
            loaded(MigrationKind::Versioned, "create_widgets", "1"),
            loaded(MigrationKind::Versioned, "create_gadgets", "1"),
        ];
        let err = check_identity_conflicts(&list).expect_err("duplicate version must fail");
        assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
    }

    /// 同名同版本（up/down 配对后合并）与不同名不报错。
    #[test]
    fn accepts_unique_names() {
        let list = vec![
            loaded(MigrationKind::Versioned, "system", "1"),
            loaded(MigrationKind::Versioned, "product", "2"),
        ];
        check_identity_conflicts(&list).expect("unique names must pass");
    }

    /// 构造一条已应用记录（供待应用判定测试）。
    fn record(
        migration: &LoadedMigration,
        checksum: Option<String>,
        is_success: bool,
    ) -> history::AppliedRecord {
        history::AppliedRecord {
            kind: migration.kind,
            name: migration.name.clone(),
            version: migration.version.clone(),
            checksum,
            applied_via: AppliedVia::Executed,
            seq: 1,
            is_success,
        }
    }

    /// 快路径与执行路径共用的待应用判定：
    /// 基线跳过；成功记录跳过（可重复迁移内容变化时重跑）；失败 / 缺失记录重跑。
    #[test]
    fn needs_apply_matches_record_state() {
        let versioned = loaded(MigrationKind::Versioned, "create_widgets", "1");
        let seed = LoadedMigration {
            version: None,
            up: "INSERT INTO demo VALUES (1);".to_string(),
            ..loaded(MigrationKind::Seed, "dict_data", "")
        };
        let baseline = loaded(MigrationKind::Baseline, "base_1", "1");

        let empty = history::AppliedMap::new();
        assert!(!needs_apply(&baseline, &empty), "基线只写标记，不执行 SQL");
        assert!(needs_apply(&versioned, &empty), "记录缺失 → 执行");
        assert!(needs_apply(&seed, &empty), "记录缺失 → 执行");

        // 版本化成功记录：指纹差异由 verify_checksums 中止启动，不在这里重跑
        let mut applied = history::AppliedMap::new();
        applied.insert(
            (versioned.kind, versioned.name.clone()),
            record(&versioned, Some("different".into()), true),
        );
        assert!(!needs_apply(&versioned, &applied));

        // 可重复 / 种子：指纹一致跳过，变化重跑
        let mut applied = history::AppliedMap::new();
        applied.insert(
            (seed.kind, seed.name.clone()),
            record(&seed, Some(seed.checksum().to_string()), true),
        );
        assert!(!needs_apply(&seed, &applied));
        applied.insert(
            (seed.kind, seed.name.clone()),
            record(&seed, Some("different".into()), true),
        );
        assert!(needs_apply(&seed, &applied));

        // 上次失败（success = false）→ 重跑
        let mut applied = history::AppliedMap::new();
        applied.insert(
            (versioned.kind, versioned.name.clone()),
            record(&versioned, None, false),
        );
        assert!(needs_apply(&versioned, &applied));
    }

    /// 默认选项符合第一期约定（拆分开、分段开、事务开、强校验）。
    #[test]
    fn default_options_match_phase_one() {
        let options = MigratorOptions::default();
        assert!(options.split_statements);
        assert!(options.up_down_sections);
        assert!(options.transaction_default);
        assert!(options.validate_checksum);
        assert_eq!(options.divergent, Divergent::Abort);
        assert!(options.version_prefix.is_empty());
        assert!(options.name_separator.is_empty());
    }
}

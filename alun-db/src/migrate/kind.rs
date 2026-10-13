//! 迁移种类、状态视图与方言类别。

use std::fmt;
use std::path::PathBuf;

use super::version::Version;

/// 迁移种类。
///
/// 与记录表 `schema_migrations.kind` 列一一对应；第一期仅产出并执行
/// [`MigrationKind::Versioned`]，其余变体为后续能力预留（其语义已在记录表中固化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MigrationKind {
    /// 版本化迁移：按版本号升序执行一次。
    Versioned,
    /// 可重复迁移：内容指纹变化即重跑（如视图 / 函数）。
    Repeatable,
    /// 种子数据：幂等重灌的初始化数据。
    Seed,
    /// 基线：存量库接入时把 `<= version` 标记为已应用。
    Baseline,
}

impl MigrationKind {
    /// 记录表 `kind` 列的取值。
    pub fn as_str(self) -> &'static str {
        match self {
            MigrationKind::Versioned => "versioned",
            MigrationKind::Repeatable => "repeatable",
            MigrationKind::Seed => "seed",
            MigrationKind::Baseline => "baseline",
        }
    }

    /// 由记录表 `kind` 列还原；无法识别时返回 `None`。
    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "versioned" => Some(MigrationKind::Versioned),
            "repeatable" => Some(MigrationKind::Repeatable),
            "seed" => Some(MigrationKind::Seed),
            "baseline" => Some(MigrationKind::Baseline),
            _ => None,
        }
    }

    /// 一次运行内的执行次序（越小越先）。
    ///
    /// 基线必须**最先**生效：它把 `<= version` 的迁移标记为已应用，若排在
    /// 版本化迁移之后，那些迁移早已被真正执行过了。
    pub fn execution_rank(self) -> u8 {
        match self {
            MigrationKind::Baseline => 0,
            MigrationKind::Versioned => 1,
            MigrationKind::Repeatable => 2,
            MigrationKind::Seed => 3,
        }
    }

    /// 该种类的迁移是否以「名称」而非「版本号」定位。
    ///
    /// 可重复 / 种子迁移没有版本，重跑判定完全依赖内容指纹。
    pub fn is_named(self) -> bool {
        matches!(self, MigrationKind::Repeatable | MigrationKind::Seed)
    }

    /// 该种类的内容变化时是否应**重跑**（而非报 `MigrationChecksum`）。
    pub fn is_rerunnable(self) -> bool {
        self.is_named()
    }
}

impl fmt::Display for MigrationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 记录表里一行的**来源**：这条迁移的 `up` 究竟是谁执行的。
///
/// 决定了 [`super::Migrator::reset`] 能否执行它的 `down` 脚本，因此必须显式记录、
/// 不能靠「`checksum` 是否为空」推断——历史行会在首次运行时补录指纹，那之后
/// 两者就无法区分了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppliedVia {
    /// 本框架执行过 `up`，指纹即所执行内容的哈希 → 可安全回滚。
    Executed,
    /// 基线标记：从未执行过任何 SQL（存量库接入时假定「已存在」）→ **不可回滚**。
    Baseline,
    /// 旧框架 `_sqlx_migrations` 导入：由旧框架执行过 → 可回滚。
    Imported,
}

impl AppliedVia {
    /// 记录表 `applied_via` 列的取值。
    pub fn as_str(self) -> &'static str {
        match self {
            AppliedVia::Executed => "executed",
            AppliedVia::Baseline => "baseline",
            AppliedVia::Imported => "imported",
        }
    }

    /// 由记录表 `applied_via` 列还原；无法识别时返回 `None`。
    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "executed" => Some(AppliedVia::Executed),
            "baseline" => Some(AppliedVia::Baseline),
            "imported" => Some(AppliedVia::Imported),
            _ => None,
        }
    }

    /// 该行的 `up` 是否由「某个真实执行者」跑过，因而可以安全执行 `down`。
    pub fn can_roll_back(self) -> bool {
        matches!(self, AppliedVia::Executed | AppliedVia::Imported)
    }
}

impl fmt::Display for AppliedVia {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 单条迁移相对当前库的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationState {
    /// 尚未应用，本次运行会被执行。
    Pending,
    /// 已应用且内容指纹一致。
    Applied,
    /// 已应用但内容指纹不一致（已应用迁移被改写）。
    Divergent,
    /// 库中有记录但当前迁移清单里没有对应文件（清单外的版本）。
    Missing,
    /// 库中有记录但文件已被删除（可重复 / 种子类）。
    Deleted,
}

impl MigrationState {
    /// 状态的可读名称。
    pub fn as_str(self) -> &'static str {
        match self {
            MigrationState::Pending => "pending",
            MigrationState::Applied => "applied",
            MigrationState::Divergent => "divergent",
            MigrationState::Missing => "missing",
            MigrationState::Deleted => "deleted",
        }
    }
}

impl fmt::Display for MigrationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 单条迁移的状态视图（`status` / `plan` 的输出元素）。
#[derive(Debug, Clone)]
pub struct MigrationStatus {
    /// 迁移种类。
    pub kind: MigrationKind,
    /// 版本号；可重复 / 种子类迁移没有版本。
    pub version: Option<Version>,
    /// 迁移名称（记录表自然键的一部分）。
    pub name: String,
    /// 来源文件路径（内存清单构造的迁移没有来源）。
    pub source: Option<PathBuf>,
    /// 库内是否已有该迁移的记录。
    pub is_applied: bool,
    /// 库内记录的指纹；历史行 / 尚未应用时为 `None`。
    pub applied_checksum: Option<String>,
    /// 本次运行计算的当前指纹。
    pub current_checksum: String,
    /// 综合状态。
    pub state: MigrationState,
}

/// SQL 方言类别。
///
/// 语句拆分器据此决定转义与引用规则（MySQL 的 `#` 注释 / 反引号 / 反斜杠转义、
/// PostgreSQL 的美元引用等）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialectKind {
    /// PostgreSQL。
    Postgres,
    /// MySQL。
    Mysql,
    /// SQLite。
    Sqlite,
}

impl DialectKind {
    /// 由驱动方言名（[`alun_db_core::traits::Dialect::name`]）判定方言类别。
    ///
    /// 未知名称按 SQLite 处理：三者中 SQLite 的字符串 / 注释规则最保守，
    /// 不会引入 PostgreSQL 美元引用或 MySQL 反斜杠转义带来的误切。
    pub fn from_dialect_name(name: &str) -> Self {
        match name {
            "mysql" => DialectKind::Mysql,
            "postgres" | "postgresql" => DialectKind::Postgres,
            _ => DialectKind::Sqlite,
        }
    }
}

impl fmt::Display for DialectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DialectKind::Postgres => "postgres",
            DialectKind::Mysql => "mysql",
            DialectKind::Sqlite => "sqlite",
        })
    }
}

//! 迁移文件的命名解析与加载。
//!
//! 真实项目的迁移目录并存多种命名风格，解析器**不做唯一锁定**（设计文档 §3.1）：
//!
//! | 风格 | 例子 | 解析结果 |
//! |---|---|---|
//! | 前缀 + 双下划线 | `V9__add_column.sql` | version=`9`, name=`add_column` |
//! | 前缀 + 单下划线 | `9_add_column.sql` | version=`9`, name=`add_column` |
//! | 纯数字起始 | `01_base_tables_init.sql` | version=`1`, name=`base_tables_init` |
//! | 可重复 | `R__create_view.sql` | kind=repeatable, name=`create_view` |
//! | 种子 | `S__dict_data.sql` | kind=seed, name=`dict_data` |
//! | 基线标记 | `B3__baseline.sql` | kind=baseline, version=`3` |
//!
//! 后缀决定方向：`.up.sql` / `.down.sql` 成对可逆，`.sql` 单向（整文件即 up，
//! 除非含 `-- +migrate Down` 分段）。
//!
//! 目录加载递归进入子目录（支持按业务域分目录）。版本化 / 基线迁移按版本号定位、
//! 须跨目录全局唯一；可重复 / 种子迁移按名称定位，不参与版本号唯一性检查。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use alun_db_core::error::{DbError, Result};

use super::MigratorOptions;
use super::kind::MigrationKind;
use super::splitter::{parse_directives, split_sections};
use super::version::Version;

/// 从文件加载的一条待执行迁移（拥有型）。
///
/// 区别于编程式清单里的 [`super::Migration`]：目录迁移的 SQL 来自磁盘，
/// 且附带来源路径与指令信息。
#[derive(Debug, Clone)]
pub(crate) struct LoadedMigration {
    /// 迁移种类。
    pub kind: MigrationKind,
    /// 版本号；可重复 / 种子类迁移没有版本。
    pub version: Option<Version>,
    /// 迁移名称（记录表自然键的一部分）。
    pub name: String,
    /// 正向脚本；基线标记为空串（不作为 SQL 执行）。
    pub up: String,
    /// 回退脚本；单向迁移与基线标记为空串。
    pub down: String,
    /// 来源文件路径；内存清单构造的迁移为 `None`。
    pub source: Option<PathBuf>,
    /// 是否禁用事务包装（`-- +no-transaction`）。
    pub is_no_transaction: bool,
}

impl LoadedMigration {
    /// 迁移的执行排序键。
    ///
    /// 次序为 **基线 → 版本化（版本升序）→ 可重复 → 种子（按名称升序）**：
    /// 基线必须最先生效，否则它要「标记为已应用」的版本早已被真正执行过。
    pub fn sort_key(&self) -> (u8, Option<Version>, String) {
        (
            self.kind.execution_rank(),
            self.version.clone(),
            self.name.clone(),
        )
    }
}

/// 迁移文件的方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotDirection {
    /// `xxx.up.sql` 或单向的 `xxx.sql`。
    Up,
    /// `xxx.down.sql`。
    Down,
}

/// 文件名解析结果（不含文件内容）。
#[derive(Debug, Clone)]
struct ParsedName {
    /// 迁移种类。
    kind: MigrationKind,
    /// 版本号；可重复 / 种子类没有版本。
    version: Option<Version>,
    /// 迁移名称。
    name: String,
}

/// 同一迁移在目录中累积的两个方向的槽位。
#[derive(Debug, Default)]
struct SlotEntry {
    /// 名称。
    name: String,
    /// 正向脚本。
    up: Option<String>,
    /// 回退脚本。
    down: Option<String>,
    /// 来源文件路径（优先取 up 文件的路径）。
    source: Option<PathBuf>,
    /// 是否禁用事务包装。
    is_no_transaction: bool,
}

/// 按「版本号」定位的槽位集合（版本化 + 基线迁移）。
type VersionedSlots = BTreeMap<(MigrationKind, Version), SlotEntry>;
/// 按「名称」定位的槽位集合（可重复 + 种子迁移）。
type NamedSlots = BTreeMap<(MigrationKind, String), SlotEntry>;

/// 一次加载累积的全部槽位。
///
/// 两个映射按各自的自然键（版本号 / 名称）定位：版本化与基线迁移靠版本号，
/// 可重复与种子迁移靠名称。
#[derive(Debug, Default)]
struct Slots {
    /// 版本化与基线迁移。
    versioned: VersionedSlots,
    /// 可重复与种子迁移。
    named: NamedSlots,
}

impl Slots {
    /// 收敛为迁移清单。
    ///
    /// # 错误
    ///
    /// 某版本只有 `down` 没有 `up` 时返回 [`DbError::Migration`]（判定须等全部
    /// 文件收集完毕，因为目录遍历顺序不保证 `up` 先于 `down`）。
    fn into_migrations(self) -> Result<Vec<LoadedMigration>> {
        let mut out = Vec::with_capacity(self.versioned.len() + self.named.len());
        for ((kind, version), entry) in self.versioned {
            let (up, down) = resolve_scripts(kind, &entry, &version.to_string())?;
            out.push(LoadedMigration {
                kind,
                version: Some(version),
                name: entry.name,
                up,
                down,
                source: entry.source,
                is_no_transaction: entry.is_no_transaction,
            });
        }
        for ((kind, name), entry) in self.named {
            let (up, down) = resolve_scripts(kind, &entry, &name)?;
            out.push(LoadedMigration {
                kind,
                version: None,
                name,
                up,
                down,
                source: entry.source,
                is_no_transaction: entry.is_no_transaction,
            });
        }
        Ok(out)
    }
}

/// 从多个来源目录加载迁移，按执行顺序返回。
///
/// # 参数
///
/// - `sources`：版本化 / 基线迁移的来源目录（递归扫描，版本号跨目录全局唯一）。
/// - `seed_sources`：种子数据目录；其中所有 `.sql` 文件一律按
///   [`MigrationKind::Seed`] 装载，文件名即种子名（无需 `S__` 前缀）。
/// - `options`：解析选项（版本前缀、名称分隔符、双段式开关）。
///
/// # 返回
///
/// 按 [`LoadedMigration::sort_key`] 升序的迁移清单。
///
/// # 错误
///
/// 前缀配置不自洽（重复 / 数字开头）、目录读取失败、文件名无法解析、版本或名称
/// 重复、有 `down` 无 `up` 时返回 [`DbError`]。
pub(crate) fn load_from_sources(
    sources: &[PathBuf],
    seed_sources: &[PathBuf],
    options: &MigratorOptions,
) -> Result<Vec<LoadedMigration>> {
    validate_options(options)?;
    let mut slots = Slots::default();
    for dir in sources {
        collect_dir(dir, options, &mut slots)?;
    }
    for dir in seed_sources {
        collect_seed_dir(dir, options, &mut slots)?;
    }
    slots.into_migrations()
}

/// 从编译期嵌入的 `(文件名, 内容)` 对加载迁移。
///
/// 与目录扫描共用同一套文件名解析与槽位合并逻辑（[`merge_slot`]），因此分类、
/// 冲突判定与 `V{...}` / `R__` / `S__` / `B{...}` 语义完全一致。
///
/// # 参数
///
/// - `pairs`：`(文件名, 文件内容)`；通常由 `migrate!` 宏在编译期用
///   `include_str!` 生成，故内容为 `'static`。
/// - `options`：解析选项。
///
/// # 返回
///
/// 按 [`LoadedMigration::sort_key`] 升序的迁移清单。
///
/// # 错误
///
/// 前缀配置不自洽、文件名无法解析、版本或名称重复时返回 [`DbError`]。
pub(crate) fn load_from_embedded(
    pairs: &[(&'static str, &'static str)],
    options: &MigratorOptions,
) -> Result<Vec<LoadedMigration>> {
    validate_options(options)?;
    let mut slots = Slots::default();
    for (filename, content) in pairs {
        merge_slot(filename, content, None, false, options, &mut slots)?;
    }
    slots.into_migrations()
}

/// 加载单个迁移文件（`run_file` 入口）。
///
/// 版本号从文件名推断；不接受 `.down.sql`（回退文件不是可执行的迁移），
/// 也不接受 `B{n}__` 基线标记（它不是 SQL 脚本）。
///
/// # 参数
///
/// - `path`：迁移文件路径。
/// - `options`：解析选项。
///
/// # 返回
///
/// 该文件的迁移。
///
/// # 错误
///
/// 文件名无法解析、文件为 `.down.sql`、或为基线标记时返回 [`DbError`]。
pub(crate) fn load_single_file(path: &Path, options: &MigratorOptions) -> Result<LoadedMigration> {
    validate_options(options)?;
    let filename = file_name_of(path)?;
    let (parsed, direction) = parse_filename(&filename, options)?.ok_or_else(|| {
        DbError::Migration(format!(
            "unparseable migration filename '{filename}'; expected `.sql` or `.{{up,down}}.sql`"
        ))
    })?;
    if direction == SlotDirection::Down {
        return Err(DbError::Migration(format!(
            "'{filename}' is a down migration; run_file expects an up script"
        )));
    }
    if parsed.kind == MigrationKind::Baseline {
        return Err(DbError::Migration(format!(
            "'{filename}' is a baseline marker, not an executable script; \
             use `Migrator::baseline` to mark versions as applied"
        )));
    }

    load_single_file_as(path, options, parsed.kind, parsed.version, parsed.name)
}

/// 加载单个迁移文件，**身份由调用方给定**（不做文件名解析）。
///
/// 供文件名不符合命名规范的补丁脚本使用（[`super::Migrator::run_file_as`]）：
/// 这类文件仍按同一套协议切分 `-- +migrate Up/Down` 分段与指令，只是版本与
/// 名称由调用方显式提供。
///
/// # 参数
///
/// - `path`：迁移文件路径。
/// - `options`：解析选项。
/// - `kind`：迁移种类。
/// - `version`：版本号；可重复 / 种子类迁移传 `None`。
/// - `name`：迁移名称。
///
/// # 返回
///
/// 该文件的迁移。
///
/// # 错误
///
/// 配置不自洽、文件读取失败或占位符未定义时返回 [`DbError`]。
pub(crate) fn load_single_file_as(
    path: &Path,
    options: &MigratorOptions,
    kind: MigrationKind,
    version: Option<Version>,
    name: String,
) -> Result<LoadedMigration> {
    validate_options(options)?;
    let content = read_migration_sql(path)?;
    let label = path.display().to_string();
    let content = apply_placeholders(&content, options, &label)?;
    let (up, down) = split_scripts(&content, options);
    let is_no_transaction = parse_directives(&up).is_no_transaction;

    Ok(LoadedMigration {
        kind,
        version,
        name,
        up,
        down,
        source: Some(path.to_path_buf()),
        is_no_transaction,
    })
}

/// 校验槽位完整性并取出正 / 回退脚本。
///
/// 基线标记不作为 SQL 执行，因此其脚本被显式置空（内容由 `baseline` 写入记录）。
fn resolve_scripts(
    kind: MigrationKind,
    entry: &SlotEntry,
    identity: &str,
) -> Result<(String, String)> {
    if kind == MigrationKind::Baseline {
        return Ok((String::new(), String::new()));
    }
    let Some(up) = entry.up.as_ref() else {
        return Err(DbError::Migration(format!(
            "migration '{identity}' has a down file but no corresponding up file"
        )));
    };
    Ok((up.clone(), entry.down.clone().unwrap_or_default()))
}

/// 递归收集目录及其子目录下的迁移文件。
///
/// 每个文件读出内容后交由 [`merge_slot`] 统一分类合并——磁盘扫描与编译期嵌入
/// 因此共用同一套解析、分类与冲突判定，不存在「嵌入迁移行为不一致」的可能。
///
/// 符号链接不跟随（`file_type()` 不解引用），因此不会因软链成环而无限递归。
fn collect_dir(dir: &Path, options: &MigratorOptions, slots: &mut Slots) -> Result<()> {
    let read_dir = fs::read_dir(dir)
        .map_err(|e| DbError::Migration(format!("read migrations dir '{}': {e}", dir.display())))?;

    for entry in read_dir {
        let entry = entry.map_err(|e| {
            DbError::Migration(format!("list migrations dir '{}': {e}", dir.display()))
        })?;
        let file_type = entry
            .file_type()
            .map_err(|e| DbError::Migration(format!("inspect {}: {e}", entry.path().display())))?;

        if file_type.is_dir() {
            collect_dir(&entry.path(), options, slots)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let filename = entry.file_name().to_string_lossy().into_owned();
        if strip_sql_suffix(&filename).is_none() {
            continue;
        }
        let content = read_migration_sql(&entry.path())?;
        merge_slot(
            &filename,
            &content,
            Some(&entry.path()),
            false,
            options,
            slots,
        )?;
    }
    Ok(())
}

/// 收集整目录的种子脚本：所有 `.sql` 一律按 [`MigrationKind::Seed`] 装载。
fn collect_seed_dir(dir: &Path, options: &MigratorOptions, slots: &mut Slots) -> Result<()> {
    let read_dir = fs::read_dir(dir)
        .map_err(|e| DbError::Migration(format!("read seed dir '{}': {e}", dir.display())))?;

    for entry in read_dir {
        let entry = entry
            .map_err(|e| DbError::Migration(format!("list seed dir '{}': {e}", dir.display())))?;
        let file_type = entry
            .file_type()
            .map_err(|e| DbError::Migration(format!("inspect {}: {e}", entry.path().display())))?;

        if file_type.is_dir() {
            collect_seed_dir(&entry.path(), options, slots)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        let filename = entry.file_name().to_string_lossy().into_owned();
        if strip_sql_suffix(&filename).is_none() {
            continue;
        }
        let content = read_migration_sql(&entry.path())?;
        merge_slot(
            &filename,
            &content,
            Some(&entry.path()),
            true,
            options,
            slots,
        )?;
    }
    Ok(())
}

/// 装载一个迁移文件：解析文件名 → 按种类合并到对应槽位。
///
/// 这是**唯一**的「文件名 + 内容 → 槽位」实现，磁盘扫描与编译期嵌入都走这里。
///
/// # 参数
///
/// - `filename`：文件名（含 `.sql` 后缀）。
/// - `content`：文件内容。
/// - `path`：来源路径；编译期嵌入的迁移为 `None`（`status` 因此不显示来源）。
/// - `is_seed_dir`：是否来自种子目录——为真时**强制**按 [`MigrationKind::Seed`]
///   装载，文件名主干即种子名（不要求 `S__` 前缀）。
/// - `options`：解析选项（含占位符表）。
/// - `slots`：累积中的槽位集合。
///
/// # 错误
///
/// 文件名无法解析、版本 / 名称重复、基线标记内含语句、占位符未定义时返回
/// [`DbError`]。
fn merge_slot(
    filename: &str,
    content: &str,
    path: Option<&Path>,
    is_seed_dir: bool,
    options: &MigratorOptions,
    slots: &mut Slots,
) -> Result<()> {
    let Some((stem, direction)) = strip_sql_suffix(filename) else {
        return Ok(());
    };

    // 占位符替换在分段 / 拆分之前完成：指纹因此覆盖「实际执行的 SQL」，
    // 占位符取值改变会被 checksum 检出，而不是静默跑出不同的 SQL。
    let content = apply_placeholders(content, options, filename)?;

    if is_seed_dir {
        if stem.trim().is_empty() {
            return Err(DbError::Migration(format!(
                "seed file '{filename}' has an empty name"
            )));
        }
        // 显式写了 `S__` / `R__` 前缀则取其名称，否则整个主干即种子名
        let name = match parse_stem(stem, options) {
            Ok(parsed) if parsed.kind.is_named() => parsed.name,
            _ => stem.to_string(),
        };
        return collect_named(
            filename,
            &content,
            path,
            ParsedName {
                kind: MigrationKind::Seed,
                version: None,
                name,
            },
            direction,
            options,
            slots,
        );
    }

    let parsed = parse_stem(stem, options).map_err(|_| unparseable(filename))?;
    match parsed.kind {
        MigrationKind::Versioned | MigrationKind::Baseline => {
            collect_versioned(filename, &content, path, parsed, direction, options, slots)
        }
        MigrationKind::Repeatable | MigrationKind::Seed => {
            collect_named(filename, &content, path, parsed, direction, options, slots)
        }
    }
}

/// 把一个版本化 / 基线迁移文件合并到版本号定位的槽位。
fn collect_versioned(
    filename: &str,
    content: &str,
    path: Option<&Path>,
    parsed: ParsedName,
    direction: SlotDirection,
    options: &MigratorOptions,
    slots: &mut Slots,
) -> Result<()> {
    let version = parsed
        .version
        .clone()
        .ok_or_else(|| DbError::Migration(format!("migration '{filename}' requires a version")))?;
    let label = source_label(filename, path);

    if parsed.kind == MigrationKind::Baseline && direction == SlotDirection::Down {
        return Err(DbError::Migration(format!(
            "'{filename}' is a baseline marker, which has no down direction"
        )));
    }

    let slot = slots
        .versioned
        .entry((parsed.kind, version.clone()))
        .or_default();
    match direction {
        SlotDirection::Down => {
            if slot.down.is_some() {
                return Err(duplicate_direction_error(
                    "down",
                    &Some(version),
                    &label,
                    slot.source.as_deref(),
                ));
            }
            slot.down = Some(content.to_string());
        }
        SlotDirection::Up => {
            if slot.up.is_some() {
                return Err(duplicate_direction_error(
                    "up",
                    &Some(version),
                    &label,
                    slot.source.as_deref(),
                ));
            }
            if parsed.kind == MigrationKind::Baseline {
                reject_baseline_statements(content, filename)?;
                // 基线标记只声明「<= version 视为已应用」，本身不是 SQL 脚本
                slot.up = Some(String::new());
            } else {
                let (up, down) = split_scripts(content, options);
                slot.is_no_transaction = parse_directives(&up).is_no_transaction;
                slot.up = Some(up);
                if !down.trim().is_empty() && slot.down.is_none() {
                    slot.down = Some(down);
                }
            }
            slot.name = parsed.name;
            slot.source = path.map(Path::to_path_buf);
        }
    }
    Ok(())
}

/// 校验基线标记文件内不含可执行 SQL。
///
/// 基线标记的语义是「把 `<= version` 的迁移标记为已应用」，它不执行任何语句。
/// 若作者在其中写了 DDL，静默跳过会让人误以为建表已经发生，故显式报错并给出
/// 正确做法（把 DDL 放进版本化迁移）。
fn reject_baseline_statements(content: &str, filename: &str) -> Result<()> {
    let statements = super::splitter::split_statements(content, super::kind::DialectKind::Sqlite)?;
    if statements.is_empty() {
        return Ok(());
    }
    Err(DbError::Migration(format!(
        "'{filename}' is a baseline marker but contains {} executable statement(s); \
         a baseline only declares `<= version` as already applied — \
         move the DDL into a versioned migration",
        statements.len()
    )))
}

/// 把一个可重复 / 种子迁移文件合并到名称定位的槽位。
fn collect_named(
    filename: &str,
    content: &str,
    path: Option<&Path>,
    parsed: ParsedName,
    direction: SlotDirection,
    options: &MigratorOptions,
    slots: &mut Slots,
) -> Result<()> {
    if parsed.version.is_some() {
        return Err(DbError::Migration(format!(
            "'{filename}' is a {} migration and must not carry a version",
            parsed.kind
        )));
    }
    if parsed.name.trim().is_empty() {
        return Err(DbError::Migration(format!(
            "'{filename}' has an empty migration name"
        )));
    }
    let label = source_label(filename, path);

    let slot = slots
        .named
        .entry((parsed.kind, parsed.name.clone()))
        .or_default();
    match direction {
        SlotDirection::Down => {
            if slot.down.is_some() {
                return Err(duplicate_direction_error(
                    "down",
                    &None,
                    &label,
                    slot.source.as_deref(),
                ));
            }
            slot.down = Some(content.to_string());
        }
        SlotDirection::Up => {
            if slot.up.is_some() {
                return Err(duplicate_direction_error(
                    "up",
                    &None,
                    &label,
                    slot.source.as_deref(),
                ));
            }
            let (up, down) = split_scripts(content, options);
            slot.is_no_transaction = parse_directives(&up).is_no_transaction;
            slot.name = parsed.name;
            slot.source = path.map(Path::to_path_buf);
            slot.up = Some(up);
            if !down.trim().is_empty() && slot.down.is_none() {
                slot.down = Some(down);
            }
        }
    }
    Ok(())
}

/// 来源标签：磁盘来源用完整路径，嵌入来源用文件名（`include_str!` 已把内容编入二进制）。
fn source_label(filename: &str, path: Option<&Path>) -> String {
    match path {
        Some(path) => path.display().to_string(),
        None => format!("<embedded>/{filename}"),
    }
}

/// 用配置的占位符表替换脚本中的 `${key}`。
///
/// **未配置占位符时不做任何扫描**——这保证含 `${` 的字符串字面量（如导入模板
/// 数据时的 `LIKE '%${x}%'`）不会被误伤；只有显式配置了占位符的项目才启用替换。
///
/// 单趟扫描并按键精确查表，因此键名互为前缀（`A` 与 `AB`）也不会相互干扰。
///
/// # 错误
///
/// 出现未闭合的 `${`、或引用了未配置的键时返回 [`DbError`]。未知占位符**绝不
/// 静默保留**——那会变成一条语法错误的 SQL，错误现场远离真正的原因。
fn apply_placeholders(sql: &str, options: &MigratorOptions, filename: &str) -> Result<String> {
    if options.placeholders.is_empty() {
        return Ok(sql.to_string());
    }

    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            return Err(DbError::Migration(format!(
                "migration '{filename}' contains an unterminated `${{` placeholder"
            )));
        };
        let key = &after[..end];
        let Some(value) = options.placeholders.get(key) else {
            return Err(DbError::InvalidValue(format!(
                "migration '{filename}' references unknown placeholder `${{{key}}}`; \
                 configured placeholders: {}",
                known_placeholder_keys(options)
            )));
        };
        out.push_str(value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// 已配置占位符键的有序列表（用于错误文案，排序保证输出稳定）。
fn known_placeholder_keys(options: &MigratorOptions) -> String {
    if options.placeholders.is_empty() {
        return "<none>".to_string();
    }
    let mut keys: Vec<&str> = options.placeholders.keys().map(String::as_str).collect();
    keys.sort_unstable();
    keys.join(", ")
}

/// 按选项切分单文件 `-- +migrate Up/Down` 双段式。
fn split_scripts(content: &str, options: &MigratorOptions) -> (String, String) {
    if options.up_down_sections {
        let sections = split_sections(content);
        (sections.up, sections.down)
    } else {
        (content.to_string(), String::new())
    }
}

/// 构造「同一迁移在同一方向上出现两次」的错误。
///
/// 多来源场景下重复迁移往往分处两个目录，故错误文案必须同时给出**两个路径**，
/// 否则调用方在自己的目录里只看到一个文件，无从定位冲突。
fn duplicate_direction_error(
    direction: &str,
    version: &Option<Version>,
    current: &str,
    previous: Option<&Path>,
) -> DbError {
    let identity = match version {
        Some(version) => format!("version {version}"),
        None => "the same name".to_string(),
    };
    let previous = previous
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<unknown source>".to_string());
    DbError::Migration(format!(
        "duplicate {direction} migration for {identity}: '{current}' conflicts with \
         '{previous}'; identities must be globally unique across all sources"
    ))
}

/// 解析迁移文件名（不读文件内容）。
///
/// # 返回
///
/// `Ok(Some(..))`：匹配到合法迁移文件名；`Ok(None)`：非 `.sql` 的旁路文件（跳过）；
/// 其余为无法解析的 `.sql` 文件（[`DbError::Migration`]）。
fn parse_filename(
    filename: &str,
    options: &MigratorOptions,
) -> Result<Option<(ParsedName, SlotDirection)>> {
    let Some((stem, direction)) = strip_sql_suffix(filename) else {
        return Ok(None);
    };
    Ok(Some((
        parse_stem(stem, options).map_err(|_| unparseable(filename))?,
        direction,
    )))
}

/// 剥离 `.down.sql` / `.up.sql` / `.sql` 后缀并给方向。
///
/// 判定顺序：先试 `.down.sql` / `.up.sql`，再试 `.sql`，避免 `V1__x.up.sql`
/// 被 `.sql` 前缀误吞。非 `.sql` 文件返回 `None`（跳过）。
fn strip_sql_suffix(filename: &str) -> Option<(&str, SlotDirection)> {
    if let Some(stem) = filename.strip_suffix(".down.sql") {
        return Some((stem, SlotDirection::Down));
    }
    if let Some(stem) = filename.strip_suffix(".up.sql") {
        return Some((stem, SlotDirection::Up));
    }
    filename
        .strip_suffix(".sql")
        .map(|stem| (stem, SlotDirection::Up))
}

/// 从文件名主干（已剥离 `.sql` 后缀）解析出种类、版本与名称。
fn parse_stem(stem: &str, options: &MigratorOptions) -> Result<ParsedName> {
    let separator = (!options.name_separator.is_empty()).then_some(options.name_separator.as_str());

    // 可重复 / 种子：`R__name` / `S__name`（前缀后直接是分隔符，无版本号）
    for (prefix, kind) in [
        (
            options.repeatable_prefix.as_str(),
            MigrationKind::Repeatable,
        ),
        (options.seed_prefix.as_str(), MigrationKind::Seed),
    ] {
        // 空前缀 = 关闭该种类，不做任何匹配
        if prefix.is_empty() {
            continue;
        }
        if let Some(rest) = stem.strip_prefix(prefix)
            && let Some(name) = strip_separator(rest, separator)
        {
            if name.trim().is_empty() {
                return Err(unparseable(stem));
            }
            return Ok(ParsedName {
                kind,
                version: None,
                name: name.to_string(),
            });
        }
    }

    // 基线：`B{version}__name`
    let baseline_prefix = options.baseline_prefix.as_str();
    if !baseline_prefix.is_empty()
        && let Some(rest) = stem.strip_prefix(baseline_prefix)
        && let Some((raw_version, raw_name)) = split_version_name(rest, separator)
        && is_version_literal(&raw_version)
        && !raw_name.trim().is_empty()
    {
        return Ok(ParsedName {
            kind: MigrationKind::Baseline,
            version: Some(Version::new(raw_version)),
            name: raw_name,
        });
    }

    // 版本化：可选前缀 + 版本 + 分隔符 + 名称
    let versioned =
        strip_version_prefix(stem, &options.version_prefix).ok_or_else(|| unparseable(stem))?;
    let (raw_version, raw_name) =
        split_version_name(versioned, separator).ok_or_else(|| unparseable(stem))?;
    if !is_version_literal(&raw_version) || raw_name.trim().is_empty() {
        return Err(unparseable(stem));
    }

    Ok(ParsedName {
        kind: MigrationKind::Versioned,
        version: Some(Version::new(raw_version)),
        name: raw_name,
    })
}

/// 校验加载选项的自洽性。
///
/// # 错误
///
/// 命中下列任一情况返回 [`DbError::InvalidValue`]：
///
/// 1. **前缀重复**：`repeatable_prefix` 与 `seed_prefix` 同值时，谁先匹配谁说了算，
///    另一个种类永远不可能命中；
/// 2. **前缀以数字开头**：`1__x` 会被判成可重复迁移，正常版本化迁移（jinhu 风格的
///    `01_base_tables_init.sql`）被整体遮蔽；
/// 3. **占位符值含换行**：替换发生在 `-- +migrate Up/Down` 分段与 `-- +no-transaction`
///    指令解析**之前**，含换行的值能凭空造出一行指令注释，静默改变迁移语义。
pub(crate) fn validate_options(options: &MigratorOptions) -> Result<()> {
    let mut configured: Vec<(&str, &'static str)> = Vec::new();
    for (prefix, label) in [
        (options.repeatable_prefix.as_str(), "repeatable_prefix"),
        (options.seed_prefix.as_str(), "seed_prefix"),
        (options.baseline_prefix.as_str(), "baseline_prefix"),
    ] {
        // 空串 = 显式关闭该种类，不参与唯一性检查
        if prefix.is_empty() {
            continue;
        }
        if prefix.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return Err(DbError::InvalidValue(format!(
                "{label} = '{prefix}' starts with a digit, which would shadow versioned \
                 migrations like `01_base_tables_init.sql`; use a non-numeric prefix"
            )));
        }
        if let Some((_, previous)) = configured.iter().find(|(value, _)| *value == prefix) {
            return Err(DbError::InvalidValue(format!(
                "{label} and {previous} are both configured as '{prefix}'; \
                 migration kind prefixes must be distinct (set one to \"\" to disable it)"
            )));
        }
        configured.push((prefix, label));
    }

    for (key, value) in &options.placeholders {
        if value.contains('\n') || value.contains('\r') {
            return Err(DbError::InvalidValue(format!(
                "placeholder '{key}' contains a line break; substituted values are inserted \
                 verbatim before `-- +migrate` / `-- +no-transaction` directives are parsed, \
                 so a multi-line value could forge a directive"
            )));
        }
    }
    Ok(())
}

/// 剥离版本前缀。
///
/// 配置了前缀时严格要求；未配置（空串）时接受「可选 `V` 前缀」——`V0002__x`
/// 与 `01_x` 都能解析（设计文档 §3.1 的「宽松解析」）。
fn strip_version_prefix<'a>(stem: &'a str, configured: &str) -> Option<&'a str> {
    if !configured.is_empty() {
        return stem.strip_prefix(configured);
    }
    let mut chars = stem.chars();
    match (chars.next(), chars.next()) {
        (Some('V') | Some('v'), Some(next)) if next.is_ascii_digit() => Some(&stem[1..]),
        _ => Some(stem),
    }
}

/// 把 `{version}{separator}{name}` 切成版本与名称。
///
/// 未指定分隔符时自动判定：优先 `__`，其次 `_`。
fn split_version_name(stem: &str, separator: Option<&str>) -> Option<(String, String)> {
    match separator {
        Some(sep) => {
            let (version, name) = stem.split_once(sep)?;
            Some((version.to_string(), trim_leading_separators(name, sep)))
        }
        None => {
            if let Some((version, name)) = stem.split_once("__") {
                return Some((version.to_string(), name.to_string()));
            }
            let (version, name) = stem.split_once('_')?;
            Some((version.to_string(), trim_leading_separators(name, "_")))
        }
    }
}

/// 剥掉名称开头重复出现的分隔符（严格单字符分隔符下的 `V1__x` 情形）。
fn trim_leading_separators(name: &str, separator: &str) -> String {
    name.trim_start_matches(separator).to_string()
}

/// 前缀后紧跟分隔符时返回其后的名称。
fn strip_separator<'a>(rest: &'a str, separator: Option<&str>) -> Option<&'a str> {
    match separator {
        Some(sep) => rest.strip_prefix(sep),
        None => rest.strip_prefix("__").or_else(|| rest.strip_prefix('_')),
    }
}

/// 版本字面量是否合法：非空且仅由数字、`.`、`-` 组成。
fn is_version_literal(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
}

/// 读取单个迁移文件内容。
fn read_migration_sql(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map_err(|e| DbError::Migration(format!("read migration file '{}': {e}", path.display())))
}

/// 取路径的文件名（缺失时返回错误而非静默）。
fn file_name_of(path: &Path) -> Result<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or_else(|| DbError::Migration(format!("invalid migration path '{}'", path.display())))
}

/// 构造「文件名无法解析」的迁移错误。
fn unparseable(filename: &str) -> DbError {
    DbError::Migration(format!(
        "unparseable migration filename '{filename}'; supported styles are \
         `V{{n}}__Name.{{up,down}}.sql`, `{{n}}_Name.{{up,down}}.sql`, \
         `R__Name.sql`, `S__Name.sql` and `B{{n}}__Name.sql`"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解析结果三元组：`(kind, version, name)`。
    fn parse(filename: &str) -> (MigrationKind, Option<String>, String) {
        let options = MigratorOptions::default();
        let (parsed, _) = parse_filename(filename, &options)
            .expect("parse ok")
            .expect("is sql file");
        (
            parsed.kind,
            parsed.version.map(|v| v.to_string()),
            parsed.name,
        )
    }

    /// x 项目风格：`V{n}__name.{up,down}.sql`。
    #[test]
    fn parses_v_prefixed_double_underscore() {
        assert_eq!(
            parse("V0002__create_sys_role.up.sql"),
            (
                MigrationKind::Versioned,
                Some("2".into()),
                "create_sys_role".into()
            )
        );
        assert_eq!(
            parse("V0002__create_sys_role.down.sql"),
            (
                MigrationKind::Versioned,
                Some("2".into()),
                "create_sys_role".into()
            )
        );
        assert_eq!(
            parse("V10__add_index.sql"),
            (
                MigrationKind::Versioned,
                Some("10".into()),
                "add_index".into()
            )
        );
    }

    /// jinhu 风格：`NN_name.sql`（纯数字前缀、单下划线、无 down）。
    #[test]
    fn parses_jinhu_style() {
        assert_eq!(
            parse("01_base_tables_init.sql"),
            (
                MigrationKind::Versioned,
                Some("1".into()),
                "base_tables_init".into()
            )
        );
        assert_eq!(
            parse("35_add_trigger.sql"),
            (
                MigrationKind::Versioned,
                Some("35".into()),
                "add_trigger".into()
            )
        );
    }

    /// facai 风格：`NN_name.up.sql` / `.down.sql`。
    #[test]
    fn parses_facai_style() {
        assert_eq!(
            parse("01_system.up.sql"),
            (MigrationKind::Versioned, Some("1".into()), "system".into())
        );
        assert_eq!(
            parse("31_system.down.sql"),
            (MigrationKind::Versioned, Some("31".into()), "system".into())
        );
    }

    /// 非 `.sql` 文件跳过；无法解析的 `.sql` 报错。
    #[test]
    fn skips_non_sql_and_rejects_unparseable() {
        let options = MigratorOptions::default();
        assert!(parse_filename("README.md", &options).expect("ok").is_none());
        assert!(parse_filename("notes.sql", &options).is_err());
        assert!(parse_filename("V1__.sql", &options).is_err());
    }

    /// 可重复 / 种子 / 基线种类可被识别（无版本 / 有版本）。
    #[test]
    fn recognizes_repeatable_seed_and_baseline_kinds() {
        assert_eq!(
            parse("R__create_view.sql"),
            (MigrationKind::Repeatable, None, "create_view".into())
        );
        assert_eq!(
            parse("S__dict_data.sql"),
            (MigrationKind::Seed, None, "dict_data".into())
        );
        assert_eq!(
            parse("B3__baseline.sql"),
            (MigrationKind::Baseline, Some("3".into()), "baseline".into())
        );
    }

    /// 严格版本前缀配置下，缺少前缀必须报错。
    #[test]
    fn strict_version_prefix_is_enforced() {
        let options = MigratorOptions {
            version_prefix: "V".into(),
            ..MigratorOptions::default()
        };
        assert!(parse_filename("V1__x.sql", &options).expect("ok").is_some());
        assert!(parse_filename("1_x.sql", &options).is_err());
    }

    /// 执行排序：基线最先，其后版本升序，再可重复与种子（按名称）。
    #[test]
    fn sort_key_orders_baseline_first_then_versioned_named() {
        let baseline = LoadedMigration {
            kind: MigrationKind::Baseline,
            version: Some(Version::new("5")),
            name: "baseline".into(),
            up: String::new(),
            down: String::new(),
            source: None,
            is_no_transaction: false,
        };
        let v10 = LoadedMigration {
            kind: MigrationKind::Versioned,
            version: Some(Version::new("10")),
            name: "later".into(),
            ..baseline.clone()
        };
        let v9 = LoadedMigration {
            kind: MigrationKind::Versioned,
            version: Some(Version::new("9")),
            name: "earlier".into(),
            ..baseline.clone()
        };
        let repeatable = LoadedMigration {
            kind: MigrationKind::Repeatable,
            version: None,
            name: "view".into(),
            ..baseline.clone()
        };

        let mut list = [
            v10.clone(),
            repeatable.clone(),
            v9.clone(),
            baseline.clone(),
        ];
        list.sort_by_key(|a| a.sort_key());
        let order: Vec<(MigrationKind, String)> =
            list.iter().map(|m| (m.kind, m.name.clone())).collect();
        assert_eq!(
            order,
            vec![
                (MigrationKind::Baseline, "baseline".to_string()),
                (MigrationKind::Versioned, "earlier".to_string()),
                (MigrationKind::Versioned, "later".to_string()),
                (MigrationKind::Repeatable, "view".to_string()),
            ]
        );
    }

    /// 自定义前缀被采纳：`Fn__` 作可重复、`Dict__` 作种子、`Base{n}__` 作基线。
    #[test]
    fn honors_configured_kind_prefixes() {
        let options = MigratorOptions {
            repeatable_prefix: "Fn".into(),
            seed_prefix: "Dict".into(),
            baseline_prefix: "Base".into(),
            ..MigratorOptions::default()
        };

        let parse_with = |name: &str| {
            let (parsed, _) = parse_filename(name, &options)
                .expect("parse ok")
                .expect("is sql file");
            (
                parsed.kind,
                parsed.version.map(|v| v.to_string()),
                parsed.name,
            )
        };

        assert_eq!(
            parse_with("Fn__active_view.sql"),
            (MigrationKind::Repeatable, None, "active_view".into())
        );
        assert_eq!(
            parse_with("Dict__sys_dict.sql"),
            (MigrationKind::Seed, None, "sys_dict".into())
        );
        assert_eq!(
            parse_with("Base3__baseline.sql"),
            (MigrationKind::Baseline, Some("3".into()), "baseline".into())
        );
        // 旧前缀不再被识别为可重复，而是照常走版本化解析（`R__x` 无版本 → 报错）
        assert!(parse_filename("R__old_view.sql", &options).is_err());
        assert!(parse_filename("S__old_dict.sql", &options).is_err());
    }

    /// 空前缀 = 关闭该种类：`R__x` 不再被识别为可重复迁移。
    #[test]
    fn empty_prefix_disables_the_kind() {
        let options = MigratorOptions {
            repeatable_prefix: String::new(),
            seed_prefix: String::new(),
            baseline_prefix: String::new(),
            ..MigratorOptions::default()
        };
        // 关闭后 `R__x` 落到版本化解析，因缺少版本号而报错（而不是静默变成可重复）
        assert!(parse_filename("R__create_view.sql", &options).is_err());
        assert!(parse_filename("S__dict.sql", &options).is_err());
        assert!(parse_filename("B3__baseline.sql", &options).is_err());
        // 正常版本化迁移不受影响
        assert!(
            parse_filename("V1__create_t1.sql", &options)
                .expect("ok")
                .is_some()
        );
        validate_options(&options).expect("全关是合法配置");
    }

    /// 前缀重复与数字开头都被显式拒绝。
    #[test]
    fn rejects_ambiguous_prefix_configuration() {
        let duplicated = MigratorOptions {
            repeatable_prefix: "X".into(),
            seed_prefix: "X".into(),
            ..MigratorOptions::default()
        };
        let err = validate_options(&duplicated).expect_err("重复前缀必须报错");
        assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
        assert!(err.to_string().contains("repeatable_prefix"), "{err}");
        assert!(err.to_string().contains("seed_prefix"), "{err}");

        let numeric = MigratorOptions {
            repeatable_prefix: "1".into(),
            ..MigratorOptions::default()
        };
        let err = validate_options(&numeric).expect_err("数字开头必须报错");
        assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
        assert!(err.to_string().contains("shadow"), "{err}");

        // 默认配置必须自洽
        validate_options(&MigratorOptions::default()).expect("默认前缀合法");
    }

    /// 占位符按 `${key}` 精确替换；互不为前缀的键不会互相干扰。
    #[test]
    fn substitutes_configured_placeholders() {
        let mut options = MigratorOptions::default();
        options.placeholders.insert("schema".into(), "app".into());
        options.placeholders.insert("A".into(), "1".into());
        options.placeholders.insert("AB".into(), "2".into());

        assert_eq!(
            apply_placeholders("CREATE TABLE ${schema}.t (id INT)", &options, "x.sql")
                .expect("substitute"),
            "CREATE TABLE app.t (id INT)"
        );
        // 键名互为前缀时按键精确匹配，不做「最长匹配」猜测
        assert_eq!(
            apply_placeholders("SELECT ${A}, ${AB}", &options, "x.sql").expect("substitute"),
            "SELECT 1, 2"
        );
        // 同一键出现多次全部替换
        assert_eq!(
            apply_placeholders("${A}${A}", &options, "x.sql").expect("substitute"),
            "11"
        );
    }

    /// 未配置占位符时**不做任何扫描**，含 `${` 的字面量原样保留。
    #[test]
    fn placeholder_substitution_is_off_when_unconfigured() {
        let options = MigratorOptions::default();
        let sql = "INSERT INTO t (body) VALUES ('%${not_a_placeholder}%')";
        assert_eq!(
            apply_placeholders(sql, &options, "x.sql").expect("no-op"),
            sql,
            "空占位符表必须原样返回，否则会误伤字符串字面量"
        );
    }

    /// 未知占位符与未闭合的 `${` 都显式报错，绝不静默保留。
    #[test]
    fn rejects_unknown_and_unterminated_placeholders() {
        let mut options = MigratorOptions::default();
        options.placeholders.insert("known".into(), "v".into());

        let err = apply_placeholders("SELECT ${unknown}", &options, "x.sql")
            .expect_err("未知占位符必须报错");
        assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
        let text = err.to_string();
        assert!(text.contains("unknown") && text.contains("known"), "{text}");

        let err =
            apply_placeholders("SELECT ${known", &options, "x.sql").expect_err("未闭合必须报错");
        assert!(matches!(err, DbError::Migration(_)), "got {err:?}");
        assert!(err.to_string().contains("unterminated"), "{err}");
    }

    /// 占位符值含换行会凭空造出指令行，必须在配置校验阶段拒绝。
    #[test]
    fn rejects_placeholder_values_with_line_breaks() {
        let mut options = MigratorOptions::default();
        options
            .placeholders
            .insert("evil".into(), "a\n-- +no-transaction".into());
        let err = validate_options(&options).expect_err("含换行的值必须报错");
        assert!(matches!(err, DbError::InvalidValue(_)), "got {err:?}");
        assert!(err.to_string().contains("line break"), "{err}");
    }
}

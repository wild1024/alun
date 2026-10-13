//! # alun-db-build
//!
//! build.rs 辅助库：解析迁移目录下的纯 SQL 迁移文件，合并出最终 schema 快照，
//! 供 `#[derive(Model, strict)]` 在编译期读取并校验模型字段。
//!
//! 遵循设计文档「迁移文件是真相来源」：快照完全由各迁移文件的
//! `-- +migrate Up` 段演化而来，不引入独立 DSL。
//!
//! ## 使用
//!
//! 在需要编译期 schema 校验的应用 crate 中，添加构建依赖后写 build.rs：
//!
//! ```ignore
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     alun_db_build::build_snapshot("migrations")?;
//!     Ok(())
//! }
//! ```
//!
//! 该函数会：
//! - 按文件名版本序读取 `migrations/` 下所有 `*.sql`；
//! - 抽取每段的 `-- +migrate Up` 内容，合并 `CREATE TABLE`、
//!   `ALTER TABLE ... ADD COLUMN` 与 `ALTER TABLE ... DROP COLUMN`
//!   演进出最终表列集；
//! - 将规范化 DDL 写入 `$OUT_DIR/schema_snapshot.sql`；
//! - 输出 `cargo:rustc-env=ALUN_DB_SCHEMA_SNAPSHOT` 与
//!   `cargo:rerun-if-changed`，使派生宏与增量构建均能感知。

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// 单个数据库列的快照：列名 + 原样保留的类型/约束片段。
struct ColumnDef {
    /// 列名（小写比较基准）。
    name: String,
    /// 列名之后保留的定义文本（类型、约束等，去尾部逗号）。
    rest: String,
}

/// 单张表演化后的快照：列的有序清单（首次出现顺序）。
#[derive(Default)]
struct TableSnap {
    /// 列定义，按首次声明顺序排列。
    columns: Vec<ColumnDef>,
}

impl TableSnap {
    /// 是否已包含指定列名（列名按 [`norm_column`] 归一后比较）。
    fn has_column(&self, name: &str) -> bool {
        let name = norm_column(name);
        self.columns.iter().any(|c| norm_column(&c.name) == name)
    }

    /// 追加一个尚未存在的列（幂等）。
    fn add_column(&mut self, col: ColumnDef) {
        if !self.has_column(&col.name) {
            self.columns.push(col);
        }
    }

    /// 删除指定列，返回是否命中。
    ///
    /// 幂等：列不存在（含该表整体没出现过）时静默忽略，与 [`Self::add_column`]
    /// 对重复列的忽略语义对称——这样 `DROP COLUMN IF EXISTS` 与方言目录下
    /// 未参与快照的加列都不会让构建失败。
    fn drop_column(&mut self, name: &str) -> bool {
        let name = norm_column(name);
        match self
            .columns
            .iter()
            .position(|c| norm_column(&c.name) == name)
        {
            Some(pos) => {
                self.columns.remove(pos);
                true
            }
            None => false,
        }
    }
}

/// 规整列名用于匹配：去首尾空白与标识符引号（`"col"` / `` `col` ``）。
/// 仅用于比较，渲染时仍保留迁移文件里的原始写法。
fn norm_column(name: &str) -> String {
    name.trim()
        .trim_matches(|c: char| c == '"' || c == '`')
        .to_string()
}

/// 生成 schema 快照并输出 cargo 构建指令。
///
/// # 参数
///
/// - `migrations_dir`：迁移文件目录，相对于调用方 crate 的
///   `CARGO_MANIFEST_DIR` 解析（如 `"migrations"`）。
///
/// # 行为
///
/// 扫描目录下所有 `*.sql` 文件，按文件名升序（即版本号序，约定文件名以
/// `YYYYMMDD_HHMMSS_描述.sql` 命名）逐个抽取 `-- +migrate Up` 段，合并
/// `CREATE TABLE` / `ALTER TABLE ... ADD COLUMN` / `ALTER TABLE ... DROP COLUMN`
/// （含 `IF EXISTS`、省略 `COLUMN`、逗号分隔的多动作形态）得到每张表的最终
/// 列集，渲染为规范化 DDL 写入 `$OUT_DIR/schema_snapshot.sql`，并输出：
///
/// 删除不存在的列（或对快照中未出现过的表删列）静默忽略：与重复加列的幂等
/// 语义对称，避免 `DROP COLUMN IF EXISTS` 或未参与快照的方言迁移中断构建。
///
/// - `cargo:rustc-env=ALUN_DB_SCHEMA_SNAPSHOT=<路径>`：供 `#[model(strict)]`
///   派生的宏通过 `std::env::var` 读取（宏与 build.rs 同属一个 cargo 目标）。
/// - `cargo:rerun-if-changed`：目录与每个迁移文件，保证迁移改动触发重建。
///
/// # Errors
///
/// `migrations_dir` 不存在、读取文件失败、`CARGO_MANIFEST_DIR` / `OUT_DIR`
/// 未设置或写快照失败时返回 [`io::Error`]。
pub fn build_snapshot(migrations_dir: &str) -> io::Result<()> {
    let manifest = env_unset("CARGO_MANIFEST_DIR")?;
    let dir = Path::new(&manifest).join(migrations_dir);
    if !dir.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("migrations dir not found: {}", dir.display()),
        ));
    }
    // 目录增删新迁移文件也应触发重建
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut files = collect_sql_files(&dir)?;
    files.sort();
    let mut tables: BTreeMap<String, TableSnap> = BTreeMap::new();
    for file in &files {
        println!("cargo:rerun-if-changed={}", file.display());
        let text = std::fs::read_to_string(file)?;
        apply_migration(&mut tables, &text);
    }

    let rendered = render_snapshot(&tables);
    let out_dir = env_unset("OUT_DIR")?;
    let dest = Path::new(&out_dir).join("schema_snapshot.sql");
    std::fs::write(&dest, rendered.as_bytes())?;
    println!("cargo:rustc-env=ALUN_DB_SCHEMA_SNAPSHOT={}", dest.display());
    Ok(())
}

/// 读取必填环境变量，缺失时转为 [`io::Error`]。
fn env_unset(name: &str) -> io::Result<String> {
    std::env::var(name)
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, format!("{name} unset: {e}")))
}

/// 收集目录下所有 `*.sql` 文件的路径。
fn collect_sql_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("sql") {
            out.push(path);
        }
    }
    Ok(out)
}

/// 将一个迁移的 `-- +migrate Up` 段合并进 schema。
fn apply_migration(tables: &mut BTreeMap<String, TableSnap>, sql: &str) {
    let up = extract_up(sql);
    for stmt in split_statements(up) {
        apply_statement(tables, &stmt);
    }
}

/// 应用单条语句：识别建表 / 加列 / 删列；其余（索引、外键、数据等）忽略。
fn apply_statement(tables: &mut BTreeMap<String, TableSnap>, stmt: &str) {
    if let Some((table, cols)) = parse_create_table(stmt) {
        let snap = tables.entry(table).or_default();
        for col in cols {
            snap.add_column(col);
        }
    } else if let Some((table, actions)) = parse_alter_actions(stmt) {
        for action in actions {
            match action {
                AlterAction::Add(col) => tables.entry(table.clone()).or_default().add_column(col),
                // 删列不碰没出现过的表：没有可删的列，也不该凭空造一张空表
                AlterAction::Drop(name) => {
                    if let Some(snap) = tables.get_mut(&table) {
                        snap.drop_column(&name);
                    }
                }
            }
        }
    }
    // 其它语句不影响列快照
}

/// 抽取 `-- +migrate Up` 与 `-- +migrate Down`（或文件末尾）之间的正向段；
/// 无标记时整体视为 Up 段。
fn extract_up(sql: &str) -> &str {
    const UP: &str = "-- +migrate Up";
    const DOWN: &str = "-- +migrate Down";
    let start = sql.find(UP).map(|i| i + UP.len()).unwrap_or(0);
    let rest = &sql[start..];
    let end = rest.find(DOWN).unwrap_or(rest.len());
    rest[..end].trim()
}

/// 按顶层分号切分多条 SQL（忽略字符串字面量内的分号）。
fn split_statements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_str = false;
    for ch in text.chars() {
        match ch {
            '\'' => {
                in_str = !in_str;
                cur.push(ch);
            }
            ';' if !in_str => {
                let seg = cur.trim();
                if !seg.is_empty() {
                    out.push(seg.to_string());
                }
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    let tail = cur.trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

/// 若 `s[i..]` 以 `kw` 开头（忽略大小写），返回其后字节偏移；否则 `None`。
fn skip_kw(s: &str, mut i: usize, kw: &str) -> Option<usize> {
    i = skip_spaces(s, i);
    let tail = s.as_bytes().get(i..)?;
    if tail.len() >= kw.len() && s.get(i..i + kw.len())?.eq_ignore_ascii_case(kw) {
        Some(i + kw.len())
    } else {
        None
    }
}

/// 从 `i` 前进，跳过空白字符。
fn skip_spaces(s: &str, i: usize) -> usize {
    s[i..]
        .char_indices()
        .find_map(|(off, ch)| {
            if ch.is_whitespace() {
                None
            } else {
                Some(i + off)
            }
        })
        .unwrap_or(s.len())
}

/// 规整表名：去首尾空白与引号，再去 schema 前缀，得到可用于匹配的简单名。
fn norm_table(name: &str) -> String {
    let trimmed = name.trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '`');
    trimmed.rsplit('.').next().unwrap_or("").to_string()
}

/// 取列段首 token 作为列名，其余文本原样保留（去尾部逗号）。
fn first_token(seg: &str) -> (String, String) {
    let mut it = seg.splitn(2, char::is_whitespace);
    let name = it.next().unwrap_or("").trim_end_matches(',');
    let rest = it.next().unwrap_or("").trim().trim_end_matches(',');
    (name.to_string(), rest.to_string())
}

/// 是否为行级约束段（不把它当列名）。
fn is_table_constraint(name: &str) -> bool {
    matches!(
        name,
        "primary" | "unique" | "foreign" | "check" | "constraint" | "key" | "index"
    )
}

/// 解析 `CREATE TABLE ...`，返回（规整表名，列定义列表）。
fn parse_create_table(stmt: &str) -> Option<(String, Vec<ColumnDef>)> {
    let s = stmt.trim_start();
    let mut i = skip_kw(s, 0, "create table")?;
    // 可选：IF NOT EXISTS
    if let Some(j) = skip_kw(s, i, "if not exists") {
        i = j;
    }
    i = skip_spaces(s, i);
    let open = s[i..].find('(')? + i;
    let name = norm_table(&s[i..open]);
    if name.is_empty() {
        return None;
    }
    let body = &s[open + 1..];
    let close = find_matching_paren(body);
    let mut cols = Vec::new();
    for seg in split_top_level(body, close) {
        let seg = seg.trim();
        if seg.is_empty() {
            continue;
        }
        let (name, rest) = first_token(seg);
        if name.is_empty() || is_table_constraint(&name.to_ascii_lowercase()) {
            continue;
        }
        cols.push(ColumnDef { name, rest });
    }
    Some((name, cols))
}

/// `ALTER TABLE` 单个子句对列集的影响。
enum AlterAction {
    /// `ADD [COLUMN] <coldef>`
    Add(ColumnDef),
    /// `DROP [COLUMN] [IF EXISTS] <name>`
    Drop(String),
}

/// 解析 `ALTER TABLE <t> <action>[, <action>...]`，返回（规整表名，动作列表）。
///
/// 支持加列（`ADD [COLUMN]`）与删列（`DROP [COLUMN] [IF EXISTS]`），含省略
/// `COLUMN` 关键字、逗号分隔的多动作形态；表名前缀 `IF EXISTS` / `ONLY` 也接受。
/// 不改变列集的子句（`ALTER COLUMN`、`RENAME`、`ADD/DROP CONSTRAINT|INDEX|
/// PRIMARY KEY|FOREIGN KEY|CHECK` 等行级约束）一律忽略；表名缺失返回 `None`。
fn parse_alter_actions(stmt: &str) -> Option<(String, Vec<AlterAction>)> {
    let s = stmt.trim_start();
    let mut i = skip_word(s, 0, "alter table")?;
    if let Some(j) = skip_word(s, i, "if exists") {
        i = j;
    }
    if let Some(j) = skip_word(s, i, "only") {
        i = j;
    }
    i = skip_spaces(s, i);
    let tail = &s[i..];
    let name_end = tail.find(char::is_whitespace).unwrap_or(tail.len());
    let table = norm_table(&tail[..name_end]);
    if table.is_empty() {
        return None;
    }
    // 子句之间以顶层逗号分隔（`DEFAULT (1, 2)` 这类括号/字面量内的逗号不算）
    let actions_text = &tail[name_end..];
    let actions = split_top_level(actions_text, actions_text.len())
        .iter()
        .filter_map(|clause| parse_alter_clause(clause.trim()))
        .collect();
    Some((table, actions))
}

/// 解析单个 `ALTER TABLE` 子句：`ADD ...` / `DROP ...`；其余（含行级约束的
/// 增删）返回 `None`。列名缺失或为约束关键字时同样忽略。
fn parse_alter_clause(clause: &str) -> Option<AlterAction> {
    if let Some(j) = skip_word(clause, 0, "add") {
        let mut j = j;
        if let Some(k) = skip_word(clause, j, "column") {
            j = k;
        }
        let def = clause[j..].trim();
        let (name, rest) = first_token(def);
        if name.is_empty() || is_table_constraint(&name.to_ascii_lowercase()) {
            return None;
        }
        return Some(AlterAction::Add(ColumnDef { name, rest }));
    }
    if let Some(j) = skip_word(clause, 0, "drop") {
        let mut j = j;
        if let Some(k) = skip_word(clause, j, "column") {
            j = k;
        }
        if let Some(k) = skip_word(clause, j, "if exists") {
            j = k;
        }
        // 列名之后可能还有 `CASCADE` / `RESTRICT` 等修饰，只取首 token
        let name = clause[j..].split_whitespace().next()?;
        let name = name.trim_end_matches(',');
        if name.is_empty() || is_table_constraint(&name.to_ascii_lowercase()) {
            return None;
        }
        return Some(AlterAction::Drop(name.to_string()));
    }
    None
}

/// 与 [`skip_kw`] 相同，但要求关键字之后是词边界（空白或行尾），
/// 避免 `ADD` 匹配到 `address`、`DROP` 匹配到 `dropped` 之类的误判。
fn skip_word(s: &str, i: usize, kw: &str) -> Option<usize> {
    let j = skip_kw(s, i, kw)?;
    match s[j..].chars().next() {
        None => Some(j),
        Some(c) if c.is_whitespace() => Some(j),
        Some(_) => None,
    }
}

/// 在 `body` 中找到与开头 '(' 配对的 ')' 偏移（忽略字符串字面量）。
fn find_matching_paren(body: &str) -> usize {
    let mut depth = 1;
    let mut in_str = false;
    for (i, ch) in body.char_indices() {
        match ch {
            '\'' => in_str = !in_str,
            '(' if !in_str => depth += 1,
            ')' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    body.len()
}

/// 按顶层逗号切分（忽略字符串字面量与嵌套括号内的逗号）。
fn split_top_level(body: &str, close: usize) -> Vec<String> {
    let chunk = &body[..close];
    let mut segs = Vec::new();
    let mut depth = 0;
    let mut in_str = false;
    let mut start = 0;
    for (i, ch) in chunk.char_indices() {
        match ch {
            '\'' => {
                in_str = !in_str;
                // 引号本身需保留在列定义文本中
            }
            '(' if !in_str => depth += 1,
            ')' if !in_str => depth -= 1,
            ',' if !in_str && depth == 0 => {
                segs.push(chunk[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    segs.push(chunk[start..].to_string());
    segs
}

/// 将快照渲染为规范化 DDL（可被宏的列解析器直接消费）。
fn render_snapshot(tables: &BTreeMap<String, TableSnap>) -> String {
    let mut out = String::from("-- generated by alun-db-build (schema snapshot)\n");
    for (table, snap) in tables {
        let body = snap
            .columns
            .iter()
            .map(|c| {
                if c.rest.is_empty() {
                    format!("    {}", c.name)
                } else {
                    format!("    {} {}", c.name, c.rest)
                }
            })
            .collect::<Vec<_>>()
            .join(",\n");
        out.push_str(&format!("CREATE TABLE {table} (\n{body}\n);\n\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_up_slices_between_markers() {
        let sql = "-- +migrate Up\nCREATE TABLE t (id INT);\n-- +migrate Down\nDROP TABLE t;";
        let up = extract_up(sql);
        assert!(up.contains("CREATE TABLE t"));
        assert!(!up.contains("DROP TABLE"));
    }

    #[test]
    fn split_statements_ignores_semicolons_in_strings() {
        let sql = "INSERT INTO t (v) VALUES ('a;b'); UPDATE t SET v = 'x';";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].contains("'a;b'"));
    }

    #[test]
    fn parse_create_table_tracks_columns_and_nesting() {
        let sql = "CREATE TABLE users (\n  id bigint not null,\n  email text,\n  meta json default ('{}'),\n  CONSTRAINT uq UNIQUE (email)\n);";
        let (table, cols) = parse_create_table(sql).unwrap();
        assert_eq!(table, "users");
        assert_eq!(cols.len(), 3, "CONSTRAINT 行不应算作列");
        assert_eq!(cols[0].name, "id");
        assert!(cols[0].rest.contains("not null"));
        assert_eq!(cols[2].name, "meta");
    }

    /// 取出唯一一个 ADD 动作的列定义。
    fn single_add(stmt: &str) -> (String, ColumnDef) {
        let (table, actions) = parse_alter_actions(stmt).unwrap();
        assert_eq!(actions.len(), 1, "期望单个动作: {stmt}");
        match actions.into_iter().next().unwrap() {
            AlterAction::Add(col) => (table, col),
            AlterAction::Drop(_) => panic!("期望 ADD 动作: {stmt}"),
        }
    }

    #[test]
    fn parse_alter_actions_handles_without_column_keyword_and_schema_prefix() {
        let (table, col) = single_add("ALTER TABLE public.users ADD age int not null");
        assert_eq!(table, "users");
        assert_eq!(col.name, "age");
        assert!(col.rest.contains("not null"));
    }

    #[test]
    fn parse_alter_actions_detects_add_inside_table_name() {
        // 表名含 "address"，ADD 关键字应在其后，且不应被 `address` 前缀误匹配
        let (table, col) = single_add("ALTER TABLE address_book ADD COLUMN phone text");
        assert_eq!(table, "address_book");
        assert_eq!(col.name, "phone");
    }

    #[test]
    fn parse_alter_actions_splits_multiple_clauses_by_top_level_comma() {
        let (table, actions) = parse_alter_actions(
            "ALTER TABLE users ADD COLUMN a int, DROP COLUMN b, ADD COLUMN c text DEFAULT ('x,y')",
        )
        .unwrap();
        assert_eq!(table, "users");
        assert_eq!(actions.len(), 3, "括号/字面量内的逗号不应切分子句");
        match &actions[..] {
            [
                AlterAction::Add(a),
                AlterAction::Drop(b),
                AlterAction::Add(c),
            ] => {
                assert_eq!(a.name, "a");
                assert_eq!(b, "b");
                assert_eq!(c.name, "c");
                assert!(c.rest.contains("('x,y')"));
            }
            _ => panic!("动作序列不符预期"),
        }
    }

    #[test]
    fn parse_alter_actions_ignores_non_column_clauses() {
        // 行级约束的增删、重命名、`ALTER COLUMN ... DROP DEFAULT` 都不改变列集
        for stmt in [
            "ALTER TABLE users ADD CONSTRAINT uq UNIQUE (email)",
            "ALTER TABLE users DROP CONSTRAINT uq",
            "ALTER TABLE users DROP INDEX idx_users_email",
            "ALTER TABLE users RENAME TO people",
            "ALTER TABLE users ALTER COLUMN email DROP DEFAULT",
        ] {
            let (_, actions) = parse_alter_actions(stmt).unwrap();
            assert!(actions.is_empty(), "不应产生列动作: {stmt}");
        }
    }

    #[test]
    fn drop_column_removes_column_from_snapshot() {
        let mut tables: BTreeMap<String, TableSnap> = BTreeMap::new();
        apply_migration(
            &mut tables,
            "-- +migrate Up\nCREATE TABLE users (id bigint, email text);",
        );
        apply_migration(
            &mut tables,
            "-- +migrate Up\nALTER TABLE users ADD COLUMN legacy_id varchar(64);",
        );
        apply_migration(
            &mut tables,
            "-- +migrate Up\nALTER TABLE users DROP COLUMN legacy_id;\n-- +migrate Down\nALTER TABLE users ADD COLUMN legacy_id varchar(64);",
        );
        let snap = tables.get("users").unwrap();
        assert!(!snap.has_column("legacy_id"), "已删列不应残留在快照中");
        assert_eq!(snap.columns.len(), 2);
        let ddl = render_snapshot(&tables);
        assert!(
            !ddl.contains("legacy_id"),
            "渲染出的快照不应含已删列：\n{ddl}"
        );
    }

    #[test]
    fn drop_column_if_exists_and_quoted_identifier() {
        let mut tables: BTreeMap<String, TableSnap> = BTreeMap::new();
        apply_migration(
            &mut tables,
            "-- +migrate Up\nCREATE TABLE users (id bigint, email text, legacy_id varchar(64));",
        );
        apply_migration(
            &mut tables,
            "-- +migrate Up\nALTER TABLE users DROP COLUMN IF EXISTS \"legacy_id\";",
        );
        let snap = tables.get("users").unwrap();
        assert!(!snap.has_column("legacy_id"));
        assert_eq!(snap.columns.len(), 2);

        // 再次 IF EXISTS 同一列：无命中，快照保持不变
        apply_migration(
            &mut tables,
            "-- +migrate Up\nALTER TABLE users DROP COLUMN IF EXISTS legacy_id;",
        );
        assert_eq!(tables.get("users").unwrap().columns.len(), 2);
    }

    #[test]
    fn drop_missing_column_is_ignored_and_creates_no_table() {
        let mut tables: BTreeMap<String, TableSnap> = BTreeMap::new();
        apply_migration(
            &mut tables,
            "-- +migrate Up\nCREATE TABLE users (id bigint, email text);",
        );
        apply_migration(
            &mut tables,
            "-- +migrate Up\nALTER TABLE users DROP COLUMN nope;\nALTER TABLE ghosts DROP COLUMN id;",
        );
        assert_eq!(tables.get("users").unwrap().columns.len(), 2, "未知列忽略");
        assert!(
            !tables.contains_key("ghosts"),
            "对未出现过的表删列不应凭空造表"
        );
    }

    #[test]
    fn apply_migration_applies_mixed_create_add_drop_sequence() {
        let mut tables: BTreeMap<String, TableSnap> = BTreeMap::new();
        apply_migration(
            &mut tables,
            "-- +migrate Up\n\
             CREATE TABLE users (id bigint, email text);\n\
             ALTER TABLE users ADD COLUMN nick text;\n\
             ALTER TABLE users ADD COLUMN legacy_id varchar(64);\n\
             ALTER TABLE users DROP COLUMN email;\n\
             ALTER TABLE users DROP COLUMN IF EXISTS legacy_id;\n\
             CREATE TABLE invoices (id bigint, total numeric(38,10));\n\
             -- +migrate Down\n\
             DROP TABLE users;",
        );
        let users = tables.get("users").unwrap();
        assert_eq!(
            users
                .columns
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["id", "nick"],
            "顺序应为首次声明序，且已删列被移除"
        );
        assert_eq!(tables.get("invoices").unwrap().columns.len(), 2);
        assert!(
            tables.contains_key("users"),
            "Down 段（DROP TABLE）不应被应用"
        );
    }

    #[test]
    fn apply_migration_merges_create_then_alter() {
        let mut tables: BTreeMap<String, TableSnap> = BTreeMap::new();
        apply_migration(
            &mut tables,
            "-- +migrate Up\nCREATE TABLE users (id int, name text);\n-- +migrate Down\nDROP TABLE users;",
        );
        apply_migration(
            &mut tables,
            "-- +migrate Up\nALTER TABLE users ADD COLUMN age int;\nCREATE INDEX idx_users_age ON users(age);\n-- +migrate Down\nDROP INDEX idx_users_age;",
        );
        let snap = tables.get("users").unwrap();
        assert_eq!(snap.columns.len(), 3, "CREATE + ALTER 应合并且幂等");
        assert_eq!(snap.columns[2].name, "age");
    }

    #[test]
    fn render_snapshot_produces_parseable_ddl() {
        let mut tables = BTreeMap::new();
        tables.insert(
            "users".to_string(),
            TableSnap {
                columns: vec![
                    ColumnDef {
                        name: "id".into(),
                        rest: "bigint not null".into(),
                    },
                    ColumnDef {
                        name: "name".into(),
                        rest: "text".into(),
                    },
                ],
            },
        );
        let ddl = render_snapshot(&tables);
        assert!(ddl.contains("CREATE TABLE users ("));
        assert!(ddl.contains("id bigint not null"));
    }
}

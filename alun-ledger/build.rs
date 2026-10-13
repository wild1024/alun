//! 迁移目录 → schema 快照的桥接（见《测试项目测试规划.md》§3.2）。
//!
//! runner 侧（`Migrator`）要求文件名编码版本与方向，且**允许单文件多语句**；
//! 而 `alun_db_build::build_snapshot` 只吃「含 `-- +migrate Up` 段的多语句文件」且
//! **不递归子目录**。因此这里把 `migrations/common/V*.up.sql` 按版本号合并为单份
//! 快照文件，再交给 `build_snapshot` 生成 `$OUT_DIR/schema_snapshot.sql` 并导出
//! `ALUN_DB_SCHEMA_SNAPSHOT`，供 `#[model(strict)]` 在编译期读取。
//!
//! 注意：方言目录（postgres/mysql/sqlite）不参与快照——它们只含索引 / 视图 /
//! 扩展等 DDL，快照解析器本身也只识别 `CREATE TABLE` 与 `ALTER TABLE ADD COLUMN`。

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=migrations/common");

    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");

    // 快照包含：方言基础 DDL（建表，含金额/时间戳的原生类型）+ common 演进列。
    // 方言可用 `ALUN_LEDGER_SNAPSHOT_DIALECT` 覆盖（默认 sqlite，即开发/测试方言）。
    let dialect = std::env::var("ALUN_LEDGER_SNAPSHOT_DIALECT").unwrap_or_else(|_| "sqlite".into());
    println!("cargo:rerun-if-env-changed=ALUN_LEDGER_SNAPSHOT_DIALECT");
    println!("cargo:rerun-if-changed=migrations/{dialect}");

    // 顺序语义：先建表（01-base），再应用 common 演进列 → 快照列序与真实库一致
    let mut base: Vec<(i64, PathBuf)> = Vec::new();
    let mut evolve: Vec<(i64, PathBuf)> = Vec::new();
    collect_up_files(
        &Path::new(&manifest)
            .join("migrations")
            .join(&dialect)
            .join("01-base"),
        &mut base,
    );
    collect_up_files(&Path::new(&manifest).join("migrations/common"), &mut evolve);
    base.sort();
    evolve.sort();
    let mut entries = base;
    entries.extend(evolve);

    let mut merged = String::from("-- +migrate Up\n");
    for (_, path) in &entries {
        let sql = fs::read_to_string(path).expect("read migration file");
        merged.push_str(sql.trim());
        merged.push_str(";\n");
    }

    let gen_dir = Path::new(&out_dir).join("schema_gen");
    fs::create_dir_all(&gen_dir).expect("create schema_gen");
    let snapshot_src = gen_dir.join("0000_snapshot.sql");
    fs::write(&snapshot_src, merged).expect("write merged snapshot source");

    // 绝对路径：Path::join 遇绝对路径会替换 CARGO_MANIFEST_DIR 前缀
    alun_db_build::build_snapshot(&gen_dir.to_string_lossy()).expect("build_snapshot");
}

/// 收集目录下的 `V{n}__*.up.sql`（不递归，与 runner/build_snapshot 语义一致）。
fn collect_up_files(dir: &Path, out: &mut Vec<(i64, PathBuf)>) {
    let read_dir = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in read_dir {
        let path = entry.expect("dir entry").path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        println!("cargo:rerun-if-changed={}", path.display());
        if !name.ends_with(".up.sql") {
            continue;
        }
        let version: i64 = name
            .trim_start_matches('V')
            .split("__")
            .next()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("bad migration filename: {name}"));
        out.push((version, path));
    }
}

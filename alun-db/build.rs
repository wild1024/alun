//! 为 `#[model(strict)]`（未显式 `schema`）编译期校验收取 build.rs 快照。
//!
//! 快照由 `tests/migrations/` 下的纯 SQL 迁移文件合并而来（见
//! `alun-db-build` crate）。仅当这些文件变化时触发重建。

fn main() -> Result<(), Box<dyn std::error::Error>> {
    alun_db_build::build_snapshot("tests/migrations")?;
    Ok(())
}

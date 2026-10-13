//! 测试公共设施（harness / 夹具 / 断言 / 故障注入）。
//!
//! 库选择：`ALUN_LEDGER_DB`（默认 `sqlite-mem`）：
//! `sqlite-mem` / `sqlite-file` / `postgres` / `mysql`。
//! `ALUN_LEDGER_MULTIDB_STRICT=1` 下连不上直接 panic（CI 必设），
//! 否则打印原因并跳过用例（本地无容器时不红）。

#![allow(dead_code)]

/// `sqlite-file` 模式使用的文件库路径（**按测试二进制隔离**）。
///
/// `cargo test` 为每个 `tests/*.rs` 起一个进程；若都指向同一个文件，
/// 并行执行时会互相等 SQLite 写锁（`SQLITE_BUSY` 重试），把并行回归拖成分钟级。
/// 这里用「测试二进制名（去掉 cargo 的哈希后缀）」区分，进程内仍共享同一个文件
/// （`harness::SHARED` 进程内串行锁继续生效）。
pub fn sqlite_file_path() -> String {
    let tag = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "ledger_test".into());
    let tag = tag.split('-').next().unwrap_or("ledger_test");
    // 文件库路径相对**包根目录**（cargo 以包根为测试进程 CWD）；
    // 包内 `target/` 不保证存在 → 主动创建。
    std::fs::create_dir_all("target").expect("create package target dir");
    format!("./target/ledger_test_{tag}.db")
}

pub mod assertions;
pub mod e2e;
pub mod faults;
pub mod fixtures;
pub mod harness;

//! L3 E2E（SQLite 文件库）：完整中间件链 + REST 全流程。
//!
//! ```bash
//! cargo test -p alun-ledger --test e2e_sqlite -- --test-threads=1 --nocapture
//! ```

mod common;

use common::e2e::run_all;
use common::harness::Dialect;

#[tokio::test(flavor = "current_thread")]
async fn e2e_sqlite_main_flows() {
    let ok = run_all(Dialect::SqliteFile).await;
    assert!(ok, "见上方 E2E 结果表（含 ❌ 项对应的现状缺陷）");
}

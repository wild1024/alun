//! L3 E2E（MySQL）。
//!
//! ```bash
//! ALUN_LEDGER_DB=mysql cargo test -p alun-ledger --features mysql --test e2e_mysql -- --test-threads=1 --nocapture
//! ```

#![cfg(feature = "mysql")]

mod common;

use common::e2e::run_all;
use common::harness::Dialect;

#[tokio::test(flavor = "current_thread")]
async fn e2e_mysql_main_flows() {
    let ok = run_all(Dialect::Mysql).await;
    assert!(ok, "见上方 E2E 结果表（含 ❌ 项对应的现状缺陷）");
}

//! L3 E2E（PostgreSQL）。
//!
//! ```bash
//! ALUN_LEDGER_DB=postgres cargo test -p alun-ledger --features postgres --test e2e_pg -- --test-threads=1 --nocapture
//! ```

#![cfg(feature = "postgres")]

mod common;

use common::e2e::run_all;
use common::harness::Dialect;

#[tokio::test(flavor = "current_thread")]
async fn e2e_pg_main_flows() {
    let ok = run_all(Dialect::Postgres).await;
    assert!(ok, "见上方 E2E 结果表（含 ❌ 项对应的现状缺陷）");
}

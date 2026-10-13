//! Alun Ledger 可执行入口。
//!
//! 配置驱动启动：读取 `config/config.toml`（可用 `ALUN_PROFILE=test` 叠加
//! `config/config-test.toml`），建连数据库并装配中间件链。
//!
//! ```bash
//! cargo run -p alun-ledger            # 默认 profile
//! ALUN_PROFILE=test cargo run -p alun-ledger
//! ```

use alun::App;

#[tokio::main]
async fn main() {
    let app = match App::new() {
        Ok(app) => app,
        Err(e) => {
            eprintln!("应用初始化失败: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = app.scan().start().await {
        eprintln!("服务运行失败: {e}");
        std::process::exit(1);
    }
}

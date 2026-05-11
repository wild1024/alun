//! 示例 01：最小化启动
//!
//! 启动一个 Web 服务，配置完全由 config.toml 驱动。

use alun::{App, Res};
use alun::prelude::*;

#[alun::get("/")]
async fn hello() -> Res<String> {
    Res::ok("Hello Alun!".into())
}

#[alun::get("/info")]
async fn info() -> Res<serde_json::Value> {
    Res::ok(serde_json::json!({ "profile": cfg().profile, "listen": cfg().server.listen }))
}

#[tokio::main]
async fn main() {
    App::new()
        .expect("初始化失败")
        .scan()
        .start()
        .await
        .unwrap();
}

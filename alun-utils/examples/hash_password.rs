//! 生成口令的 Argon2 哈希（与 [`alun_utils::Crypto::hash_password`] 完全同参，含随机盐）。
//!
//! 用法：`cargo run -p alun-utils --example hash_password -- <口令>`
//!
//! 典型用途：为内置初始口令**预生成**哈希常量（例如业务侧的管理员默认口令），
//! 避免首次启动在现场执行 Argon2 慢哈希（本机 debug 构建实测约 420ms）。
//! 注意：预生成常量仅适用于**公开的默认口令**；用户自定义口令仍应现场哈希。

fn main() {
    let password = match std::env::args().nth(1) {
        Some(value) => value,
        None => {
            eprintln!("用法: cargo run -p alun-utils --example hash_password -- <口令>");
            std::process::exit(2);
        }
    };
    match alun_utils::Crypto::hash_password(&password) {
        Ok(hash) => println!("{hash}"),
        Err(e) => {
            eprintln!("哈希失败: {e}");
            std::process::exit(1);
        }
    }
}

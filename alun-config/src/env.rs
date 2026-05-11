//! 环境检测、命令行参数解析与环境变量覆盖

use crate::AppConfig;
use std::env;

/// 检测当前 profile：命令行参数 > 环境变量 > "dev"
pub fn detect_profile() -> String {
    for arg in env::args() {
        if let Some(profile) = arg.strip_prefix("profile=") {
            return profile.to_string();
        }
    }

    env::var("ALUN_PROFILE")
        .or_else(|_| env::var("ALUN_ENV"))
        .unwrap_or_else(|_| "dev".into())
}

/// 解析命令行参数
///
/// 返回 `(should_gen_config, should_print_config)`
///
/// ```text
/// cargo run -- gen-config
/// cargo run -- generate-config
/// cargo run -- print-config
/// cargo run -- profile=prod
/// ```
pub fn parse_args() -> (bool, bool) {
    let mut gen_config = false;
    let mut print_config = false;

    for arg in env::args() {
        if arg == "gen-config" || arg == "generate-config" {
            gen_config = true;
        }
        if arg == "print-config" {
            print_config = true;
        }
    }

    (gen_config, print_config)
}

/// 环境变量覆盖配置
///
/// 约定：`ALUN_` 前缀的大写字段路径映射
///   - `ALUN_SERVER_LISTEN=3000`
///   - `ALUN_DATABASE_NAME=mydb`
///   - `ALUN_LOG_LEVEL=debug`
pub fn merge_env_overrides(cfg: &mut AppConfig) {
    for (key, value) in env::vars() {
        if !key.starts_with("ALUN_") || value.is_empty() {
            continue;
        }

        let path = key[5..].to_lowercase();
        let parts: Vec<&str> = path.split('_').collect();

        match parts.as_slice() {
            ["server", "listen"] => cfg.server.listen = value,
            ["log", "level"] => cfg.log.level = value,
            ["log", "format"] => cfg.log.format = value,
            ["log", "dir"] => cfg.log.dir = Some(value),
            ["database", "host"] => cfg.database.host = value,
            ["database", "name"] => cfg.database.name = value,
            ["database", "user"] => cfg.database.user = value,
            ["database", "password"] => cfg.database.password = value,
            ["redis", "url"] => cfg.redis.url = value,
            ["cache", "type"] => cfg.cache.r#type = value,
            ["router", "prefix"] => cfg.router.prefix = value,
            ["upload", "path"] => cfg.upload.path = value,
            ["download", "path"] => cfg.download.path = value,
            ["template", "path"] => cfg.template.path = value,
            _ => {}
        }
    }
}

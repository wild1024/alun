//! 数据库连接配置构造。
//!
//! 将 `alun-config` 的字段式 [`DatabaseConfig`](alun_config::DatabaseConfig)
//! 映射为新版 `alun-db` 的 [`DatabaseConfig`](alun_db::DatabaseConfig)。
//!
//! 兼容原有框架的配置做法：应用侧依然只写 `type/host/port/name/user/
//! password/ssl_mode/max_connections/...` 字段，由本模块统一转换为新版
//! 的 `url + pool` 形式，并处理加密密码解密。
//!
//! 依赖方向说明：新版 `alun-db` 自闭、不依赖 `alun-config`，因此本转换
//! 收敛在 `alun-web`（同时依赖 `alun-config` 与 `alun-utils`），避免把
//! 配置层耦合进数据库层。

use std::time::Duration;

use alun_config::DatabaseConfig as SrcConfig;
use alun_db::{DatabaseConfig as DbConfig, PoolConfig};
use alun_utils::crypto::Crypto;

/// 将字段式旧配置映射为新版 `DatabaseConfig`。
///
/// 根据 `DatabaseConfig.r#type` 生成连接 URL（含 `ssl_mode` 查询参数），
/// 将 `max_connections/min_connections/connect_timeout` 映射为池配置，
/// 并处理加密密码的解密。
///
/// # 密码处理
///
/// 当 `password_encrypted` 为真时，使用 `ALUN_SECRET_KEY`（32 字节 hex）
/// 解密 `base64(nonce):base64(ciphertext)` 形式的密文密码。
///
/// # 错误
///
/// 秘钥缺失/长度非法、解密失败或数据库类型不受支持时返回 `String`。
pub fn build_config(src: &SrcConfig) -> Result<DbConfig, String> {
    let password = resolve_password(src)?;
    let url = build_url(src, &password)?;

    Ok(DbConfig {
        db_type: normalize_db_type(&src.r#type)?,
        url,
        password: if password.is_empty() {
            None
        } else {
            Some(password)
        },
        pool: PoolConfig {
            max_connections: src.max_connections,
            min_connections: src.min_connections,
            connect_timeout: Duration::from_secs(src.connect_timeout),
            idle_timeout: None,
            max_lifetime: None,
        },
    })
}

/// 构造连接 URL。
fn build_url(config: &SrcConfig, password: &str) -> Result<String, String> {
    let port = config.port.unwrap_or_else(|| default_port(&config.r#type));

    match config.r#type.as_str() {
        "postgres" | "postgresql" => {
            let base = format!(
                "postgres://{}:{}@{}:{}/{}",
                config.user, password, config.host, port, config.name
            );
            if config.ssl_mode.is_empty() {
                Ok(base)
            } else {
                Ok(format!("{}?sslmode={}", base, config.ssl_mode))
            }
        }
        "mysql" => Ok(format!(
            "mysql://{}:{}@{}:{}/{}",
            config.user, password, config.host, port, config.name
        )),
        "sqlite" => Ok(format!("sqlite://./{}", config.name)),
        other => Err(format!("不支持的数据库类型: {}", other)),
    }
}

/// 将配置中的类型归一化为新版枚举取值。
fn normalize_db_type(db_type: &str) -> Result<String, String> {
    match db_type {
        "postgresql" | "postgres" => Ok("postgres".into()),
        "mysql" => Ok("mysql".into()),
        "sqlite" => Ok("sqlite".into()),
        other => Err(format!("不支持的数据库类型: {}", other)),
    }
}

/// 解析最终明文密码（必要时解密）。
fn resolve_password(config: &SrcConfig) -> Result<String, String> {
    if config.password_encrypted && !config.password.is_empty() {
        let key = secret_key()?;
        let parts: Vec<&str> = config.password.split(':').collect();
        if parts.len() != 2 {
            return Err("加密密码格式错误，期望 base64_iv:base64_ciphertext".into());
        }
        Crypto::aes_decrypt(&key, parts[1], parts[0]).ok_or_else(|| "密码解密失败".to_string())
    } else {
        Ok(config.password.clone())
    }
}

/// 读取 32 字节 AES-GCM 秘钥（`ALUN_SECRET_KEY`，hex 编码）。
fn secret_key() -> Result<Vec<u8>, String> {
    let raw =
        std::env::var("ALUN_SECRET_KEY").map_err(|_| "缺少环境变量 ALUN_SECRET_KEY".to_string())?;
    let key = hex::decode(&raw).map_err(|e| format!("ALUN_SECRET_KEY 不是合法 hex: {}", e))?;
    if key.len() != 32 {
        return Err(format!(
            "ALUN_SECRET_KEY 长度必须为 32 字节，实际 {}",
            key.len()
        ));
    }
    Ok(key)
}

/// 各数据库类型的默认端口。
fn default_port(db_type: &str) -> u16 {
    match db_type {
        "postgres" | "postgresql" => 5432,
        "mysql" => 3306,
        _ => 0,
    }
}

//! 数据库工厂：从 DatabaseConfig 创建连接池
//!
//! - 支持 PostgreSQL / MySQL / SQLite
//! - 支持加密密码（base64(AES-GCM) 解密）
//! - 自动连接测试
//! - 连接池参数配置

use crate::{Db, DbResult, DbError};
use alun_config::DatabaseConfig;
use tracing::info;

/// 从配置创建 Db 实例 + 连接测试
///
/// 根据 `DatabaseConfig.type` 自动选择 PostgreSQL/MySQL/SQLite 后端，
/// 支持加密密码自动解密，创建后立即执行 `SELECT 1` 连接测试。
pub async fn create_db(config: &DatabaseConfig) -> DbResult<Db> {
    if !config.enabled {
        return Err(DbError::Other("数据库未启用".into()));
    }

    let password = if config.password_encrypted && !config.password.is_empty() {
        decrypt_password(&config.password)?
    } else {
        config.password.clone()
    };

    let url = build_connection_url(config, &password);

    info!("数据库连接: {}:{}@{}:{}/{}",
        config.user, "***", config.host,
        config.port.unwrap_or(default_port(&config.r#type)),
        config.name,
    );

    match config.r#type.as_str() {
        "postgres" | "postgresql" => {
            let pool = sqlx::postgres::PgPoolOptions::new()
                .max_connections(config.max_connections)
                .min_connections(config.min_connections)
                .connect(&url).await?;
            test_connection_pg(&pool).await?;
            Ok(Db::postgres(pool))
        }
        "mysql" => {
            let pool = sqlx::mysql::MySqlPoolOptions::new()
                .max_connections(config.max_connections)
                .min_connections(config.min_connections)
                .connect(&url).await?;
            test_connection_mysql(&pool).await?;
            Ok(Db::mysql(pool))
        }
        "sqlite" => {
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(config.max_connections)
                .connect(&url).await?;
            test_connection_sqlite(&pool).await?;
            Ok(Db::sqlite(pool))
        }
        other => Err(DbError::Argument(format!("不支持的数据库类型: {}", other))),
    }
}

/// 仅当数据库启用时创建，否则返回 None
pub async fn create_db_if_enabled(config: &DatabaseConfig) -> Option<DbResult<Db>> {
    if !config.enabled { return None; }
    Some(create_db(config).await)
}

fn default_port(db_type: &str) -> u16 {
    match db_type {
        "postgres" | "postgresql" => 5432,
        "mysql" => 3306,
        _ => 0,
    }
}

async fn test_connection_pg(pool: &sqlx::PgPool) -> DbResult<()> {
    let row: (i32,) = sqlx::query_as("SELECT 1").fetch_one(pool).await?;
    info!("PostgreSQL 连接测试通过: result={}", row.0);
    Ok(())
}

async fn test_connection_mysql(pool: &sqlx::MySqlPool) -> DbResult<()> {
    let row: (i32,) = sqlx::query_as("SELECT 1").fetch_one(pool).await?;
    info!("MySQL 连接测试通过: result={}", row.0);
    Ok(())
}

async fn test_connection_sqlite(pool: &sqlx::SqlitePool) -> DbResult<()> {
    let row: (i32,) = sqlx::query_as("SELECT 1").fetch_one(pool).await?;
    info!("SQLite 连接测试通过: result={}", row.0);
    Ok(())
}

fn build_connection_url(config: &DatabaseConfig, password: &str) -> String {
    let port = config.port.unwrap_or(default_port(&config.r#type));

    match config.r#type.as_str() {
        "postgres" | "postgresql" => format!(
            "postgres://{}:{}@{}:{}/{}",
            config.user, password, config.host, port, config.name
        ),
        "mysql" => format!(
            "mysql://{}:{}@{}:{}/{}",
            config.user, password, config.host, port, config.name
        ),
        "sqlite" => format!("sqlite:{}", config.name),
        _ => format!("postgres://{}:{}@{}:{}/{}", config.user, password, config.host, port, config.name),
    }
}

fn decrypt_password(encrypted: &str) -> DbResult<String> {
    let key = get_secret_key();
    let parts: Vec<&str> = encrypted.split(':').collect();
    if parts.len() != 2 {
        return Err(DbError::Argument("加密密码格式错误，期望 base64_iv:base64_ciphertext".into()));
    }
    alun_utils::crypto::Crypto::aes_decrypt(&key, parts[1], parts[0])
        .ok_or_else(|| DbError::Argument("密码解密失败".into()))
}

fn get_secret_key() -> Vec<u8> {
    std::env::var("ALUN_SECRET_KEY")
        .ok()
        .and_then(|s| hex::decode(&s).ok())
        .filter(|k| k.len() == 32)
        .unwrap_or_else(|| vec![0u8; 32])
}

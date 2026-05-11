/// Alun 快速入门示例 —— 配置驱动 + 全功能演示
///
/// 启动方式：
/// ```
/// cargo run --example quick-start                     # 启动服务
/// cargo run --example quick-start -- gen-config       # 生成默认 config/config.toml
/// cargo run --example quick-start -- print-config     # 打印配置
/// cargo run --example quick-start -- profile=prod     # 指定 profile
/// ALUN_LOG_LEVEL=debug cargo run --example quick-start # 环境变量覆盖
/// ```

use alun::prelude::*;
use alun_utils::StrExt;

// ──── 数据模型 ────────────────────────────────────

// 实际项目中按需定义业务模型

// ──── Handlers ────────────────────────────────────

async fn index() -> &'static str { "🚀 Alun Framework v0.1.0" }

async fn health() -> Res<serde_json::Value> {
    Res::ok(serde_json::json!({
        "status": "ok",
        "framework": "alun",
        "features": ["config", "log", "middleware", "plugins", "utils"]
    }))
}

/// 演示 String -> Snake/Camel 互转
async fn string_demo(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Res<serde_json::Value> {
    let input = params.get("input").cloned().unwrap_or_else(|| "hello_alun".into());
    let camel = input.to_camel();
    let snake = input.to_snake();
    let masked = alun_utils::Mask::mobile(&input);

    Res::ok(serde_json::json!({
        "input": input,
        "to_camel": camel,
        "to_snake": snake,
        "masked": masked,
    }))
}

/// 短 ID 生成
async fn generate_ids() -> Res<serde_json::Value> {
    Res::ok(serde_json::json!({
        "sid_short": alun_utils::Sid::short(),
        "sid_tiny": alun_utils::Sid::tiny(),
        "sid_tsid": alun_utils::Sid::tsid(),
        "uuid4": alun_utils::Sid::uuid(),
        "uuid7": alun_utils::Sid::uuid7(),
    }))
}

/// 日期工具演示
async fn date_demo() -> Res<serde_json::Value> {
    let now = alun_utils::Date::now();
    let ts = now.timestamp();

    Res::ok(serde_json::json!({
        "now": alun_utils::Date::fmt(&now, "%Y-%m-%d %H:%M:%S"),
        "relative": alun_utils::Date::relative(ts),
        "begin_of_day": alun_utils::Date::fmt(&alun_utils::Date::begin_of_day(&now), "%Y-%m-%dT%H:%M:%SZ"),
    }))
}

/// 脱敏演示
async fn mask_demo() -> Res<serde_json::Value> {
    Res::ok(serde_json::json!({
        "mobile": { "original": "13812345678", "masked": alun_utils::Mask::mobile("13812345678") },
        "email":  { "original": "alice@company.com", "masked": alun_utils::Mask::email("alice@company.com") },
        "id_card":{ "original": "320112199001011234", "masked": alun_utils::Mask::id_card("320112199001011234") },
        "name":   { "original": "张三丰", "masked": alun_utils::Mask::name("张三丰") },
    }))
}

/// 验证演示
async fn validate_demo(
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Res<serde_json::Value> {
    let s = params.get("input").cloned().unwrap_or_default();

    Res::ok(serde_json::json!({
        "input": s,
        "is_email": alun_utils::Valid::is_email(&s),
        "is_mobile": alun_utils::Valid::is_mobile(&s),
        "is_url": alun_utils::Valid::is_url(&s),
        "is_ipv4": alun_utils::Valid::is_ipv4(&s),
    }))
}

/// Crypto 演示
async fn crypto_demo() -> Res<serde_json::Value> {
    let hash = alun_utils::Crypto::sha256("alun");
    let key = alun_utils::Crypto::random_key();
    let key_hex = hex::encode(&key);
    let token = alun_utils::Crypto::random_token(32);

    Res::ok(serde_json::json!({
        "sha256_alun": hash,
        "random_key_hex": key_hex,
        "random_token": token,
    }))
}

// ──── 主函数 ──────────────────────────────────────

#[tokio::main]
async fn main() -> alun::Result<()> {
    let app = App::from_config()?;

    app
        .parse_cli()

        .get("/", index)
        .get("/api/health", health)

        .get("/api/demo/strings", string_demo)
        .get("/api/demo/ids", generate_ids)
        .get("/api/demo/date", date_demo)
        .get("/api/demo/mask", mask_demo)
        .get("/api/demo/validate", validate_demo)
        .get("/api/demo/crypto", crypto_demo)

        .serve("8080")
        .await
}

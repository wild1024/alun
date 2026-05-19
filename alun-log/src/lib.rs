//! 日志初始化：根据配置设置 tracing 输出

use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::EnvFilter;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

/// 初始化日志系统
///
/// 根据 `LogConfig` 设置 tracing 输出：
/// - `format = "text"`: 彩色文本格式（默认）
/// - `format = "json"`: JSON 结构化格式（对接 ELK/Loki）
/// - `dir` 非空时：日滚文件输出（按天创建新文件）
/// - `dir` 为空时：标准输出
///
/// # 日志过滤规则
///
/// 优先从环境变量 `RUST_LOG` 读取过滤级别；
/// 未设置时构建如下默认过滤器：
/// - **用户代码**：默认 `debug` 级别，确保业务日志（如 `tracing::debug!`）可见
/// - **alun 框架 crates**：使用 `LogConfig.level`（默认 `info`），抑制框架内部调试噪音
/// - **第三方喧闹 crates**（`tower_http`/`hyper`/`sqlx` 等）：固定 `warn` 级别
pub fn init(config: &alun_config::LogConfig) {
    let level = &config.level;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| build_default_filter(level));

    let format = config.format.as_str();

    // JSON 格式
    if format == "json" {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_span_events(FmtSpan::CLOSE)
            .json();

        if let Some(dir) = &config.dir {
            let file_appender = RollingFileAppender::new(Rotation::DAILY, dir, &config.file_prefix);
            let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
            let _ = subscriber.with_writer(non_blocking).try_init();
        } else {
            let _ = subscriber.try_init();
        }
    } else {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_span_events(FmtSpan::CLOSE);

        if let Some(dir) = &config.dir {
            let file_appender = RollingFileAppender::new(Rotation::DAILY, dir, &config.file_prefix);
            let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
            let _ = subscriber.with_writer(non_blocking).try_init();
        } else {
            let _ = subscriber.try_init();
        }
    }
}

/// 构建默认的日志过滤器
///
/// 默认规则：
/// - **用户代码**：`debug` 级别，确保业务日志（如 `tracing::debug!`）可见
/// - **alun 框架 crates**：使用配置的 `level`（默认 `info`），抑制框架内部调试噪音
/// - **喧闹的第三方 crates**（`tower_http`/`hyper`/`sqlx` 等）：固定 `warn` 级别，减少输出干扰
///
/// 用户可通过环境变量 `RUST_LOG` 完全覆盖此默认行为。
fn build_default_filter(level: &str) -> EnvFilter {
    let filter_str = format!(
        "debug,\
         alun={level},alun_web={level},alun_core={level},alun_db={level},\
         alun_config={level},alun_log={level},alun_macros={level},alun_utils={level},\
         alun_plugin={level},alun_cache={level},alun_task={level},alun_kafka={level},\
         alun_fs={level},alun_template={level},\
         tower_http=warn,hyper=warn,sqlx=warn,reqwest=warn,rustls=warn,\
         tokio_tungstenite=warn,tungstenite=warn,mio=warn,h2=warn",
        level = level
    );
    EnvFilter::new(&filter_str)
}

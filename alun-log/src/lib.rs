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
/// 优先从环境变量 `RUST_LOG` 读取过滤级别；
/// 未设置则使用 `LogConfig.level`（默认 `info`）。
pub fn init(config: &alun_config::LogConfig) {
    let level = &config.level;
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("alun={}", level)));

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

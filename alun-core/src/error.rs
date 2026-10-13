use std::fmt;

/// 框架骨架层的统一错误类型
///
/// 启动流程、插件生命周期、任务调度等**非 HTTP 场景**通过此类型返回错误。
/// 支持从 `std::io::Error`、`String`、`&str` 自动转换。
///
/// > **本类型不认识数据库，也不认识 HTTP**：
/// > `DbError` 属于 `alun-db-core`，`ApiError` 属于 `alun-web`，而本 crate 与它们
/// > **互不依赖**（硬约束，见 `alun-web/src/page.rs` 的说明）。HTTP 侧的收敛由
/// > `alun-web` 承担——`ApiError` 是它的本地类型，`From<任意外部错误>` 都合法，
/// > 因此业务端的 handler **只需要** `ResResult<T>` 与 `ApiError` 两个名字。
///
/// # 示例
///
/// ```ignore
/// fn load_config() -> alun_core::Result<AppConfig> {
///     Err(alun_core::Error::Config("config.toml 格式错误".into()))
/// }
/// ```
#[derive(Debug)]
pub enum Error {
    /// 配置加载或解析错误（如 TOML 格式错误、必填字段缺失）
    Config(String),
    /// 插件生命周期错误（携带插件名和底层错误源）
    Plugin {
        /// 出错的插件名称
        name: String,
        /// 底层错误源
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// 服务器启动或运行失败（如端口被占用）
    Server(String),
    /// IO 操作错误（如文件读写失败、网络故障）
    Io(std::io::Error),
    /// 模板渲染错误
    Template(String),
    /// 通用业务错误消息
    Msg(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Config(msg) => write!(f, "配置错误: {msg}"),
            Error::Plugin { name, source } => write!(f, "插件 [{name}] 错误: {source}"),
            Error::Server(msg) => write!(f, "服务器错误: {msg}"),
            Error::Io(e) => write!(f, "IO 错误: {e}"),
            Error::Template(msg) => write!(f, "模板错误: {msg}"),
            Error::Msg(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Plugin { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Msg(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Msg(s.to_string())
    }
}

/// 框架统一 Result 类型
///
/// `Ok(T)` 表示成功，`Err(Error)` 表示框架内任意模块的错误。
/// 所有公开 API 的返回值均使用此别名。
pub type Result<T> = std::result::Result<T, Error>;

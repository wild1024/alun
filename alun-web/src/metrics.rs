//! F10 · HTTP 侧可观测性：自动埋点 + Prometheus 导出端点
//!
//! 分两层，各自独立开关：
//!
//! | feature                 | 能力                                                        |
//! |-------------------------|-----------------------------------------------------------|
//! | `metrics`               | HTTP 自动埋点（计数器 / 直方图 / 在途 gauge）。无 recorder 时是 no-op |
//! | `metrics-prometheus`    | 安装 Prometheus recorder，并把 `/metrics` 挂到主路由              |
//!
//! # 指标清单
//!
//! | 指标 | 类型 | 标签 | 含义 |
//! |---|---|---|---|
//! | `alun_http_requests_total` | counter | `method` `route` `status` | 请求数（`status` 取类别 `2xx`/`4xx`/`5xx`） |
//! | `alun_http_request_duration_seconds` | histogram | `method` `route` | 请求耗时（秒） |
//! | `alun_http_requests_in_flight` | gauge | — | 在途请求数（容量与堆积观测） |
//!
//! # 两个刻意的取舍
//!
//! 1. **`route` 用"归一化路径"而非原始 URI**。
//!    直接用 URI 会把 `/users/1` `/users/2` … 变成无限多条时间序列
//!    （**标签基数爆炸**，是 Prometheus 最典型的自伤方式）。这里把纯数字段
//!    折叠为 `:id`、UUID 折叠为 `:uuid`、长令牌折叠为 `:token`，并对标签总数
//!    设硬上限（超出归 `__other__`）——把"能观测"和"不会把自己打挂"同时保住。
//!
//! 2. **`status` 用类别而非精确码**。
//!    精确码只有十来个取值，看似无害，但乘以 `route` 后序列数翻倍；而
//!    "4xx 率 / 5xx 率"才是 SLO 真正要看的量。需要定位具体码时查日志。

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{LazyLock, OnceLock};
use std::time::Instant;

use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};
use parking_lot::RwLock;
use tower::{Layer, Service};

use alun_config::MetricsConfig;

/// 请求计数
pub const HTTP_REQUESTS_TOTAL: &str = "alun_http_requests_total";
/// 请求耗时（秒，直方图）
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "alun_http_request_duration_seconds";
/// 在途请求数
pub const HTTP_REQUESTS_IN_FLIGHT: &str = "alun_http_requests_in_flight";

/// `route` 标签的**硬上限**。
///
/// 超过后新路径统一归为 [`ROUTE_OTHER`](Self::ROUTE_OTHER)。这是**兜底而非
/// 主要机制**：正常应用的路由数远小于此；触发上限通常意味着有人在用
/// 通配路由承接任意路径（或正在被路径遍历扫描），此时恰恰不该为新路径建序列。
pub const MAX_ROUTE_LABELS: usize = 512;

/// 路径标签超限时使用的归并值。
pub const ROUTE_OTHER: &str = "__other__";

/// 已见过的 `route` 标签集合（值经 `Box::leak` 转 `'static`，数量有界）。
static ROUTE_LABELS: LazyLock<RwLock<HashSet<&'static str>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

/// 注册**本 crate 及已启用的下层**的指标元数据（单位 + 说明）。
///
/// 主入口是 [`install_prometheus`]（recorder 装好后再调用）；各埋点里的惰性
/// 注册只是**兜底**——`metrics` 在没有 recorder 时会**静默丢弃** `describe_*`，
/// 一旦惰性分支先跑过一次，后续就再也补不上。
pub fn describe_all() {
    describe();
    #[cfg(all(feature = "db", feature = "metrics"))]
    alun_db::metrics::describe();
    #[cfg(all(feature = "cache", feature = "metrics"))]
    alun_cache::metrics::describe();
}

/// 注册指标元数据（单位 + 说明），让 Prometheus 的文本输出带上 `# HELP`。
///
/// **幂等**；由埋点在首个请求时自动触发，也可在装配阶段显式调用
/// （进程启动即暴露 `/metrics`、还没收到流量时也能看到说明）。
pub fn describe() {
    metrics::describe_counter!(
        HTTP_REQUESTS_TOTAL,
        metrics::Unit::Count,
        "HTTP 请求数；status 取类别（2xx/4xx/5xx），route 为归一化路径"
    );
    metrics::describe_histogram!(
        HTTP_REQUEST_DURATION_SECONDS,
        metrics::Unit::Seconds,
        "HTTP 请求耗时（秒）"
    );
    metrics::describe_gauge!(
        HTTP_REQUESTS_IN_FLIGHT,
        metrics::Unit::Count,
        "当前在途请求数（容量与堆积观测）"
    );
}

/// 把路径折叠为**低基数**的 `route` 标签。
///
/// - 纯数字段 → `:id`
/// - UUID → `:uuid`
/// - 长度 ≥ 24 的字母数字串（令牌 / 哈希 / 雪花 ID） → `:token`
/// - 其余原样保留（路由模板本身）
///
/// # 为什么用 `Box::leak`
///
/// `metrics` 的标签值需要 `SharedString`（本质 `Cow<'static, str>`）。每个请求
/// 都 `to_string()` 会产生一次堆分配，而路径标签的集合是**有限且小**的——
/// 泄漏有界（≤ [`MAX_ROUTE_LABELS`]），换来热路径上零分配的 `&'static str`。
/// 这是"有界泄漏"与"每请求分配"之间的明确取舍，前者在长期运行的服务里更划算。
fn route_label(path: &str) -> &'static str {
    let normalized = normalize_path(path);
    {
        let guard = ROUTE_LABELS.read();
        if let Some(&known) = guard.get(normalized.as_str()) {
            return known;
        }
    }
    let mut guard = ROUTE_LABELS.write();
    // 双检：上面的读锁释放期间可能已有其它线程插入
    if let Some(&known) = guard.get(normalized.as_str()) {
        return known;
    }
    if guard.len() >= MAX_ROUTE_LABELS {
        return ROUTE_OTHER;
    }
    let leaked: &'static str = Box::leak(normalized.into_boxed_str());
    guard.insert(leaked);
    leaked
}

/// 路径归一化（见 [`route_label`]）。
fn normalize_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for seg in path.split('/') {
        // 跳过空段（前导 `/` 与重复的 `//`），避免产出 `//users` 这类双斜杠标签
        if seg.is_empty() {
            continue;
        }
        out.push('/');
        if seg.bytes().all(|b| b.is_ascii_digit()) {
            out.push_str(":id");
        } else if is_uuid(seg) {
            out.push_str(":uuid");
        } else if seg.len() >= 24
            && seg
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            out.push_str(":token");
        } else {
            out.push_str(seg);
        }
    }
    if out.is_empty() { "/".to_string() } else { out }
}

/// 形如 `8-4-4-4-12` 的十六进制串。
fn is_uuid(seg: &str) -> bool {
    seg.len() == 36
        && matches!(seg.as_bytes().get(8), Some(b'-'))
        && matches!(seg.as_bytes().get(13), Some(b'-'))
        && matches!(seg.as_bytes().get(18), Some(b'-'))
        && matches!(seg.as_bytes().get(23), Some(b'-'))
        && seg
            .bytes()
            .enumerate()
            .all(|(i, b)| b.is_ascii_hexdigit() || matches!(i, 8 | 13 | 18 | 23))
}

/// HTTP 方法 → 静态标签值（`metrics` 的标签不接受非 `'static` 的 `&str`）。
fn method_label(method: &Method) -> &'static str {
    if method == Method::GET {
        "GET"
    } else if method == Method::POST {
        "POST"
    } else if method == Method::PUT {
        "PUT"
    } else if method == Method::PATCH {
        "PATCH"
    } else if method == Method::DELETE {
        "DELETE"
    } else if method == Method::HEAD {
        "HEAD"
    } else if method == Method::OPTIONS {
        "OPTIONS"
    } else {
        "OTHER"
    }
}

/// 状态码 → 类别标签。
fn status_label(status: StatusCode) -> &'static str {
    match status.as_u16() / 100 {
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        5 => "5xx",
        _ => "other",
    }
}

// ───────────────────────────── HTTP 埋点 ─────────────────────────────

/// HTTP 指标中间件。
///
/// 应挂在**最外层**（与 `RequestLogLayer` 同级），这样 401 / 429 / 500 等
/// 被内层中间件拒绝的请求同样计入——否则限流与鉴权失败会从 SLO 里"消失"。
#[derive(Clone, Default)]
pub struct MetricsLayer {
    /// 不埋点的路径（如 `/metrics` 自身、`/health`）。
    ///
    /// **务必把指标端点自身排除**：否则 Prometheus 每 15s 抓取一次就会给
    /// 计数器加一，抓取频率与请求量混在一起，指标自己污染自己。
    pub exclude_paths: Vec<String>,
}

impl MetricsLayer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn exclude_paths(mut self, paths: Vec<String>) -> Self {
        self.exclude_paths = paths;
        self
    }
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsService {
            inner,
            exclude_paths: self.exclude_paths.clone(),
        }
    }
}

/// [`MetricsLayer`] 产出的服务。
#[derive(Clone)]
pub struct MetricsService<S> {
    inner: S,
    exclude_paths: Vec<String>,
}

impl<S, ReqBody> Service<Request<ReqBody>> for MetricsService<S>
where
    S: Service<Request<ReqBody>, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        // 首次请求时注册指标元数据（`# HELP`）；之后只剩一次原子读。
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(describe);

        let path = req.uri().path().to_string();
        let counted = !self.exclude_paths.iter().any(|p| p == &path);
        let method = method_label(req.method());
        let route = if counted {
            route_label(&path)
        } else {
            "excluded"
        };

        let start = Instant::now();
        let mut inner = self.inner.clone();
        if counted {
            metrics::gauge!(HTTP_REQUESTS_IN_FLIGHT).increment(1.0);
        }

        Box::pin(async move {
            let out = inner.call(req).await;
            if counted {
                let status = match &out {
                    Ok(resp) => status_label(resp.status()),
                    Err(_) => "error",
                };
                metrics::gauge!(HTTP_REQUESTS_IN_FLIGHT).decrement(1.0);
                metrics::counter!(
                    HTTP_REQUESTS_TOTAL,
                    "method" => method,
                    "route" => route,
                    "status" => status
                )
                .increment(1);
                metrics::histogram!(
                    HTTP_REQUEST_DURATION_SECONDS,
                    "method" => method,
                    "route" => route
                )
                .record(start.elapsed().as_secs_f64());
            }
            out
        })
    }
}

// ─────────────────── Prometheus 导出（feature metrics-prometheus） ───────────────────

#[cfg(feature = "metrics-prometheus")]
mod prometheus {
    use super::*;
    use metrics_exporter_prometheus::PrometheusBuilder;

    /// 全局 Prometheus 句柄（安装后可读）。
    pub(super) static HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
        OnceLock::new();
    /// 端点 IP 白名单（安装时快照；空 = 仅本机回环）。
    pub(super) static ALLOW_IPS: OnceLock<Vec<String>> = OnceLock::new();

    /// 安装 Prometheus recorder 并启动后台 upkeep。
    ///
    /// `upkeep` 负责回收空闲序列与推进直方图的时间窗口；不跑它，长期运行下
    /// 序列集合只增不减。框架代为 spawn，业务无需感知。
    /// 延迟直方图的桶边界（秒）。
    ///
    /// 默认导出是 **summary**（分位数），但分位数无法在 Prometheus 侧聚合，
    /// 而 HTTP 延迟最常用的是 `histogram_quantile()` 与 `_bucket` 的速率计算。
    /// 因此显式改用 **histogram + 固定桶**：从 1ms（内网命中）到 5s（慢查询），
    /// 覆盖 Web 服务的完整量级。
    const BUCKETS: &[f64] = &[
        0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
    ];

    pub fn install(cfg: &MetricsConfig) -> alun_core::Result<()> {
        let builder = PrometheusBuilder::new();
        let builder = builder
            .set_buckets(BUCKETS)
            .unwrap_or_else(|_| PrometheusBuilder::new());
        let recorder = builder.build_recorder();
        let handle = recorder.handle();
        // 先落句柄再装全局：即使全局安装失败（重复安装），端点仍可渲染
        // （内容为空），不会出现"端点 404"这种更难排查的次生故障。
        let _ = HANDLE.set(handle.clone());
        let _ = ALLOW_IPS.set(cfg.allow_ips.clone());
        metrics::set_global_recorder(recorder)
            .map_err(|_| alun_core::Error::Config("metrics: 全局 recorder 已安装".into()))?;

        // recorder 就位**之后**再注册元数据。`metrics` 门面的 `describe_*` 在
        // 没有 recorder 时是**静默丢弃**的（不是缓存后再补），所以顺序反了
        // 就永远拿不到 `# HELP` 行——这也是各 crate 里那个 `Once` 惰性注册
        // 只能当"兜底"、不能当主路径的原因。
        describe_all();

        // 仅在真正成为全局 recorder 时才起 upkeep：重复安装时再 spawn 一个循环
        // 只会白白多出一个常驻任务。
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tick.tick().await;
                handle.run_upkeep();
            }
        });
        Ok(())
    }

    /// 渲染当前指标文本（未安装时返回 `None`）。
    pub fn render() -> Option<String> {
        HANDLE.get().map(|h| h.render())
    }
}

#[cfg(feature = "metrics-prometheus")]
pub use prometheus::{install as install_prometheus, render as render_metrics};

/// `/metrics` 端点处理函数。
///
/// # 访问控制
///
/// 端点挂在**中间件链之外**（`Router::layer` 只作用于注册时已存在的路由，
/// 因此在链之后添加的路由不穿任何 layer），所以它既不受 `AuthLayer` 保护，
/// 也不受 `RateLimitLayer` 限制——**访问控制只能靠自己**，这里按
/// `[metrics].allow_ips` 判定：
///
/// - 白名单为空 → 仅允许回环地址（`127.0.0.1` / `::1`）；
/// - 命中白名单或回环 → 返回指标文本；
/// - 其余 → `403`，且**不泄露**端点是否存在（返回体为空）。
#[cfg(feature = "metrics-prometheus")]
pub async fn metrics_endpoint(req: Request) -> Response {
    if !client_allowed(&req) {
        return (StatusCode::FORBIDDEN).into_response();
    }
    match prometheus::render() {
        Some(body) => (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                "text/plain; version=0.0.4; charset=utf-8",
            )],
            body,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND).into_response(),
    }
}

/// 判定请求来源是否有权访问 `/metrics`。
#[cfg(feature = "metrics-prometheus")]
fn client_allowed(req: &Request) -> bool {
    let allow = prometheus::ALLOW_IPS.get().cloned().unwrap_or_default();
    match client_ip(req) {
        // 拿不到对端地址（如自定义 serve）时按"非回环"处理，避免误放行
        None => false,
        Some(ip) => {
            if ip.is_loopback() {
                return true;
            }
            allow.iter().any(|a| a.parse::<IpAddr>().ok() == Some(ip))
        }
    }
}

/// 取客户端 IP：`X-Forwarded-For` → `X-Real-IP` → TCP 对端。
///
/// 与 `RequestLogLayer` 的取值顺序保持一致，避免"日志里的 IP 和指标端点的
/// 判定 IP 不是同一个"这类排查陷阱。
#[cfg(feature = "metrics-prometheus")]
fn client_ip(req: &Request) -> Option<IpAddr> {
    let header_ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.trim().parse::<IpAddr>().ok())
        .or_else(|| {
            req.headers()
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<IpAddr>().ok())
        });
    header_ip.or_else(|| {
        req.extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| ci.0.ip())
    })
}

/// 把 `/metrics` 端点挂到 `router` 上。
///
/// 必须在**中间件链构建完成之后**调用：axum 的 `layer` 只作用于调用时已存在
/// 的路由，后加的这条因此天然"裸奔"——不穿鉴权、不穿限流、也不穿请求日志，
/// 这在语义上正是我们想要的（抓取器不带 Token）。代价是访问控制必须
/// 由 [`metrics_endpoint`] 自己做，见其文档。
#[cfg(feature = "metrics-prometheus")]
pub fn attach_endpoint(router: Router, cfg: &MetricsConfig) -> Router {
    router.route(&cfg.path, get(metrics_endpoint))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    /// 取渲染句柄（全局 recorder 只能装一次，**多测试共享**）。
    ///
    /// 走 `install_prometheus` 而不是自行 `install_recorder`：前者把句柄先落进
    /// `prometheus::HANDLE` 再装全局，因此无论哪个测试先跑，后续调用拿到的都是
    /// **同一个、且与全局 recorder 一致**的句柄——否则"谁先谁成功、后者 panic"
    /// 会造成随机失败（测试并行顺序不定）。
    #[cfg(feature = "metrics-prometheus")]
    fn test_handle() -> metrics_exporter_prometheus::PrometheusHandle {
        let _ = install_prometheus(&MetricsConfig::default());
        prometheus::HANDLE
            .get()
            .cloned()
            .expect("prometheus handle 应已安装")
    }

    #[cfg(not(feature = "metrics-prometheus"))]
    fn test_handle() -> metrics_exporter_prometheus::PrometheusHandle {
        static H: std::sync::OnceLock<metrics_exporter_prometheus::PrometheusHandle> =
            std::sync::OnceLock::new();
        H.get_or_init(|| {
            metrics_exporter_prometheus::PrometheusBuilder::new()
                .install_recorder()
                .expect("install recorder")
        })
        .clone()
    }

    #[test]
    fn path_segments_are_collapsed() {
        // 数字主键 → `:id`（否则 `/users/1`、`/users/2`… 各成一条序列）
        assert_eq!(normalize_path("/users/123"), "/users/:id");
        assert_eq!(normalize_path("/"), "/");
        // UUID → `:uuid`
        assert_eq!(
            normalize_path("/files/3f2b8c1e-9a4d-4f7b-8c2e-1d5a6b7c8d9e"),
            "/files/:uuid"
        );
        // 长令牌 / 哈希 → `:token`
        assert_eq!(
            normalize_path("/t/0123456789abcdef0123456789abcdef"),
            "/t/:token"
        );
        // 路由模板本身原样保留
        assert_eq!(normalize_path("/api/v1/users"), "/api/v1/users");
    }

    #[test]
    fn same_path_yields_same_static_label() {
        let a = route_label("/orders/1");
        let b = route_label("/orders/2");
        assert_eq!(a, b, "不同主键应折叠为同一个 route 标签");
        assert_eq!(a, "/orders/:id");
    }

    #[tokio::test]
    async fn http_requests_are_counted_with_route_label() {
        let handle = test_handle();
        let app = Router::new()
            .route("/users/{id}", get(|| async { "ok" }))
            .layer(MetricsLayer::new().exclude_paths(vec!["/metrics".into()]));

        let resp = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/users/42")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK);

        let rendered = handle.render();
        assert!(
            rendered.contains("alun_http_requests_total"),
            "应记录请求计数：{rendered}"
        );
        assert!(
            rendered.contains("alun_http_request_duration_seconds"),
            "应记录耗时直方图：{rendered}"
        );
        assert!(
            rendered.contains(r#"route="/users/:id""#),
            "route 标签应为归一化路径：{rendered}"
        );
        assert!(
            rendered.contains(r#"status="2xx""#),
            "status 应为类别而非精确码：{rendered}"
        );
    }

    #[tokio::test]
    async fn excluded_paths_are_not_counted() {
        let handle = test_handle();
        let before = handle
            .render()
            .lines()
            .filter(|l| {
                l.starts_with("alun_http_requests_total") && l.contains(r#"route="/metrics""#)
            })
            .count();
        let app = Router::new()
            .route("/metrics", get(|| async { "m" }))
            .layer(MetricsLayer::new().exclude_paths(vec!["/metrics".into()]));
        let _ = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        let after = handle
            .render()
            .lines()
            .filter(|l| {
                l.starts_with("alun_http_requests_total") && l.contains(r#"route="/metrics""#)
            })
            .count();
        assert_eq!(
            before, after,
            "抓取端点自身不应计入请求量（否则抓取频率污染业务指标）"
        );
    }

    /// 端点挂在中间件链之外 ⇒ 访问控制只能靠自己：非回环默认 403。
    #[cfg(feature = "metrics-prometheus")]
    #[tokio::test]
    async fn metrics_endpoint_is_private_by_default() {
        let cfg = MetricsConfig::default();
        // 允许重复安装失败（其它测试可能已装全局 recorder）
        let _ = install_prometheus(&cfg);
        let app = attach_endpoint(Router::new(), &cfg);

        // 无 `ConnectInfo`（拿不到对端地址）→ 按"非回环"处理 → 403
        let resp = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("oneshot");
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "未知来源访问指标端点应被拒绝"
        );

        // 回环地址放行
        let mut req = HttpRequest::builder()
            .uri("/metrics")
            .body(Body::empty())
            .expect("request");
        let loopback: std::net::SocketAddr = "127.0.0.1:5555".parse().expect("addr");
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(loopback));
        let resp = app.oneshot(req).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK, "本机回环应可抓取指标");
    }
}

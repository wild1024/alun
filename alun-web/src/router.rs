
use axum::Router;
use axum::handler::Handler;
use axum::routing::{get, post, put, delete};

type RouteEntry = Box<dyn FnOnce(Router) -> Router + Send>;

/// 路由注册器 —— 延迟构建 axum Router
///
/// 所有路由以闭包形式暂存，调用 `into_axum()` 时一次性构建。
/// 支持合并（`merge`）和嵌套（`nest`）。
pub struct AlunRouter {
    /// 路由构建闭包列表
    builders: Vec<RouteEntry>,
}

impl AlunRouter {
    /// 创建空的路由注册器
    pub fn new() -> Self {
        Self {
            builders: Vec::new(),
        }
    }

    /// 注册 GET 路由
    pub fn add_get<H, T>(&mut self, path: &str, handler: H)
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders.push(Box::new(move |r| r.route(&path, get(handler))));
    }

    /// 注册 POST 路由
    pub fn add_post<H, T>(&mut self, path: &str, handler: H)
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders.push(Box::new(move |r| r.route(&path, post(handler))));
    }

    /// 注册 PUT 路由
    pub fn add_put<H, T>(&mut self, path: &str, handler: H)
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders.push(Box::new(move |r| r.route(&path, put(handler))));
    }

    /// 注册 DELETE 路由
    pub fn add_delete<H, T>(&mut self, path: &str, handler: H)
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders.push(Box::new(move |r| r.route(&path, delete(handler))));
    }

    /// 注册通用路由（指定 HTTP 方法字符串）
    ///
    /// 支持 GET/POST/PUT/DELETE，其他方法使用 `axum::routing::any`。
    pub fn add_route<H, T>(&mut self, method: &str, path: &str, handler: H)
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        let method = method.to_uppercase();
        self.builders.push(Box::new(move |r| {
            let router = match method.as_str() {
                "GET"    => get(handler),
                "POST"   => post(handler),
                "PUT"    => put(handler),
                "DELETE" => delete(handler),
                _        => axum::routing::any(handler),
            };
            r.route(&path, router)
        }));
    }

    /// 注册带 `tower::Layer` 的 GET 路由 —— 对标 axum 的 `get(handler).route_layer(layer)`
    ///
    /// `wrap` 闭包接收 `get(handler)` 产生的 `MethodRouter`，用户可调用 `.route_layer(layer)` 添加方法级中间件。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// use crate::middleware::{RequirePermissionLayer, RequireRoleLayer};
    ///
    /// router.add_get_with_layer("/admin", admin_handler, |mr| {
    ///     mr.route_layer(RequireRoleLayer::any(vec!["admin".into()]))
    /// });
    /// ```
    pub fn add_get_with_layer<H, T>(
        &mut self, path: &str, handler: H,
        wrap: impl FnOnce(axum::routing::MethodRouter<()>) -> axum::routing::MethodRouter<()> + Send + 'static,
    )
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders.push(Box::new(move |r| {
            let mr = get(handler);
            let mr = wrap(mr);
            r.route(&path, mr)
        }));
    }

    /// 注册带 `tower::Layer` 的 POST 路由
    pub fn add_post_with_layer<H, T>(
        &mut self, path: &str, handler: H,
        wrap: impl FnOnce(axum::routing::MethodRouter<()>) -> axum::routing::MethodRouter<()> + Send + 'static,
    )
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders.push(Box::new(move |r| {
            let mr = post(handler);
            let mr = wrap(mr);
            r.route(&path, mr)
        }));
    }

    /// 注册带 `tower::Layer` 的 PUT 路由
    pub fn add_put_with_layer<H, T>(
        &mut self, path: &str, handler: H,
        wrap: impl FnOnce(axum::routing::MethodRouter<()>) -> axum::routing::MethodRouter<()> + Send + 'static,
    )
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders.push(Box::new(move |r| {
            let mr = put(handler);
            let mr = wrap(mr);
            r.route(&path, mr)
        }));
    }

    /// 注册带 `tower::Layer` 的 DELETE 路由
    pub fn add_delete_with_layer<H, T>(
        &mut self, path: &str, handler: H,
        wrap: impl FnOnce(axum::routing::MethodRouter<()>) -> axum::routing::MethodRouter<()> + Send + 'static,
    )
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders.push(Box::new(move |r| {
            let mr = delete(handler);
            let mr = wrap(mr);
            r.route(&path, mr)
        }));
    }

    /// 合并子路由注册器（嵌套到指定前缀下）
    pub fn merge(&mut self, prefix: &str, sub: AlunRouter) {
        let prefix = prefix.to_string();
        self.builders.push(Box::new(move |r| r.nest(&prefix, sub.into_axum())));
    }

    /// 消耗注册器，构建最终的 axum Router
    pub fn into_axum(self) -> Router {
        self.builders
            .into_iter()
            .fold(Router::new(), |r, f| f(r))
    }
}

impl Default for AlunRouter {
    fn default() -> Self {
        Self::new()
    }
}


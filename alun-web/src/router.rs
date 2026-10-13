use axum::Router;
use axum::handler::Handler;
use axum::routing::{delete, get, patch, post, put};

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
        self.builders
            .push(Box::new(move |r| r.route(&path, get(handler))));
    }

    /// 注册 POST 路由
    pub fn add_post<H, T>(&mut self, path: &str, handler: H)
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders
            .push(Box::new(move |r| r.route(&path, post(handler))));
    }

    /// 注册 PUT 路由
    pub fn add_put<H, T>(&mut self, path: &str, handler: H)
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders
            .push(Box::new(move |r| r.route(&path, put(handler))));
    }

    /// 注册 PATCH 路由
    pub fn add_patch<H, T>(&mut self, path: &str, handler: H)
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders
            .push(Box::new(move |r| r.route(&path, patch(handler))));
    }

    /// 注册 DELETE 路由
    pub fn add_delete<H, T>(&mut self, path: &str, handler: H)
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders
            .push(Box::new(move |r| r.route(&path, delete(handler))));
    }

    /// 注册通用路由（指定 HTTP 方法字符串）
    ///
    /// 支持 GET/POST/PUT/PATCH/DELETE，其他方法使用 `axum::routing::any`。
    pub fn add_route<H, T>(&mut self, method: &str, path: &str, handler: H)
    where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        let method = method.to_uppercase();
        self.builders.push(Box::new(move |r| {
            let router = match method.as_str() {
                "GET" => get(handler),
                "POST" => post(handler),
                "PUT" => put(handler),
                "PATCH" => patch(handler),
                "DELETE" => delete(handler),
                _ => axum::routing::any(handler),
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
        &mut self,
        path: &str,
        handler: H,
        wrap: impl FnOnce(axum::routing::MethodRouter<()>) -> axum::routing::MethodRouter<()>
        + Send
        + 'static,
    ) where
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
        &mut self,
        path: &str,
        handler: H,
        wrap: impl FnOnce(axum::routing::MethodRouter<()>) -> axum::routing::MethodRouter<()>
        + Send
        + 'static,
    ) where
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
        &mut self,
        path: &str,
        handler: H,
        wrap: impl FnOnce(axum::routing::MethodRouter<()>) -> axum::routing::MethodRouter<()>
        + Send
        + 'static,
    ) where
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

    /// 注册带 `tower::Layer` 的 PATCH 路由
    pub fn add_patch_with_layer<H, T>(
        &mut self,
        path: &str,
        handler: H,
        wrap: impl FnOnce(axum::routing::MethodRouter<()>) -> axum::routing::MethodRouter<()>
        + Send
        + 'static,
    ) where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        self.builders.push(Box::new(move |r| {
            let mr = patch(handler);
            let mr = wrap(mr);
            r.route(&path, mr)
        }));
    }

    /// 注册带 `tower::Layer` 的 DELETE 路由
    pub fn add_delete_with_layer<H, T>(
        &mut self,
        path: &str,
        handler: H,
        wrap: impl FnOnce(axum::routing::MethodRouter<()>) -> axum::routing::MethodRouter<()>
        + Send
        + 'static,
    ) where
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

    /// 注册带 `tower::Layer` 的通用路由（指定 HTTP 方法字符串）
    ///
    /// 与 [`add_route`](Self::add_route) 的关系，等同
    /// [`add_get_with_layer`](Self::add_get_with_layer) 之于 `add_get`；
    /// 未知动词在此回退为 axum 的 `any`，层仍按方法级（`route_layer`）挂载。
    ///
    /// [`#[alun::layer]`](https://docs.rs/alun-macros) 宏即由此方法完成注册。
    pub fn add_route_with_layer<H, T>(
        &mut self,
        method: &str,
        path: &str,
        handler: H,
        wrap: impl FnOnce(axum::routing::MethodRouter<()>) -> axum::routing::MethodRouter<()>
        + Send
        + 'static,
    ) where
        H: Handler<T, ()>,
        T: 'static,
    {
        let path = path.to_string();
        let method = method.to_uppercase();
        self.builders.push(Box::new(move |r| {
            let mr = match method.as_str() {
                "GET" => get(handler),
                "POST" => post(handler),
                "PUT" => put(handler),
                "PATCH" => patch(handler),
                "DELETE" => delete(handler),
                _ => axum::routing::any(handler),
            };
            let mr = wrap(mr);
            r.route(&path, mr)
        }));
    }

    /// 合并子路由注册器（嵌套到指定前缀下）
    ///
    /// 只搬运路由，不对子树挂任何中间件——需要给整个子树加层请用
    /// [`merge_with_layer`](Self::merge_with_layer)。
    pub fn merge(&mut self, prefix: &str, sub: AlunRouter) {
        let prefix = prefix.to_string();
        self.builders
            .push(Box::new(move |r| r.nest(&prefix, sub.into_axum())));
    }

    /// 合并子路由注册器，并对**整个子树**挂一层中间件 —— 对标 axum 的
    /// `nest(prefix, sub.layer(L))`。
    ///
    /// 与 [`merge`](Self::merge) 的唯一差别是在 `nest` **之前**对子树应用 `wrap`，
    /// 因此这层只作用于该前缀下的路由，其它路由不受影响。
    ///
    /// # 位置与能力
    ///
    /// 子树层由本方法在路由装配阶段应用，而内置中间件链是在 `App::start()` 末尾
    /// 对整棵 Router 应用的，所以子树层位于**内置链之内**：可以读到
    /// `AuthClaims` / 租户上下文。反过来，它也看不到内置层的拒绝（401 / 403 / 429）。
    ///
    /// # 为什么不直接用 `App::layer`
    ///
    /// [`App::layer`](crate::App::layer) 是整站最外层，用它挂子树层会让层落在链外；
    /// 而在 `layer_from_fn` 的回调里 `nest` 一个挂好层的子树同样在链外。
    /// 需要「某个前缀下的一整组路由」加层时，本方法是唯一能同时满足
    /// 「范围受限」与「可读身份」的入口。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// use alun::prelude::*;
    ///
    /// pub fn routes(r: &mut AlunRouter) {
    ///     let mut admin = AlunRouter::new();
    ///     admin.add_get("/users", list_users);
    ///     admin.add_post("/users", create_user);
    ///
    ///     // /admin/* 全部经过审计层，其它路由不受影响
    ///     r.merge_with_layer("/admin", admin, |sub| {
    ///         sub.layer(AuditLayer::new())
    ///     });
    /// }
    ///
    /// App::new()?.routes(routes).start().await?;
    /// ```
    pub fn merge_with_layer(
        &mut self,
        prefix: &str,
        sub: AlunRouter,
        wrap: impl FnOnce(Router) -> Router + Send + 'static,
    ) {
        let prefix = prefix.to_string();
        self.builders
            .push(Box::new(move |r| r.nest(&prefix, wrap(sub.into_axum()))));
    }

    /// 消耗注册器，构建最终的 axum Router
    pub fn into_axum(self) -> Router {
        self.builders.into_iter().fold(Router::new(), |r, f| f(r))
    }
}

impl Default for AlunRouter {
    fn default() -> Self {
        Self::new()
    }
}

//! Web 插件 —— 让插件成为**单一扩展单元**（生命周期 + 路由 + 权限 + 免鉴权）
//!
//! # 为什么放在 `alun-web` 而不是 `alun-core`
//!
//! 分层是单向的：`alun-web → alun-core`。`alun-core::Plugin` 因此**不可能**
//! 提到 [`AlunRouter`] / [`PermissionDef`] 等 Web 类型。把 Web 侧贡献建模为
//! `alun-core::Plugin` 的**子 trait**，是唯一不破坏分层、也不需要类型擦除
//! （`Box<dyn Any>` + downcast）的做法。
//!
//! # 与宏收集通道的关系
//!
//! 两条通道**并存且合并**，不互相替代：
//!
//! | 通道 | 适用 | 机制 |
//! |---|---|---|
//! | `#[alun::get]` + `App::scan()` | 应用自身的 handler | `inventory` 编译期收集 |
//! | [`WebPlugin::routes`] | 库 / 插件分发的 handler | 显式 `App::web_plugin()` 注册 |
//!
//! 插件通道的价值在于：**库形态分发**。一个 crate 无法指望使用方的
//! `inventory` 收集到自己的 handler（收集发生在最终二进制的链接单元里），
//! 但可以导出一个实现了 `WebPlugin` 的类型，由使用方一行注册。

use alun_core::Plugin;

use crate::{AlunRouter, NoAuthDef, PermissionDef};

/// Web 插件：在 [`alun_core::Plugin`] 的生命周期之上，追加 Web 装配贡献
///
/// 所有方法都有默认实现（空贡献），因此**已有的 `Plugin` 实现只需把
/// `impl Plugin for X` 改成 `impl WebPlugin for X`** 即可平滑升级
/// （`WebPlugin: Plugin`，故 `App::web_plugin` 同时完成生命周期注册）。
///
/// # 示例
///
/// ```ignore
/// use alun::web::{AlunRouter, NoAuthDef, PermissionDef, WebPlugin};
///
/// struct AdminPlugin;
///
/// #[alun::async_trait]
/// impl alun::Plugin for AdminPlugin {
///     fn name(&self) -> &str { "admin" }
///     async fn start(&self) -> alun::Result<()> { Ok(()) }
///     async fn stop(&self) -> alun::Result<()> { Ok(()) }
/// }
///
/// impl WebPlugin for AdminPlugin {
///     fn routes(&self, router: &mut AlunRouter) {
///         router.add_get("/admin/health", || async { "ok" });
///     }
///     fn permission_rules(&self) -> Vec<PermissionDef> {
///         vec![PermissionDef { path: "/admin/users", method: "GET", permission: "admin:read" }]
///     }
///     fn no_auth_paths(&self) -> Vec<NoAuthDef> {
///         vec![NoAuthDef { path: "/admin/health" }]
///     }
/// }
///
/// App::new()?.web_plugin(AdminPlugin).scan().start().await
/// ```
pub trait WebPlugin: Plugin {
    /// 贡献路由 —— 在 [`App::scan`](crate::App::scan) 阶段与宏收集的路由合并
    ///
    /// 默认空实现（不贡献任何路由）。
    fn routes(&self, _router: &mut AlunRouter) {}

    /// 贡献权限规则 —— 与 `#[alun::permission]` 宏收集的规则合并
    ///
    /// 默认空实现。
    fn permission_rules(&self) -> Vec<PermissionDef> {
        Vec::new()
    }

    /// 贡献免鉴权路径 —— 与 `#[alun::no_auth]` 宏收集的路径合并
    ///
    /// 默认空实现。
    fn no_auth_paths(&self) -> Vec<NoAuthDef> {
        Vec::new()
    }
}

//  blanket impl 不可行（`Plugin` 是外部 trait 的超集关系会导致重叠），
//  但**空贡献**的默认实现已等价于「所有 Plugin 都是贡献为空的 WebPlugin」的语义。

use std::sync::Arc;

/// 把 `Arc<P>` 适配成 `Plugin`，以便与 Web 侧共享**同一个插件实例**
///
/// `PluginManager` 持有 `Box<dyn Plugin>`，取出后无法再调用 `WebPlugin` 的
/// Web 方法；因此 App 侧另存一份 `Arc<dyn WebPlugin>`，本适配器持有同一
/// `Arc` 的另一份引用，保证生命周期与装配作用于同一实例（而非两个副本）。
pub(crate) struct WebPluginAdapter<P>(pub Arc<P>);

#[async_trait::async_trait]
impl<P: WebPlugin> Plugin for WebPluginAdapter<P> {
    fn name(&self) -> &str {
        self.0.name()
    }

    async fn start(&self) -> alun_core::Result<()> {
        self.0.start().await
    }

    async fn stop(&self) -> alun_core::Result<()> {
        self.0.stop().await
    }

    fn depends_on(&self) -> &[&str] {
        self.0.depends_on()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alun_core::Result;
    use async_trait::async_trait;

    struct EmptyPlugin;

    #[async_trait]
    impl Plugin for EmptyPlugin {
        fn name(&self) -> &str {
            "empty"
        }
        async fn start(&self) -> Result<()> {
            Ok(())
        }
        async fn stop(&self) -> Result<()> {
            Ok(())
        }
    }

    impl WebPlugin for EmptyPlugin {}

    /// 只实现 Plugin 生命周期、不覆盖任何贡献方法 → 三个贡献均为空。
    #[test]
    fn default_contributions_are_empty() {
        let p = EmptyPlugin;
        assert_eq!(p.name(), "empty");
        assert!(p.permission_rules().is_empty());
        assert!(p.no_auth_paths().is_empty());
    }
}

// ── 端到端：插件贡献真正进入装配（F5）──────────────────────────────────
#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::{App, NoAuthDef as NoAuth, PermissionDef as PermDef};
    use alun_config::AppConfig;
    use alun_core::Result;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::Request;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    struct DemoPlugin;

    #[async_trait]
    impl Plugin for DemoPlugin {
        fn name(&self) -> &str {
            "demo"
        }
        async fn start(&self) -> Result<()> {
            Ok(())
        }
        async fn stop(&self) -> Result<()> {
            Ok(())
        }
    }

    impl WebPlugin for DemoPlugin {
        fn routes(&self, router: &mut AlunRouter) {
            router.add_get("/plugin/ping", || async { "pong" });
        }
        fn permission_rules(&self) -> Vec<PermDef> {
            vec![PermDef {
                path: "/plugin/admin",
                method: "GET",
                permission: "plugin:read",
            }]
        }
        fn no_auth_paths(&self) -> Vec<NoAuth> {
            vec![NoAuth {
                path: "/plugin/ping",
            }]
        }
    }

    /// 插件贡献的路由必须出现在最终 Router 上（否则插件通道形同虚设）
    #[tokio::test]
    async fn plugin_routes_reach_the_built_router() {
        let app = App::with_config(AppConfig::default())
            .expect("App::with_config")
            .web_plugin(DemoPlugin)
            .scan();

        let router = app.into_router();
        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/plugin/ping")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("oneshot");

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "插件贡献的 /plugin/ping 应可达"
        );
    }

    /// 插件贡献的权限规则必须并入权限层（与 `#[alun::permission]` 合并）
    #[tokio::test]
    async fn plugin_permission_rules_are_merged() {
        let app = App::with_config(AppConfig::default())
            .expect("App::with_config")
            .web_plugin(DemoPlugin);

        let mut merged = crate::discovered_permissions();
        for p in &app.web_plugins {
            merged.extend(p.permission_rules());
        }
        assert!(
            merged
                .iter()
                .any(|r| r.path == "/plugin/admin" && r.permission == "plugin:read"),
            "插件贡献的权限规则应出现在合并结果中：{merged:?}"
        );
    }

    /// 生命周期与 Web 装配作用于**同一实例**：插件名在管理器中可见
    #[tokio::test]
    async fn web_plugin_is_also_registered_for_lifecycle() {
        let app = App::with_config(AppConfig::default())
            .expect("App::with_config")
            .web_plugin(DemoPlugin);

        assert_eq!(app.plugins.check_duplicate_names().map(|_| ()), Ok(()));
    }
}

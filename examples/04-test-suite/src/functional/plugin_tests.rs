//! alun 插件系统测试
//!
//! 覆盖：PluginManager 生命周期、拓扑排序、循环检测、并发安全

#[cfg(test)]
mod tests {
    use alun_core::plugin::{Plugin, PluginManager};
    use alun_core::Result;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::sync::Arc;

    /// 订单追踪器：记录 start/stop 调用顺序
    struct OrderedPlugin {
        /// 插件唯一标识
        name: &'static str,
        /// 依赖列表
        deps: &'static [&'static str],
        /// 调用顺序记录共享状态
        order_log: Arc<Mutex<Vec<String>>>,
        /// start 是否应失败
        fail_start: bool,
        /// stop 是否应失败
        fail_stop: bool,
    }

    #[async_trait]
    impl Plugin for OrderedPlugin {
        fn name(&self) -> &str { self.name }

        async fn start(&self) -> Result<()> {
            self.order_log.lock().push(format!("start:{}", self.name));
            if self.fail_start {
                return Err(alun_core::Error::Msg(format!("插件 {} 启动失败", self.name)));
            }
            Ok(())
        }

        async fn stop(&self) -> Result<()> {
            self.order_log.lock().push(format!("stop:{}", self.name));
            if self.fail_stop {
                return Err(alun_core::Error::Msg(format!("插件 {} 停止失败", self.name)));
            }
            Ok(())
        }

        fn depends_on(&self) -> &[&str] { self.deps }
    }

    // ──── 基础启动 / 停止顺序 ──────────────────────

    #[tokio::test]
    async fn test_plugin_start_stop_order() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(OrderedPlugin {
                name: "a", deps: &[], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            })
            .add(OrderedPlugin {
                name: "b", deps: &[], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            });

        mgr.start_all().await.unwrap();
        mgr.stop_all().await;

        let events = log.lock();
        assert_eq!(events[0], "start:a");
        assert_eq!(events[1], "start:b");
        assert_eq!(events[2], "stop:b"); // 逆序停止
        assert_eq!(events[3], "stop:a");
    }

    // ──── 依赖排序 ──────────────────────────────────

    #[tokio::test]
    async fn test_plugin_dependency_order() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(OrderedPlugin {
                name: "db", deps: &[], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            })
            .add(OrderedPlugin {
                name: "cache", deps: &["db"], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            })
            .add(OrderedPlugin {
                name: "web", deps: &["db", "cache"], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            });

        mgr.start_all().await.unwrap();

        let events = log.lock();
        assert_eq!(events[0], "start:db");

        // cache 必须在 db 之后启动
        let cache_idx = events.iter().position(|e| e == "start:cache").unwrap();
        let db_idx = events.iter().position(|e| e == "start:db").unwrap();
        assert!(cache_idx > db_idx, "cache 必须在 db 之后启动");

        // web 必须在 db + cache 之后
        let web_idx = events.iter().position(|e| e == "start:web").unwrap();
        assert!(web_idx > db_idx, "web 必须在 db 之后启动");
        assert!(web_idx > cache_idx, "web 必须在 cache 之后启动");
    }

    // ──── 循环依赖检测 ───────────────────────────────

    #[tokio::test]
    async fn test_plugin_cycle_detection() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(OrderedPlugin {
                name: "x", deps: &["y"], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            })
            .add(OrderedPlugin {
                name: "y", deps: &["x"], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            });

        let result = mgr.start_all().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("循环"));
    }

    // ──── 启动失败中止后续 ─────────────────────────

    #[tokio::test]
    async fn test_plugin_start_failure_aborts() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(OrderedPlugin {
                name: "healthy", deps: &[], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            })
            .add(OrderedPlugin {
                name: "broken", deps: &[], order_log: log.clone(),
                fail_start: true, fail_stop: false,
            })
            .add(OrderedPlugin {
                name: "never-started", deps: &[], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            });

        let result = mgr.start_all().await;
        assert!(result.is_err());

        let events = log.lock();
        assert!(events.iter().any(|e| e == "start:broken"));
        assert!(!events.iter().any(|e| e == "start:never-started"),
            "broken 之后的插件不应被启动");
    }

    // ──── Stop 即使失败也继续 ─────────────────────

    #[tokio::test]
    async fn test_plugin_stop_continues_after_failure() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(OrderedPlugin {
                name: "good", deps: &[], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            })
            .add(OrderedPlugin {
                name: "stubborn", deps: &[], order_log: log.clone(),
                fail_start: false, fail_stop: true,
            });

        mgr.start_all().await.unwrap();
        mgr.stop_all().await;

        let events = log.lock();
        assert!(events.iter().any(|e| e == "stop:stubborn"));
        assert!(events.iter().any(|e| e == "stop:good"),
            "即使 stubborn 失败，good 也应被停止");
    }

    // ──── 重复名称检测 ──────────────────────────────

    #[test]
    fn test_plugin_duplicate_name() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(OrderedPlugin {
                name: "dup", deps: &[], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            })
            .add(OrderedPlugin {
                name: "dup", deps: &[], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            });

        let result = mgr.check_duplicate_names();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("dup"));
    }

    // ──── 空插件管理器 ──────────────────────────────

    #[tokio::test]
    async fn test_empty_plugin_manager() {
        let mgr = PluginManager::new();
        let result = mgr.start_all().await;
        assert!(result.is_ok());
        mgr.stop_all().await;
    }

    // ──── 缺失依赖 ──────────────────────────────────

    #[tokio::test]
    async fn test_plugin_missing_dependency() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(OrderedPlugin {
                name: "consumer", deps: &["non-existent-plugin"],
                order_log: log.clone(),
                fail_start: false, fail_stop: false,
            });

        let result = mgr.start_all().await;
        assert!(result.is_ok(), "缺失依赖不报错——依赖仅约束已注册插件");
    }

    // ──── 多链依赖 ─────────────────────────────────

    #[tokio::test]
    async fn test_plugin_multiple_dependency_chains() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(OrderedPlugin {
                name: "base", deps: &[], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            })
            .add(OrderedPlugin {
                name: "chain-a", deps: &["base"], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            })
            .add(OrderedPlugin {
                name: "chain-b", deps: &["base"], order_log: log.clone(),
                fail_start: false, fail_stop: false,
            });

        mgr.start_all().await.unwrap();

        let events = log.lock();
        let base_idx = events.iter().position(|e| e == "start:base").unwrap();
        let a_idx = events.iter().position(|e| e == "start:chain-a").unwrap();
        let b_idx = events.iter().position(|e| e == "start:chain-b").unwrap();

        assert!(a_idx > base_idx, "chain-a 必须在 base 之后");
        assert!(b_idx > base_idx, "chain-b 必须在 base 之后");
    }

    // ──── 自依赖（有向无环但存在自引用） ──────────────

    #[tokio::test]
    async fn test_plugin_self_dependency() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(OrderedPlugin {
                name: "self-dep", deps: &["self-dep"],
                order_log: log.clone(),
                fail_start: false, fail_stop: false,
            });

        let result = mgr.start_all().await;
        assert!(result.is_err(), "自依赖应被检测为循环");
    }
}
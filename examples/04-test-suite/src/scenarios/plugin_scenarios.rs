//! 插件集成场景测试
//!
//! 模拟真实应用中的插件生命周期管理

#[cfg(test)]
mod tests {
    use alun_core::plugin::{Plugin, PluginManager};
    use alun_core::Result;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::sync::Arc;

    struct LogPlugin {
        name: &'static str,
        deps: &'static [&'static str],
        log: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Plugin for LogPlugin {
        fn name(&self) -> &str { self.name }
        async fn start(&self) -> Result<()> {
            self.log.lock().push(format!("{} started", self.name));
            Ok(())
        }
        async fn stop(&self) -> Result<()> {
            self.log.lock().push(format!("{} stopped", self.name));
            Ok(())
        }
        fn depends_on(&self) -> &[&str] { self.deps }
    }

    // ──── 场景 1：数据库型应用插件栈 ───────────────

    #[tokio::test]
    async fn test_typical_app_plugin_stack() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(LogPlugin { name: "database", deps: &[], log: log.clone() })
            .add(LogPlugin { name: "cache", deps: &[], log: log.clone() })
            .add(LogPlugin { name: "kafka", deps: &[], log: log.clone() })
            .add(LogPlugin { name: "task_worker", deps: &["kafka"], log: log.clone() })
            .add(LogPlugin { name: "file_storage", deps: &[], log: log.clone() })
            .add(LogPlugin { name: "web_server", deps: &["database", "cache", "file_storage"], log: log.clone() });

        mgr.start_all().await.unwrap();

        let events = log.lock();
        let db_idx = events.iter().position(|e| e == "database started").unwrap();
        let cache_idx = events.iter().position(|e| e == "cache started").unwrap();
        let kafka_idx = events.iter().position(|e| e == "kafka started").unwrap();
        let task_idx = events.iter().position(|e| e == "task_worker started").unwrap();
        let web_idx = events.iter().position(|e| e == "web_server started").unwrap();

        assert!(task_idx > kafka_idx, "task_worker 必须在 kafka 之后");
        assert!(web_idx > db_idx, "web_server 必须在 database 之后");
        assert!(web_idx > cache_idx, "web_server 必须在 cache 之后");
    }

    // ──── 场景 2：带深度依赖的插件栈 ───────────────

    #[tokio::test]
    async fn test_deep_dependency_chain() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(LogPlugin { name: "l0", deps: &[], log: log.clone() })
            .add(LogPlugin { name: "l1", deps: &["l0"], log: log.clone() })
            .add(LogPlugin { name: "l2", deps: &["l1"], log: log.clone() })
            .add(LogPlugin { name: "l3", deps: &["l2"], log: log.clone() })
            .add(LogPlugin { name: "l4", deps: &["l3"], log: log.clone() });

        mgr.start_all().await.unwrap();

        let events = log.lock();
        assert!(events[0].contains("l0"));
        assert!(events[1].contains("l1"));
        assert!(events[2].contains("l2"));
        assert!(events[3].contains("l3"));
        assert!(events[4].contains("l4"));
    }

    // ──── 场景 3：多分支依赖图 ───────────────────

    #[tokio::test]
    async fn test_multi_branch_dependency_graph() {
        let log = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(LogPlugin { name: "root_a", deps: &[], log: log.clone() })
            .add(LogPlugin { name: "root_b", deps: &[], log: log.clone() })
            .add(LogPlugin { name: "branch_a1", deps: &["root_a"], log: log.clone() })
            .add(LogPlugin { name: "branch_a2", deps: &["root_a"], log: log.clone() })
            .add(LogPlugin { name: "branch_b1", deps: &["root_b"], log: log.clone() })
            .add(LogPlugin { name: "merge", deps: &["branch_a2", "branch_b1"], log: log.clone() });

        mgr.start_all().await.unwrap();

        let events = log.lock();
        let merge_idx = events.iter().position(|e| e == "merge started").unwrap();
        let a2_idx = events.iter().position(|e| e == "branch_a2 started").unwrap();
        let b1_idx = events.iter().position(|e| e == "branch_b1 started").unwrap();

        assert!(merge_idx > a2_idx, "merge 必须在 branch_a2 之后");
        assert!(merge_idx > b1_idx, "merge 必须在 branch_b1 之后");
    }
}
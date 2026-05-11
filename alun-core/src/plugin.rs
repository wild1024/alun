use async_trait::async_trait;
use crate::error::Result;

/// 插件生命周期：start/stop 对称
///
/// 极简设计——只定义 start / stop 两个生命周期方法。
/// Rust 版增强：异步化 + 依赖声明 + 拓扑排序启动。
///
/// # 示例
///
/// ```ignore
/// use alun_core::plugin::Plugin;
///
/// struct DbPlugin { pool: PgPool }
///
/// #[async_trait]
/// impl Plugin for DbPlugin {
///     fn name(&self) -> &str { "pg" }
///
///     async fn start(&self) -> Result<()> {
///         sqlx::query("SELECT 1").fetch_one(&self.pool).await?;
///         Ok(())
///     }
///
///     async fn stop(&self) -> Result<()> {
///         self.pool.close().await;
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Plugin: Send + Sync {
    /// 插件唯一名称（用于注册、日志、依赖解析）
    fn name(&self) -> &str;

    /// 启动插件：验证连接、初始化资源
    ///
    /// 在 `PluginManager::start_all()` 中被调用，按拓扑顺序依次执行。
    /// 若返回 `Err`，则启动流程中止，后续插件不会执行。
    async fn start(&self) -> Result<()>;

    /// 关闭插件：释放资源
    ///
    /// 在 `PluginManager::stop_all()` 中被调用，按逆序执行。
    /// 即使返回 `Err`，也会继续关闭其余插件（仅记录日志）。
    async fn stop(&self) -> Result<()>;

    /// 依赖的其他插件名称（用于拓扑排序，保证启动顺序）
    ///
    /// 默认返回空数组（无依赖）。
    fn depends_on(&self) -> &[&str] {
        &[]
    }
}

use std::collections::{HashMap, HashSet, VecDeque};
use tracing::{info, error};

/// 插件管理器：负责注册、拓扑排序启动、逆序关闭
///
/// # 示例
///
/// ```ignore
/// let mgr = PluginManager::new()
///     .add(db_plugin)
///     .add(cache_plugin);
/// mgr.start_all().await?;
/// // ... 运行应用 ...
/// mgr.stop_all().await;
/// ```
pub struct PluginManager {
    plugins: Vec<Box<dyn Plugin>>,
}

impl PluginManager {
    /// 创建空的插件管理器
    pub fn new() -> Self {
        Self { plugins: Vec::new() }
    }

    /// 手动注册插件（链式调用）
    ///
    /// # 示例
    ///
    /// ```ignore
    /// PluginManager::new()
    ///     .add(DbPlugin::new(db_config))
    ///     .add(CachePlugin::new(cache_config))
    /// ```
    pub fn add<P: Plugin + 'static>(mut self, plugin: P) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    /// 编译期自动发现的插件批量注册
    ///
    /// 与 `#[plugin]` 宏配合使用，将 linkme 收集到的插件批量加入管理器。
    pub fn add_discovered(mut self, plugins: Vec<Box<dyn Plugin>>) -> Self {
        self.plugins.extend(plugins);
        self
    }

    /// 拓扑排序后依次启动所有插件
    ///
    /// 若任一插件启动失败，流程立即中止并返回错误。
    pub async fn start_all(&self) -> Result<()> {
        let ordered = self.topological_sort()?;

        for plugin in ordered {
            info!("启动插件: {}", plugin.name());
            if let Err(e) = plugin.start().await {
                error!("插件 {} 启动失败: {}", plugin.name(), e);
                return Err(e);
            }
        }

        Ok(())
    }

    /// 逆序关闭所有插件（后启动的先关闭）
    ///
    /// 即使某个插件关闭失败，也会继续关闭其余插件（仅记录错误日志）。
    pub async fn stop_all(&self) {
        for plugin in self.plugins.iter().rev() {
            info!("停止插件: {}", plugin.name());
            if let Err(e) = plugin.stop().await {
                error!("插件 {} 停止异常: {}", plugin.name(), e);
            }
        }
    }

    /// Kahn 算法拓扑排序，检测循环依赖
    ///
    /// 返回按依赖顺序排列的插件引用。若存在循环依赖则返回 `Config` 错误。
    fn topological_sort(&self) -> Result<Vec<&Box<dyn Plugin>>> {
        let name_to_idx: HashMap<&str, usize> = self.plugins
            .iter()
            .enumerate()
            .map(|(i, p)| (p.name(), i))
            .collect();

        let count = self.plugins.len();
        let mut in_degree = vec![0usize; count];
        let mut adj = vec![Vec::new(); count];

        for (i, plugin) in self.plugins.iter().enumerate() {
            for dep in plugin.depends_on() {
                if let Some(&j) = name_to_idx.get(dep) {
                    adj[j].push(i);
                    in_degree[i] += 1;
                }
            }
        }

        let mut queue: VecDeque<usize> = (0..count)
            .filter(|&i| in_degree[i] == 0)
            .collect();

        let mut sorted = Vec::with_capacity(count);
        while let Some(u) = queue.pop_front() {
            sorted.push(&self.plugins[u]);
            for &v in &adj[u] {
                in_degree[v] -= 1;
                if in_degree[v] == 0 {
                    queue.push_back(v);
                }
            }
        }

        if sorted.len() != count {
            // 检测循环依赖
            let cycle: Vec<&str> = self.plugins.iter()
                .enumerate()
                .filter(|(i, _)| in_degree[*i] > 0)
                .map(|(_, p)| p.name())
                .collect();
            return Err(crate::error::Error::Config(
                format!("插件循环依赖: {:?}", cycle)
            ));
        }

        Ok(sorted)
    }

    /// 确保插件之间没有 name 冲突
    pub fn check_duplicate_names(&self) -> std::result::Result<(), String> {
        let mut seen = HashSet::new();
        for p in &self.plugins {
            if !seen.insert(p.name()) {
                return Err(format!("插件名重复: {}", p.name()));
            }
        }
        Ok(())
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::sync::Arc;

    struct TestPlugin {
        name: &'static str,
        deps: &'static [&'static str],
        order: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Plugin for TestPlugin {
        fn name(&self) -> &str { self.name }
        async fn start(&self) -> Result<()> {
            self.order.lock().push(format!("start:{}", self.name));
            Ok(())
        }
        async fn stop(&self) -> Result<()> {
            self.order.lock().push(format!("stop:{}", self.name));
            Ok(())
        }
        fn depends_on(&self) -> &[&str] { self.deps }
    }

    #[tokio::test]
    async fn test_topological_start() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(TestPlugin {
                name: "c", deps: &["a", "b"],
                order: order.clone(),
            })
            .add(TestPlugin {
                name: "b", deps: &["a"],
                order: order.clone(),
            })
            .add(TestPlugin {
                name: "a", deps: &[],
                order: order.clone(),
            });

        mgr.start_all().await.unwrap();

        let log = order.lock();
        assert_eq!(log[0], "start:a");
        assert_eq!(log[1], "start:b");
        assert_eq!(log[2], "start:c");
    }

    #[tokio::test]
    async fn test_cycle_detection() {
        let order = Arc::new(Mutex::new(Vec::new()));

        let mgr = PluginManager::new()
            .add(TestPlugin {
                name: "x", deps: &["y"],
                order: order.clone(),
            })
            .add(TestPlugin {
                name: "y", deps: &["x"],
                order: order.clone(),
            });

        let result = mgr.start_all().await;
        assert!(result.is_err());
    }
}

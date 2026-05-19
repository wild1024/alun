//! 存储后端注册中心
//!
//! 按 `backend_type` 管理所有 `StorageBackend` 实例及其配置。
//! 支持运行时手动注册和编译期自动发现（linkme 分布式切片）。
//!
//! 参考 alun_task::HandlerRegistry 的设计模式。

use std::collections::HashMap;
use std::sync::Arc;

use crate::backend::StorageBackend;
use crate::types::BackendConfig;

/// 存储后端注册中心
///
/// 维护 `backend_type → (backend_instance, config)` 的映射，
/// 提供按类型查找后端的能力。
#[derive(Clone)]
pub struct BackendRegistry {
    /// backend_type → (backend_instance, config)
    backends: HashMap<String, (Arc<dyn StorageBackend>, BackendConfig)>,
    /// 默认后端类型
    default_type: Option<String>,
}

impl BackendRegistry {
    /// 创建空的注册中心
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
            default_type: None,
        }
    }

    /// 注册一个存储后端及其配置
    ///
    /// 若已存在同 `backend_type` 的后端，将被覆盖。
    ///
    /// # 参数
    /// - `backend` - 实现了 StorageBackend 的实例
    /// - `config` - 后端配置
    pub fn register(
        &mut self,
        backend: impl StorageBackend + 'static,
        config: BackendConfig,
    ) -> &mut Self {
        let bt = backend.backend_type().to_string();
        self.backends.insert(bt, (Arc::new(backend), config));
        self
    }

    /// 从编译期自动发现的后端注册
    ///
    /// 读取 [`STORAGE_BACKENDS`](crate::types::STORAGE_BACKENDS) 分布式切片中的所有条目并注册。
    /// 可安全调用多次（后续条目覆盖先前的同名 backend_type）。
    pub fn from_discovered(&mut self) -> &mut Self {
        for entry in crate::types::STORAGE_BACKENDS {
            let backend = (entry.constructor_fn)();
            let config = (entry.config_fn)();
            self.backends.insert(
                entry.backend_type.to_string(),
                (Arc::from(backend), config),
            );
        }
        self
    }

    /// 设置默认后端类型
    ///
    /// 当调用 `write()` 等方法未指定后端类型时，使用此后端。
    pub fn with_default(&mut self, backend_type: &str) -> &mut Self {
        self.default_type = Some(backend_type.to_string());
        self
    }

    /// 按 backend_type 获取后端实例
    ///
    /// # 返回
    /// - `Some(&Arc<dyn StorageBackend>)` - 找到的后端
    /// - `None` - 未注册
    pub fn get(&self, backend_type: &str) -> Option<&Arc<dyn StorageBackend>> {
        self.backends.get(backend_type).map(|(b, _)| b)
    }

    /// 获取默认后端
    ///
    /// 优先返回 `default_type` 对应的后端，若未设置则返回注册中心中的第一个。
    pub fn default_backend(&self) -> Option<&Arc<dyn StorageBackend>> {
        self.default_type
            .as_ref()
            .and_then(|t| self.backends.get(t))
            .or_else(|| self.backends.values().next())
            .map(|(b, _)| b)
    }

    /// 按 backend_type 获取配置
    pub fn get_config(&self, backend_type: &str) -> Option<&BackendConfig> {
        self.backends.get(backend_type).map(|(_, c)| c)
    }

    /// 获取所有已注册的后端类型
    pub fn backend_types(&self) -> Vec<String> {
        self.backends.keys().cloned().collect()
    }

    /// 已注册的后端数量
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// 对所有已注册后端执行健康检查
    ///
    /// # 返回
    /// `Vec<(backend_type, health_check_result)>` 列表
    pub async fn health_check_all(&self) -> Vec<(String, Result<(), String>)> {
        let mut results = Vec::with_capacity(self.backends.len());
        for (bt, (backend, _)) in &self.backends {
            results.push((bt.clone(), backend.health_check().await));
        }
        results
    }
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::FileMeta;
    use async_trait::async_trait;

    struct TestBackend;
    #[async_trait]
    impl StorageBackend for TestBackend {
        fn backend_type(&self) -> &str { "test" }
        async fn write(&self, _name: &str, _data: &[u8]) -> Result<FileMeta, String> {
            Err("not implemented".into())
        }
        async fn read(&self, _path: &str) -> Result<Vec<u8>, String> {
            Err("not implemented".into())
        }
        async fn delete(&self, _path: &str) -> Result<(), String> { Ok(()) }
        async fn exists(&self, _path: &str) -> bool { false }
    }

    #[test]
    fn test_register_and_get() {
        let mut reg = BackendRegistry::new();
        reg.register(TestBackend, BackendConfig {
            backend_type: "test".into(),
            ..Default::default()
        });

        assert_eq!(reg.len(), 1);
        assert!(reg.get("test").is_some());
        assert!(reg.get("unknown").is_none());
    }

    #[test]
    fn test_default_backend() {
        let mut reg = BackendRegistry::new();
        reg.register(TestBackend, BackendConfig {
            backend_type: "test".into(),
            ..Default::default()
        });

        assert!(reg.default_backend().is_some());

        let reg2 = BackendRegistry::new();
        assert!(reg2.default_backend().is_none());
    }

    #[test]
    fn test_with_default() {
        let mut reg = BackendRegistry::new();
        reg.register(TestBackend, BackendConfig {
            backend_type: "test".into(),
            ..Default::default()
        });

        reg.with_default("test");
        assert!(reg.default_backend().is_some());

        reg.with_default("nonexistent");
        assert!(reg.default_backend().is_some()); // 回退到第一个
    }
}
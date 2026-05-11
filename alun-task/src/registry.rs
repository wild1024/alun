use std::collections::HashMap;
use std::sync::Arc;

use crate::handler::TaskHandler;
use crate::TaskConfig;

/// 处理器注册中心
///
/// 按 `task_type` 注册 `TaskHandler` 实例及其 `TaskConfig`。
/// 在应用启动时一次性完成注册，运行时只读访问。
#[derive(Clone)]
pub struct HandlerRegistry {
    /// task_type → (handler, config) 映射
    handlers: HashMap<i16, (Arc<dyn TaskHandler>, TaskConfig)>,
}

impl HandlerRegistry {
    /// 创建空的处理器注册中心
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// 注册一个处理器及其配置
    pub fn register(
        &mut self,
        handler: impl TaskHandler + 'static,
        config: TaskConfig,
    ) -> &mut Self {
        let task_type = handler.task_type();
        if task_type != config.task_type {
            tracing::warn!(
                "Handler task_type ({}) 与 config.task_type ({}) 不一致，将使用 config 中的值",
                task_type, config.task_type
            );
        }
        self.handlers
            .insert(config.task_type, (Arc::new(handler), config));
        self
    }

    /// 从编译期自动发现的 handler 注册（配合 `#[task_handler]` 宏）
    ///
    /// 读取 `TASK_HANDLERS` 分布式切片中的所有条目并注册。
    /// 可安全调用多次（后续条目覆盖先前的同名 task_type）。
    pub fn from_discovered(&mut self) -> &mut Self {
        for entry in crate::TASK_HANDLERS {
            let handler = (entry.handler_fn)();
            let config = (entry.config_fn)();
            self.handlers
                .insert(entry.task_type, (Arc::from(handler), config));
        }
        self
    }

    /// 按 task_type 获取处理器和配置
    pub fn get(&self, task_type: i16) -> Option<(Arc<dyn TaskHandler>, &TaskConfig)> {
        self.handlers
            .get(&task_type)
            .map(|(h, c)| (Arc::clone(h), c))
    }

    /// 获取所有已注册的 task_type
    pub fn task_types(&self) -> Vec<i16> {
        self.handlers.keys().copied().collect()
    }

    /// 获取配置
    pub fn get_config(&self, task_type: i16) -> Option<&TaskConfig> {
        self.handlers.get(&task_type).map(|(_, c)| c)
    }

    /// 已注册的 handler 数量
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

impl Default for HandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}
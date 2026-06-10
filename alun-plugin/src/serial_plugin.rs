//! 单号生成器插件：遵循 `TaskPlugin` 模式的薄封装
//!
//! 插件本身不持有任何数据库连接或 SQL 语句。
//! 所有持久化逻辑通过 `SerialGenerator` trait 委托给业务方实现。
//!
//! # 使用方式
//!
//! ## 方式一：内存后端（零配置）
//!
//! ```toml
//! plugins.enabled = ["serial"]
//!
//! [plugins.serial]
//! backend = "memory"
//!
//! [[plugins.serial.rules]]
//! key = "order"
//! format = "ORD{YYYY}{MM}{DD}{SEQ:8}"
//! cycle = "daily"
//! ```
//!
//! ## 方式二：自定义后端（参考 TaskPlugin）
//!
//! ```ignore
//! // 业务方实现 SerialGenerator trait
//! let my_backend = Arc::new(MyDbSerialBackend::new(db_pool));
//! let plugin = SerialPlugin::new(cfg.plugins.serial, my_backend);
//! app.plugin(plugin).scan().start().await;
//! ```
//!
//! ## 在 Handler 中使用
//!
//! ```ignore
//! #[alun::get("/api/serial/next/{rule_key}")]
//! async fn next_serial(
//!     Extension(plugin): Extension<SerialPlugin>,
//!     Path(rule_key): Path<String>,
//! ) -> Result<Res<String>, ApiError> {
//!     let no = plugin.generator().generate(&rule_key).await
//!         .map_err(|e| ApiError::internal(e.to_string()))?;
//!     Ok(Res::ok(no))
//! }
//! ```

use std::sync::Arc;
use async_trait::async_trait;
use alun_core::{Plugin, Result as CoreResult};
use alun_config::SerialConfig;
use alun_utils::{SerialGenerator, SerialRule, MemorySerialBackend, CyclePeriod, IncrementStrategy};
use tracing::info;

/// 单号生成器插件
///
/// 遵循 `TaskPlugin` 模式——插件是薄封装，不持有任何数据库连接或 SQL。
/// 持久化逻辑通过 `Arc<dyn SerialGenerator>` 委托给业务方。
///
/// # 自动配置模式
///
/// 若未通过 `new()` 传入自定义生成器，插件在 `start()` 时自动创建 Memory 后端。
/// 对于 Redis/PG 后端，需通过 `new(serial_config, arc_generator)` 注入。
pub struct SerialPlugin {
    /// 单号生成器配置
    config: SerialConfig,
    /// 实际使用的生成器实例（构造时或 start() 时设置）
    generator: parking_lot::RwLock<Option<Arc<dyn SerialGenerator>>>,
    /// 预注入的生成器（new() 传入时非空）
    preset_generator: Option<Arc<dyn SerialGenerator>>,
}

impl SerialPlugin {
    /// 使用自定义生成器创建插件（推荐——遵循 TaskPlugin 模式）
    ///
    /// # 示例
    ///
    /// ```ignore
    /// let backend = Arc::new(RedisSerialBackend::new(redis_conn));
    /// let plugin = SerialPlugin::new(cfg.plugins.serial, backend);
    /// ```
    pub fn new(config: SerialConfig, generator: Arc<dyn SerialGenerator>) -> Self {
        Self {
            preset_generator: Some(generator),
            generator: parking_lot::RwLock::new(None),
            config,
        }
    }

    /// 使用默认 memory 后端创建插件（零依赖自动配置）
    pub fn with_memory(config: SerialConfig) -> Self {
        Self {
            config,
            generator: parking_lot::RwLock::new(None),
            preset_generator: None,
        }
    }

    /// 获取生成器引用
    ///
    /// # Panics
    ///
    /// 插件未启动时调用会 panic。
    pub fn generator(&self) -> Arc<dyn SerialGenerator> {
        self.generator.read().clone()
            .expect("SerialPlugin 尚未启动，请确保插件已注册并启动")
    }

    /// 从 SerialRuleConfig 转换为 SerialRule
    fn convert_rule(rc: &alun_config::SerialRuleConfig) -> SerialRule {
        let cycle = match rc.cycle.as_str() {
            "daily" => CyclePeriod::Daily,
            "monthly" => CyclePeriod::Monthly,
            "yearly" => CyclePeriod::Yearly,
            _ => CyclePeriod::NoCycle,
        };
        let step = if let Some(max_str) = rc.step.strip_prefix("random:") {
            let max: u64 = max_str.parse().unwrap_or(1);
            IncrementStrategy::Random { max }
        } else {
            IncrementStrategy::Sequential
        };
        SerialRule {
            key: rc.key.clone(),
            format: rc.format.clone(),
            cycle,
            initial_value: rc.initial_value,
            step,
            is_enabled: true,
        }
    }
}

#[async_trait]
impl Plugin for SerialPlugin {
    fn name(&self) -> &str { "serial" }

    async fn start(&self) -> CoreResult<()> {
        // 使用预注入的生成器，否则默认 memory
        let gen = self.preset_generator.clone()
            .unwrap_or_else(|| {
                info!("单号生成器使用 memory 后端（默认）");
                Arc::new(MemorySerialBackend::new())
            });

        // 注册配置文件中定义的静态规则
        for rc in &self.config.rules {
            let rule = Self::convert_rule(rc);
            gen.register_rule(rule.clone()).await
                .map_err(|e| alun_core::Error::Msg(
                    format!("注册单号规则 '{}' 失败: {}", rule.key, e)
                ))?;
            info!("单号规则已注册: key={}, format={}", rule.key, rule.format);
        }

        *self.generator.write() = Some(gen);
        let rule_count = self.config.rules.len();
        info!("单号生成器就绪，后端={}, 规则数={}", self.config.backend, rule_count);
        Ok(())
    }

    async fn stop(&self) -> CoreResult<()> {
        info!("单号生成器插件已停止");
        Ok(())
    }
}
//! Alun 插件系统：通知、缓存、异步任务、定时任务
//!
//! 设计理念：
//! 1. 每个插件实现 `Plugin` trait，通过 `PluginManager` 管理
//! 2. 支持按需加载（通过配置 `plugins.enabled` 列表）
//! 3. 支持扩展第三方插件（实现 `Plugin` trait 即可注册）

pub mod notification;
pub mod cache_plugin;
pub mod async_task;
pub mod scheduler;
pub mod sid_plugin;
pub mod serial_plugin;

use alun_core::PluginManager;
use alun_config::AppConfig;
use tracing::info;
/// 根据配置创建所有启用的插件，返回 PluginManager
pub fn create_plugins_from_config(config: &AppConfig) -> PluginManager {
    let mut mgr = PluginManager::new();
    let plugins_cfg = &config.plugins;

    for name in &plugins_cfg.enabled {
        match name.as_str() {
            "notification" => {
                let p = notification::NotificationPlugin::from_config(
                    &plugins_cfg.notification,
                );
                mgr = mgr.add(p);
                info!("插件: notification 已注册");
            }
            "async-task" => {
                let p = async_task::AsyncTaskPlugin::new(
                    plugins_cfg.async_task.workers,
                );
                mgr = mgr.add(p);
                info!("插件: async-task 已注册");
            }
            "scheduler" => {
                let p = scheduler::SchedulerPlugin::new();
                mgr = mgr.add(p);
                info!("插件: scheduler 已注册");
            }
            "cache" => {
                let p = cache_plugin::CachePlugin::new(&config.app_name, &config.cache, &config.redis);
                mgr = mgr.add(p);
                info!("插件: cache 已注册");
            }
            "sid" => {
                let p = sid_plugin::SidPlugin::new();
                mgr = mgr.add(p);
                info!("插件: sid 已注册");
            }
            "serial" => {
                let p = serial_plugin::SerialPlugin::with_memory(
                    plugins_cfg.serial.clone(),
                );
                mgr = mgr.add(p);
                info!("插件: serial 已注册（memory 后端）");
            }
            _ => {
                tracing::warn!("未知插件: {}，跳过", name);
            }
        }
    }

    mgr
}

//! 短ID生成插件：基于 UUID v4/v7 的分布式业务ID
//!
//! 使用方式：`state.sid.generate_order_id()`

use async_trait::async_trait;
use alun_core::{Plugin, Result};
use alun_utils::Sid;

/// 短 ID 生成插件
///
/// 封装 `alun_utils::Sid`，提供 UUID v4/v7、TSID、短 ID 等多种 ID 生成策略。
pub struct SidPlugin;

impl SidPlugin {
    /// 创建短 ID 插件
    pub fn new() -> Self { Self }

    /// 生成 UUID v4（随机）
    pub fn uuid(&self) -> String { Sid::uuid() }
    /// 生成 16 字符短 ID（URL 安全）
    pub fn short(&self) -> String { Sid::short() }
    /// 生成 8 字符微型 ID
    pub fn tiny(&self) -> String { Sid::tiny() }
    /// 生成 UUID v7（时间有序）
    pub fn uuid7(&self) -> String { Sid::uuid7() }
    /// 生成 TSID（时间有序 64 位 ID）
    pub fn tsid(&self) -> String { Sid::tsid() }

    /// 带前缀的业务 ID，如 "ORD" + uuid7 → "ORD_0192331a..."
    pub fn biz_id(&self, prefix: &str) -> String {
        format!("{}_{}", prefix, Sid::uuid7())
    }
}

impl Default for SidPlugin {
    fn default() -> Self { Self }
}

#[async_trait]
impl Plugin for SidPlugin {
    fn name(&self) -> &str { "sid" }
    async fn start(&self) -> Result<()> {
        tracing::info!("短ID插件就绪");
        Ok(())
    }
    async fn stop(&self) -> Result<()> { Ok(()) }
}

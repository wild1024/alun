//! 短 ID 生成器：基于 UUID v4/v7 或雪花算法

use uuid::Uuid;
use std::time::{SystemTime, UNIX_EPOCH};

/// 短 ID 生成器
///
/// 提供 UUID v4/v7、雪花风格 TSID、短 ID、微型 ID 等分布式唯一标识生成。
pub struct Sid;

impl Sid {
    /// 32 位 UUID（无连字符）
    pub fn uuid() -> String { Uuid::new_v4().simple().to_string() }

    /// 短 ID（16 位 hex）
    pub fn short() -> String {
        let u = Uuid::new_v4();
        u.simple().to_string()[..16].to_string()
    }

    /// 极短 ID（8 位 hex）——用于临时标识
    pub fn tiny() -> String {
        let u = Uuid::new_v4();
        u.simple().to_string()[..8].to_string()
    }

    /// 基于时间戳的 20 位短 ID（毫秒级唯一）
    pub fn tsid() -> String {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let rand: u32 = rand::random();
        format!("{:013x}{:07x}", ts, rand & 0x0FFFFFFF)
    }

    /// UUID v7（时间有序，更适合数据库主键）
    pub fn uuid7() -> String {
        Uuid::now_v7().simple().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_short() { assert_eq!(Sid::short().len(), 16); }
    #[test]
    fn test_tiny() { assert_eq!(Sid::tiny().len(), 8); }
    #[test]
    fn test_uuid() { assert_eq!(Sid::uuid().len(), 32); }
}

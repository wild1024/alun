//! `stmt::compact` —— 无依赖的生态类型紧凑表示。
//!
//! 本模块定义了一组**不依赖任何第三方库**的日期、时间、小数、标识、
//! 网络地址的紧凑表示。它们用于 `stmt::Value`，由 `TypeMapper` 负责
//! 与生态类型（chrono / rust_decimal / uuid 等）互转。
//!
//! 这样做的好处是：`alun-db-core` 不需要引入任何生态类型即可完成
//! 值的表示与传递；生态类型的转换只在 `TypeMapper` 实现层出现。

use std::net::IpAddr;

// ==================== 日期 ====================

/// 日期：自 Unix epoch（1970-01-01）起的天数。
///
/// - 正值表示 epoch 之后
/// - 负值表示 epoch 之前
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompactDate(pub i32);

// ==================== 时间 ====================

/// 时间：自午夜（00:00:00）起的纳秒数。
///
/// ## 不变量
///
/// 值必须落在 `[0, 86_400_000_000_000)` 区间内。
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompactTime(pub u64);

impl CompactTime {
    /// 一天的纳秒总数。
    pub const NANOS_PER_DAY: u64 = 86_400_000_000_000;

    /// 校验值是否在合法范围内。
    pub fn is_valid(&self) -> bool {
        self.0 < Self::NANOS_PER_DAY
    }
}

// ==================== 时间戳 ====================

/// 无时区时间戳：自 Unix epoch 起的微秒数。
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompactTimestamp(pub i64);

/// 带时区时间戳：UTC 微秒 + 原始时区偏移。
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct CompactTimestampTz {
    /// UTC 时间点，自 Unix epoch 起的微秒数。
    pub utc_micros: i64,
    /// 原始时区相对 UTC 的偏移，单位秒。
    ///
    /// ## 不变量
    ///
    /// 值必须落在 `[-86_400, 86_400)` 区间内。
    pub offset_secs: i32,
}

impl CompactTimestampTz {
    /// 校验偏移量是否在合法范围内。
    pub fn is_valid(&self) -> bool {
        self.offset_secs > -86_400 && self.offset_secs < 86_400
    }
}

// ==================== 十进制 ====================

/// 十进制数：`mantissa × 10^(-scale)`。
///
/// ## 示例
///
/// ```text
/// CompactDecimal { mantissa: 12345, scale: 2 }  ==  123.45
/// CompactDecimal { mantissa: -6789, scale: 3 }  ==  -6.789
/// ```
///
/// ## 不变量
///
/// - `scale <= 38`
/// - `mantissa` 与 `scale` 的组合所表示的值的精度不超过 38 位
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct CompactDecimal {
    /// 尾数（含符号）。
    pub mantissa: i128,
    /// 标度，表示小数点左移的位数。
    pub scale: u32,
}

impl CompactDecimal {
    /// 允许的最大标度。
    pub const MAX_SCALE: u32 = 38;

    /// 校验标度是否合法。
    pub fn is_valid(&self) -> bool {
        self.scale <= Self::MAX_SCALE
    }
}

// ==================== 标识 ====================

/// 128 位标识。用于 UUID 和 ULID。
///
/// 使用字节数组而非 `u128`，避免字节序问题。
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CompactId(pub [u8; 16]);

// ==================== 网络 ====================

/// CIDR 网段：IP 地址 + 前缀长度。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompactCidr {
    /// 网络地址。
    pub addr: IpAddr,
    /// 前缀长度。IPv4 时范围 `[0, 32]`，IPv6 时范围 `[0, 128]`。
    pub prefix_len: u8,
}

impl CompactCidr {
    /// 校验前缀长度是否与地址族匹配。
    pub fn is_valid(&self) -> bool {
        match self.addr {
            IpAddr::V4(_) => self.prefix_len <= 32,
            IpAddr::V6(_) => self.prefix_len <= 128,
        }
    }
}

//! User-Agent 解析工具
//!
//! 从 User-Agent 字符串中提取设备类型、浏览器类型和操作系统信息。

/// User-Agent 解析结果
pub struct UaInfo {
    /// 设备类型：PC / MOBILE / TABLET / UNKNOWN
    pub device_type: String,
    /// 浏览器类型：Chrome / Firefox / Safari / Edge / Unknown
    pub browser_type: String,
    /// 操作系统类型：Windows / macOS / Linux / iOS / Android / Unknown
    pub os_type: String,
}

/// 解析 User-Agent 字符串，提取设备、浏览器和操作系统信息
///
/// # 参数
/// - `ua`: User-Agent 字符串
///
/// # 返回
/// 包含 device_type、browser_type、os_type 的 `UaInfo`
pub fn parse_user_agent(ua: &str) -> UaInfo {
    let ua_lower = ua.to_lowercase();

    let device_type = if ua_lower.contains("mobile") || ua_lower.contains("android") && !ua_lower.contains("tablet") {
        "MOBILE".to_string()
    } else if ua_lower.contains("ipad") || ua_lower.contains("tablet") {
        "TABLET".to_string()
    } else {
        "PC".to_string()
    };

    let browser_type = if ua_lower.contains("edg/") || ua_lower.contains("edge/") {
        "Edge".to_string()
    } else if ua_lower.contains("firefox/") {
        "Firefox".to_string()
    } else if ua_lower.contains("chrome/") && !ua_lower.contains("edg/") {
        "Chrome".to_string()
    } else if ua_lower.contains("safari/") && !ua_lower.contains("chrome/") {
        "Safari".to_string()
    } else {
        "Unknown".to_string()
    };

    let os_type = if ua_lower.contains("windows") {
        "Windows".to_string()
    } else if ua_lower.contains("mac os") || ua_lower.contains("macos") {
        "macOS".to_string()
    } else if ua_lower.contains("linux") && !ua_lower.contains("android") {
        "Linux".to_string()
    } else if ua_lower.contains("android") {
        "Android".to_string()
    } else if ua_lower.contains("ios") || ua_lower.contains("iphone") || ua_lower.contains("ipad") {
        "iOS".to_string()
    } else {
        "Unknown".to_string()
    };

    UaInfo { device_type, browser_type, os_type }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chrome_windows() {
        let info = parse_user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36");
        assert_eq!(info.device_type, "PC");
        assert_eq!(info.browser_type, "Chrome");
        assert_eq!(info.os_type, "Windows");
    }

    #[test]
    fn test_firefox_mac() {
        let info = parse_user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:109.0) Gecko/20100101 Firefox/121.0");
        assert_eq!(info.device_type, "PC");
        assert_eq!(info.browser_type, "Firefox");
        assert_eq!(info.os_type, "macOS");
    }

    #[test]
    fn test_mobile_chrome_android() {
        let info = parse_user_agent("Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36");
        assert_eq!(info.device_type, "MOBILE");
        assert_eq!(info.browser_type, "Chrome");
        assert_eq!(info.os_type, "Android");
    }

    #[test]
    fn test_safari_ios() {
        let info = parse_user_agent("Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1");
        assert_eq!(info.device_type, "MOBILE");
        assert_eq!(info.browser_type, "Safari");
        assert_eq!(info.os_type, "iOS");
    }
}
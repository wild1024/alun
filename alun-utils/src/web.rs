//! Web 解析工具：URL 解析、IP 提取、User-Agent 解析等

use std::net::SocketAddr;
use url::Url;

/// 检查是否为私有 IP（IPv4/IPv6）
///
/// IPv4 私有地址包括：私有地址（10/8, 172.16/12, 192.168/16）、环回地址（127/8）、
/// 链路本地地址（169.254/16）、未指定地址（0.0.0.0）、组播地址（224/4）
///
/// IPv6 私有地址包括：环回地址（::1）、未指定地址（::）、链路本地单播（fe80::/10）、
/// 唯一本地地址（fc00::/7）、组播地址（ff::/8）
///
/// # 参数
/// - `ip`: IP 地址字符串
///
/// # 返回
/// 若为私有 IP 返回 true，否则返回 false
pub fn is_private_ip(ip: &str) -> bool {
    if let Ok(addr) = ip.parse::<std::net::IpAddr>() {
        match addr {
            std::net::IpAddr::V4(ipv4) => {
                ipv4.is_private()
                    || ipv4.is_loopback()
                    || ipv4.is_link_local()
                    || ipv4.is_unspecified()
                    || ipv4.is_multicast()
            }
            std::net::IpAddr::V6(ipv6) => {
                ipv6.is_loopback()
                    || ipv6.is_unspecified()
                    || ipv6.is_unicast_link_local()
                    || ipv6.is_unique_local()
                    || ipv6.is_multicast()
            }
        }
    } else {
        false
    }
}

/// 从请求头提取客户端真实 IP
///
/// **安全说明（S-02）**：不得信任 `X-Forwarded-For` 的**首段**——它由客户端可控，
/// 攻击者轮换伪造值即可绕过登录/接口的 IP 限流。真实可信来源取决于部署拓扑：
/// 后端通常位于 nginx 之后，其中：
/// - `X-Real-IP`：被 nginx `proxy_set_header X-Real-IP $remote_addr` 覆写为真实客户端 IP（不可伪造）；
/// - `X-Forwarded-For` **末段**：nginx `$proxy_add_x_forwarded_for` 将真实客户端 IP 追加在末尾。
///
/// # 优先级
/// 1. `X-Real-IP`（可信代理覆写，最可信）
/// 2. `X-Forwarded-For` 末段（可信代理追加的真实客户端 IP）
/// 3. 连接对端 IP（app 直连场景，未经过代理）
/// 4. 回退至 "0.0.0.0"
///
/// # 参数
/// - `headers`: HTTP 请求头
/// - `connect_info`: 连接地址信息
///
/// # 返回
/// 客户端真实 IP 字符串，若无法获取则返回 "0.0.0.0"
pub fn extract_client_ip(headers: &http::HeaderMap, connect_info: &SocketAddr) -> String {
    fn is_valid_ip(s: &str) -> bool {
        s.trim().parse::<std::net::IpAddr>().is_ok()
    }

    // 1. X-Real-IP：nginx 覆写为真实客户端 IP（不可伪造）
    if let Some(ip) = headers
        .get("X-Real-IP")
        .and_then(|h| h.to_str().ok())
        .filter(|s| is_valid_ip(s))
    {
        return ip.trim().to_string();
    }

    // 2. X-Forwarded-For：取末段（nginx 追加的真实客户端 IP），避免信任攻击者可控的首段
    if let Some(ip) = headers
        .get("X-Forwarded-For")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.split(',').next_back())
        .filter(|s| is_valid_ip(s))
    {
        return ip.trim().to_string();
    }

    // 3. 连接对端 IP（直连场景）
    let connect_ip = connect_info.ip().to_string();
    if !is_private_ip(&connect_ip) {
        return connect_ip;
    }

    "0.0.0.0".to_string()
}

/// Web 解析工具
///
/// 提供 URL 解析、真实 IP 获取、私网 IP 判断、查询字符串构造等功能。
pub struct WebExt;

impl WebExt {
    /// 解析 URL 获取域名
    pub fn domain(url_str: &str) -> Option<String> {
        Url::parse(url_str)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_string()))
    }

    /// 解析 URL 获取路径
    pub fn path(url_str: &str) -> Option<String> {
        Url::parse(url_str).ok().map(|u| u.path().to_string())
    }

    /// 从请求头获取真实 IP（X-Forwarded-For 或 X-Real-IP）
    pub fn real_ip(headers: &[(String, String)], remote_addr: &str) -> String {
        for (key, val) in headers {
            if key.to_lowercase() == "x-forwarded-for" {
                return val.split(',').next().unwrap_or("").trim().to_string();
            }
            if key.to_lowercase() == "x-real-ip" {
                return val.clone();
            }
        }
        remote_addr
            .split(':')
            .next()
            .unwrap_or(remote_addr)
            .to_string()
    }
    /// 检查是否为私有 IP（委托给公共函数 `is_private_ip`）
    pub fn is_private_ip(ip: &str) -> bool {
        is_private_ip(ip)
    }
    /// 构建 URL 查询字符串
    pub fn build_query(params: &[(&str, &str)]) -> String {
        if params.is_empty() {
            return String::new();
        }
        let parts: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{}={}", urlencoding(k), urlencoding(v)))
            .collect();
        format!("?{}", parts.join("&"))
    }
}

fn urlencoding(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderMap, HeaderName, HeaderValue};
    use std::net::SocketAddr;

    /// 构造请求头
    fn header_maps(entries: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in entries {
            let name: HeaderName = k.parse().unwrap();
            let value: HeaderValue = v.parse().unwrap();
            map.insert(name, value);
        }
        map
    }

    /// 构造连接地址（任意端口，仅验证对端 IP 语义）
    fn connect(ip: &str) -> SocketAddr {
        format!("{}:54321", ip).parse().unwrap()
    }

    #[test]
    fn x_real_ip_takes_priority_over_xff_first_segment() {
        // 攻击者伪造 XFF 首段，但 X-Real-IP 为真实 IP 时应取 X-Real-IP
        let headers = header_maps(&[
            ("x-forwarded-for", "1.2.3.4, 203.0.113.9"),
            ("x-real-ip", "203.0.113.9"),
        ]);
        assert_eq!(
            extract_client_ip(&headers, &connect("10.0.0.5")),
            "203.0.113.9"
        );
    }

    #[test]
    fn xff_uses_last_segment_not_attacker_first_segment() {
        // 无 X-Real-IP 时，XFF 应取末段（nginx 追加的真实客户端 IP），而非攻击者可控的首段
        let headers = header_maps(&[("x-forwarded-for", "1.2.3.4, 198.51.100.7")]);
        assert_eq!(
            extract_client_ip(&headers, &connect("10.0.0.5")),
            "198.51.100.7"
        );
    }

    #[test]
    fn single_hop_xff_returns_itself() {
        // 单跳 XFF（无代理追加），取唯一值作为真实 IP
        let headers = header_maps(&[("x-forwarded-for", "203.0.113.42")]);
        assert_eq!(
            extract_client_ip(&headers, &connect("10.0.0.5")),
            "203.0.113.42"
        );
    }

    #[test]
    fn invalid_ip_in_headers_falls_back() {
        // 伪造非 IP 首段应忽略，回退到末段合法 IP
        let headers = header_maps(&[("x-forwarded-for", "not-an-ip, 198.51.100.7")]);
        assert_eq!(
            extract_client_ip(&headers, &connect("10.0.0.5")),
            "198.51.100.7"
        );
        // 全部非法时，直连为私网则回退 0.0.0.0
        let bad = header_maps(&[("x-forwarded-for", "evil")]);
        assert_eq!(extract_client_ip(&bad, &connect("10.0.0.5")), "0.0.0.0");
    }

    #[test]
    fn private_connect_ip_does_not_leak() {
        // 无任何可信头且直连为私网时，不暴露内部 IP，回退 0.0.0.0
        let headers = header_maps(&[]);
        assert_eq!(extract_client_ip(&headers, &connect("10.1.1.1")), "0.0.0.0");
    }
}

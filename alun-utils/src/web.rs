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
/// # 优先级
/// 1. CF-Connecting-IP（Cloudflare CDN）
/// 2. X-Forwarded-For（首个非私有 IP，AWS ELB/代理场景）
/// 3. X-Real-IP / X-Client-IP / X-Cluster-Client-IP
/// 4. 连接地址 IP
/// 5. 回退至 "0.0.0.0"
///
/// # 参数
/// - `headers`: HTTP 请求头
/// - `connect_info`: 连接地址信息
///
/// # 返回
/// 客户端真实 IP 字符串，若无法获取则返回 "0.0.0.0"
pub fn extract_client_ip(headers: &http::HeaderMap, connect_info: &SocketAddr) -> String {
    if let Some(cf_ip) = headers.get("CF-Connecting-IP").and_then(|h| h.to_str().ok()) {
        return cf_ip.to_string();
    }

    if let Some(x_forwarded_for) = headers.get("X-Forwarded-For").and_then(|h| h.to_str().ok()) {
        for ip in x_forwarded_for.split(',') {
            let trimmed = ip.trim();
            if !is_private_ip(trimmed) {
                return trimmed.to_string();
            }
        }
    }

    let other_headers = ["X-Real-IP", "X-Client-IP", "X-Cluster-Client-IP"];
    for header_name in &other_headers {
        if let Some(ip) = headers.get(*header_name).and_then(|h| h.to_str().ok()) {
            if !is_private_ip(ip) {
                return ip.to_string();
            }
        }
    }

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

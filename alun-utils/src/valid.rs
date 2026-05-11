//! 常规验证工具：邮箱、手机、URL、身份证等

use regex::Regex;
use std::sync::OnceLock;

/// 通用验证工具
///
/// 提供邮箱、手机号、URL、数字、字母数字、长度范围、
/// 密码强度、IPv4、Base64 等常用格式的布尔校验方法。
pub struct Valid;

impl Valid {
    fn email_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap())
    }

    fn mobile_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"^1[3-9]\d{9}$").unwrap())
    }

    fn url_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"^https?://[^\s/$.?#].[^\s]*$").unwrap())
    }

    /// 验证邮箱
    pub fn is_email(s: &str) -> bool { Self::email_re().is_match(s) }

    /// 验证手机号（中国大陆）
    pub fn is_mobile(s: &str) -> bool { Self::mobile_re().is_match(s) }

    /// 验证 URL
    pub fn is_url(s: &str) -> bool { Self::url_re().is_match(s) }

    /// 验证是否为纯数字
    pub fn is_digits(s: &str) -> bool { !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) }

    /// 验证是否为字母+数字组合
    pub fn is_alphanumeric(s: &str) -> bool { !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric()) }

    /// 验证字符串长度范围
    pub fn len_between(s: &str, min: usize, max: usize) -> bool {
        let len = s.chars().count();
        len >= min && len <= max
    }

    /// 验证密码强度（至少 8 位，包含大小写+数字）
    pub fn is_strong_password(s: &str) -> bool {
        if s.len() < 8 { return false; }
        let has_lower = s.chars().any(|c| c.is_ascii_lowercase());
        let has_upper = s.chars().any(|c| c.is_ascii_uppercase());
        let has_digit = s.chars().any(|c| c.is_ascii_digit());
        has_lower && has_upper && has_digit
    }

    /// 验证 IPv4
    pub fn is_ipv4(s: &str) -> bool {
        s.parse::<std::net::Ipv4Addr>().is_ok()
    }

    /// 验证 Base64
    pub fn is_base64(s: &str) -> bool {
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s).is_ok() && s.len() % 4 == 0
    }
}

/// 将 validator crate 的 ValidationErrors 转换为可读的错误消息
///
/// 需要启用 `validator-integration` feature
#[cfg(feature = "validator")]
impl Valid {
    pub fn format_validation_errors(errors: &validator::ValidationErrors) -> String {
        let mut messages = Vec::new();
        for (field, field_errors) in errors.field_errors() {
            for error in field_errors {
                if let Some(msg) = &error.message {
                    messages.push(format!("{}: {}", field, msg));
                } else {
                    messages.push(format!("{}: {}", field, error.code));
                }
            }
        }
        messages.join("; ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_email() { assert!(Valid::is_email("a@b.com")); assert!(!Valid::is_email("not-email")); }
    #[test]
    fn test_mobile() { assert!(Valid::is_mobile("13812345678")); assert!(!Valid::is_mobile("1234")); }
    #[test]
    fn test_password() { assert!(Valid::is_strong_password("Abcdefg1")); assert!(!Valid::is_strong_password("123456")); }
}

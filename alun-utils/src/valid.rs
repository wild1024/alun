//! 常规验证工具：邮箱、手机、URL、UUID、身份证等

use regex::Regex;
use std::sync::OnceLock;

/// 通用验证工具
///
/// 提供邮箱、手机号、URL、数字、字母数字、长度范围、
/// 密码强度、IPv4、Base64、UUID、身份证、日期、JSON 等常用格式的布尔校验方法。
pub struct Valid;

impl Valid {
    // ---- 正则缓存 ----

    fn email_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap())
    }

    fn mobile_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"^(1[3-9]\d{9})|(0\d{2,3}-\d{7,8})$").unwrap())
    }

    fn phone_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"^\+?[1-9]\d{1,14}$").unwrap())
    }

    fn url_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"^https?://(?:www\.)?[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b(?:[-a-zA-Z0-9()@:%_\+.~#?&//=]*)$").unwrap())
    }

    fn username_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9_.-]{3,50}$").unwrap())
    }

    fn color_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"^#[0-9A-Fa-f]{6}$").unwrap())
    }

    fn id_card_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"^\d{17}[\dXx]$").unwrap())
    }

    fn html_re() -> &'static Regex {
        static RE: OnceLock<Regex> = OnceLock::new();
        RE.get_or_init(|| Regex::new(r"<[^>]*>").unwrap())
    }

    // ---- 基础验证 ----

    /// 验证邮箱
    pub fn is_email(s: &str) -> bool { Self::email_re().is_match(s) }

    /// 验证手机号（中国大陆）或固话（如 010-12345678）
    pub fn is_mobile(s: &str) -> bool { Self::mobile_re().is_match(s) }

    /// 验证电话号码（E.164 格式）
    pub fn is_phone(s: &str) -> bool { Self::phone_re().is_match(s) }

    /// 验证 URL
    pub fn is_url(s: &str) -> bool { Self::url_re().is_match(s) }

    /// 验证用户名（3~50 位字母、数字、下划线、点、横线）
    pub fn is_username(s: &str) -> bool { Self::username_re().is_match(s) }

    /// 验证十六进制颜色（#RRGGBB）
    pub fn is_color(s: &str) -> bool { Self::color_re().is_match(s) }

    /// 验证是否为纯数字
    pub fn is_digits(s: &str) -> bool { !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) }

    /// 验证是否为字母+数字组合
    pub fn is_alphanumeric(s: &str) -> bool { !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric()) }

    /// 验证字符串长度范围
    pub fn len_between(s: &str, min: usize, max: usize) -> bool {
        let len = s.chars().count();
        len >= min && len <= max
    }

    // ---- 密码 ----

    /// 验证密码强度（至少 8 位，包含大小写+数字+特殊字符）
    pub fn is_strong_password(s: &str) -> bool {
        if s.len() < 8 { return false; }
        let has_lower = s.chars().any(|c| c.is_ascii_lowercase());
        let has_upper = s.chars().any(|c| c.is_ascii_uppercase());
        let has_digit = s.chars().any(|c| c.is_ascii_digit());
        let has_special = s.chars().any(|c| "!@#$%^&*()_+-=[]{}|;:,.<>?".contains(c));
        has_lower && has_upper && has_digit && has_special
    }

    // ---- 网络与编码 ----

    /// 验证 IPv4
    pub fn is_ipv4(s: &str) -> bool {
        s.parse::<std::net::Ipv4Addr>().is_ok()
    }

    /// 验证 Base64
    pub fn is_base64(s: &str) -> bool {
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s).is_ok() && s.len() % 4 == 0
    }

    // ---- UUID ----

    /// 验证 UUID（v1~v7 均支持）
    pub fn is_uuid(s: &str) -> bool {
        uuid::Uuid::parse_str(s).is_ok()
    }

    // ---- 身份证 ----

    /// 验证中国居民身份证号（含校验位）
    pub fn is_id_card(s: &str) -> bool {
        if !Self::id_card_re().is_match(s) {
            return false;
        }
        Self::validate_id_card_check_digit(s)
    }

    /// 身份证校验位验证
    fn validate_id_card_check_digit(id_card: &str) -> bool {
        if id_card.len() != 18 {
            return false;
        }
        let weights = [7, 9, 10, 5, 8, 4, 2, 1, 6, 3, 7, 9, 10, 5, 8, 4, 2];
        let check_digits = ['1', '0', 'X', '9', '8', '7', '6', '5', '4', '3', '2'];
        let mut sum = 0u32;
        for (i, ch) in id_card.chars().take(17).enumerate() {
            match ch.to_digit(10) {
                Some(digit) => sum += digit * weights[i],
                None => return false,
            }
        }
        let expected = check_digits[(sum % 11) as usize];
        let actual = id_card.chars().last().unwrap_or(' ');
        expected == actual.to_ascii_uppercase()
    }

    // ---- 日期 ----

    /// 验证日期格式（YYYY-MM-DD）
    pub fn is_date(s: &str) -> bool {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
    }

    /// 验证日期时间格式（ISO 8601 / RFC 3339）
    pub fn is_datetime(s: &str) -> bool {
        chrono::DateTime::parse_from_rfc3339(s).is_ok()
    }

    // ---- JSON ----

    /// 验证是否为合法 JSON 字符串
    pub fn is_json(s: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(s).is_ok()
    }

    // ---- HTML ----

    /// 检测字符串是否包含 HTML 标签
    pub fn has_html(s: &str) -> bool {
        Self::html_re().is_match(s)
    }

    /// 检测字符串是否不包含 HTML 标签
    pub fn is_html_free(s: &str) -> bool {
        !Self::has_html(s)
    }

    // ---- 文件 ----

    /// 验证文件扩展名是否在允许列表中
    pub fn is_file_extension(filename: &str, allowed: &[&str]) -> bool {
        if let Some(ext) = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
        {
            let ext_lower = ext.to_lowercase();
            return allowed.iter().any(|a| a.to_lowercase() == ext_lower);
        }
        false
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
    fn test_email() {
        assert!(Valid::is_email("a@b.com"));
        assert!(!Valid::is_email("not-email"));
    }

    #[test]
    fn test_mobile() {
        assert!(Valid::is_mobile("13812345678"));
        assert!(Valid::is_mobile("010-12345678"));
        assert!(Valid::is_mobile("021-87654321"));
        assert!(Valid::is_mobile("0755-12345678"));
        assert!(!Valid::is_mobile("1234"));
    }

    #[test]
    fn test_phone() {
        assert!(Valid::is_phone("+8613812345678"));
        assert!(!Valid::is_phone("not-phone"));
    }

    #[test]
    fn test_username() {
        assert!(Valid::is_username("john_doe"));
        assert!(Valid::is_username("user.name-123"));
        assert!(!Valid::is_username("ab")); // 太短
    }

    #[test]
    fn test_color() {
        assert!(Valid::is_color("#FF00AA"));
        assert!(Valid::is_color("#000000"));
        assert!(!Valid::is_color("FF00AA")); // 缺少 #
        assert!(!Valid::is_color("#FF00A")); // 位数不足
    }

    #[test]
    fn test_uuid() {
        assert!(Valid::is_uuid("550e8400-e29b-41d4-a716-446655440000"));
        assert!(Valid::is_uuid("00000000-0000-0000-0000-000000000000"));
        assert!(!Valid::is_uuid("not-a-uuid"));
    }

    #[test]
    fn test_id_card() {
        assert!(!Valid::is_id_card("1234")); // 太短
        assert!(!Valid::is_id_card("110101199003076790")); // 校验位错
    }

    #[test]
    fn test_date() {
        assert!(Valid::is_date("2024-01-01"));
        assert!(!Valid::is_date("2024-13-01")); // 月份错误
        assert!(!Valid::is_date("not-a-date"));
    }

    #[test]
    fn test_datetime() {
        assert!(Valid::is_datetime("2024-01-01T00:00:00Z"));
        assert!(Valid::is_datetime("2024-01-01T00:00:00+08:00"));
        assert!(!Valid::is_datetime("not-a-datetime"));
    }

    #[test]
    fn test_json() {
        assert!(Valid::is_json(r#"{"key": "value"}"#));
        assert!(Valid::is_json(r#"[1, 2, 3]"#));
        assert!(!Valid::is_json("not-json"));
    }

    #[test]
    fn test_html() {
        assert!(Valid::has_html("<div>hello</div>"));
        assert!(Valid::has_html("<script>alert(1)</script>"));
        assert!(!Valid::has_html("plain text"));
        assert!(Valid::is_html_free("plain text"));
    }

    #[test]
    fn test_file_extension() {
        let allowed = &["jpg", "png", "gif"];
        assert!(Valid::is_file_extension("photo.jpg", allowed));
        assert!(Valid::is_file_extension("photo.JPG", allowed));
        assert!(!Valid::is_file_extension("photo.pdf", allowed));
    }

    #[test]
    fn test_password() {
        assert!(Valid::is_strong_password("Abcdefg1!"));
        assert!(!Valid::is_strong_password("123456")); // 太短
        assert!(!Valid::is_strong_password("Abcdefg1")); // 缺少特殊字符
    }

    #[test]
    fn test_url() {
        assert!(Valid::is_url("https://example.com/path"));
        assert!(!Valid::is_url("not-a-url"));
    }

    #[test]
    fn test_ipv4() {
        assert!(Valid::is_ipv4("127.0.0.1"));
        assert!(!Valid::is_ipv4("not-ip"));
    }

    #[test]
    fn test_base64() {
        assert!(Valid::is_base64("SGVsbG8="));
        assert!(!Valid::is_base64("not-base64!"));
    }
}
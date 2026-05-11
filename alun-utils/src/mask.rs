//! 敏感信息脱敏工具

use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;

static MOBILE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^1[3-9]\d{9}$").unwrap());
static ID_CARD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{17}[\dXx]$").unwrap());
static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\w.\-]+@[\w.\-]+\.\w+$").unwrap());

/// 递归对 JSON 进行脱敏
///
/// 遍历 JSON 对象的所有字段，对匹配 `sensitive_fields` 的字段名、
/// 或字段值内容匹配手机号/身份证/邮箱格式的值进行脱敏。
///
/// # 参数
///
/// * `value` - 待脱敏的 JSON
/// * `sensitive_fields` - 敏感字段名列表
pub fn mask_json_value(value: Value, sensitive_fields: &[&str]) -> Value {
    match value {
        Value::Object(map) => mask_object(map, sensitive_fields),
        Value::Array(arr) => Value::Array(arr.into_iter().map(|v| mask_json_value(v, sensitive_fields)).collect()),
        other => mask_scalar_if_needed(other),
    }
}

fn mask_object(map: Map<String, Value>, sensitive_fields: &[&str]) -> Value {
    let mut masked = Map::new();
    for (key, val) in map {
        let is_sensitive = sensitive_fields.iter().any(|f| {
            f.eq_ignore_ascii_case(&key)
        });
        if is_sensitive {
            masked.insert(key, Value::String("****".into()));
        } else {
            masked.insert(key, mask_json_value(val, sensitive_fields));
        }
    }
    Value::Object(masked)
}

fn mask_scalar_if_needed(val: Value) -> Value {
    match &val {
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() { return val; }
            if s.len() >= 11 && (MOBILE_RE.is_match(s) || s.len() == 11 && s.starts_with('1')) {
                return Value::String(mask_mobile(s));
            }
            if s.len() >= 15 && s.len() <= 20 && contains_alpha_numeric(s) {
                if ID_CARD_RE.is_match(s) {
                    return Value::String(mask_id_card(s));
                }
            }
            if EMAIL_RE.is_match(s) {
                return Value::String(mask_email(s));
            }
            val
        }
        _ => val,
    }
}

fn contains_alpha_numeric(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_digit())
}

fn mask_mobile(s: &str) -> String {
    if s.len() < 7 { return s.to_string(); }
    format!("{}****{}", &s[..3], &s[s.len()-4..])
}

fn mask_id_card(s: &str) -> String {
    if s.len() < 8 { return s.to_string(); }
    format!("{}****{}", &s[..4], &s[s.len()-4..])
}

fn mask_email(s: &str) -> String {
    if let Some(at) = s.find('@') {
        let prefix = &s[..at];
        if prefix.len() <= 2 { format!("*{}", &s[at..]) }
        else { format!("{}***{}", &prefix[..1], &s[at..]) }
    } else { s.to_string() }
}

/// 敏感信息脱敏工具 —— 对手机号、邮箱、身份证、银行卡、人名等进行部分遮盖
///
/// # 示例
///
/// ```ignore
/// use alun_utils::Mask;
/// assert_eq!(Mask::mobile("13812345678"), "138****5678");
/// assert_eq!(Mask::email("alice@mail.com"), "a***@mail.com");
/// ```
pub struct Mask;

impl Mask {
    /// 手机号脱敏：保留前3后4位
    pub fn mobile(phone: &str) -> String { mask_mobile(phone) }
    /// 邮箱脱敏：保留首字符和域名部分
    pub fn email(email: &str) -> String { mask_email(email) }
    /// 身份证脱敏：保留前4后4位
    pub fn id_card(id: &str) -> String { mask_id_card(id) }
    /// 银行卡脱敏：保留前4后4位，中间用 ` **** ` 分隔
    pub fn bank_card(card: &str) -> String {
        if card.len() < 8 { return card.to_string(); }
        format!("{} **** {}", &card[..4], &card[card.len()-4..])
    }
    /// 姓名脱敏：保留首字符，其余用 `*` 代替
    pub fn name(name: &str) -> String {
        let chars: Vec<char> = name.chars().collect();
        if chars.len() <= 1 { return name.to_string(); }
        let mut result = String::new();
        result.push(chars[0]);
        for _ in 1..chars.len() { result.push('*'); }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_mobile() { assert_eq!(Mask::mobile("13812345678"), "138****5678"); }
    #[test]
    fn test_email() { assert_eq!(Mask::email("alice@mail.com"), "a***@mail.com"); }
}

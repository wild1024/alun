//! 字符串工具：驼峰/蛇形互转、截断、判空、随机串等

/// 字符串扩展 trait（为 `&str` 添加实用方法）
///
/// 导入后可对任何字符串切片调用驼峰/蛇形互转、截断、判空、随机串等方法。
pub trait StrExt {
    /// 是否为空白（仅含空格/制表符/换行符）
    fn is_blank(&self) -> bool;
    /// 蛇形命名 → 驼峰命名（如 `user_name` → `userName`）
    fn to_camel(&self) -> String;
    /// 驼峰命名 → 蛇形命名（如 `UserName` → `user_name`）
    fn to_snake(&self) -> String;
    /// 按字符数截断（超出末尾补 `...`）
    fn truncate(&self, max: usize) -> String;
    /// 生成指定长度的随机字母数字串
    fn random(len: usize) -> String;
    /// 非空白字符（`is_blank` 的取反）
    fn has_text(&self) -> bool { !self.is_blank() }
}

impl StrExt for str {
    fn is_blank(&self) -> bool { self.trim().is_empty() }

    fn to_camel(&self) -> String {
        self.split('_')
            .enumerate()
            .map(|(i, w)| {
                if i == 0 { w.to_lowercase() }
                else { let mut c = w.chars(); c.next().map(|x| x.to_uppercase().chain(c).collect()).unwrap_or_default() }
            })
            .collect()
    }

    fn to_snake(&self) -> String {
        let mut result = String::with_capacity(self.len() + 4);
        for (i, ch) in self.chars().enumerate() {
            if ch.is_uppercase() && i > 0 {
                result.push('_');
            }
            result.push(ch.to_ascii_lowercase());
        }
        result
    }

    fn truncate(&self, max: usize) -> String {
        if self.len() <= max { self.to_string() }
        else { format!("{}...", &self[..max]) }
    }

    fn random(len: usize) -> String {
        use rand::Rng;
        const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let mut rng = rand::thread_rng();
        (0..len).map(|_| CHARSET[rng.gen_range(0..CHARSET.len())] as char).collect()
    }
}

/// 清理文件名 —— 将非字母/数字/点/横线/下划线的字符替换为 `_`
pub fn sanitize_filename(filename: &str) -> String {
    use regex::Regex;
    let re = Regex::new(r"[^a-zA-Z0-9.\-_]").unwrap();
    re.replace_all(filename, "_").to_string()
}

/// 解析 JSON 字符串为 `serde_json::Value`
pub fn parse_json_value(value: &str) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::from_str(value)
}

/// 格式化文件大小（字节 → 人类可读）
///
/// # 示例
///
/// ```
/// assert_eq!(alun_utils::str::format_file_size(0), "0 B");
/// assert_eq!(alun_utils::str::format_file_size(1024), "1.00 KB");
/// assert_eq!(alun_utils::str::format_file_size(1_500_000), "1.43 MB");
/// ```
pub fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

    if bytes == 0 {
        return "0 B".to_string();
    }

    let i = (bytes as f64).log(1024.0).floor() as i32;
    let size = bytes as f64 / 1024_f64.powi(i);
    let unit = UNITS.get(i as usize).unwrap_or(&"B");

    format!("{:.2} {}", size, unit)
}

/// 清理字符串参数 —— 去除前后空格
pub fn clean_string_param(s: &str) -> String {
    s.trim().to_string()
}

/// 清理邮箱参数 —— 去除前后空格，转为小写
pub fn clean_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// 清理密码参数 —— 只去除前后空格，保留中间空格
pub fn clean_password(password: &str) -> String {
    password.trim().to_string()
}

/// 用户输入清理器 —— 提供注册/登录请求参数的规范化清理
pub struct InputCleaner;

impl InputCleaner {
    /// 清理注册请求：邮箱小写去空格、密码去空格、昵称去空格
    ///
    /// 返回 `(email, password, nickname)` 三元组。
    pub fn clean_register_input(
        email: &str,
        password: &str,
        nickname: &str,
    ) -> (String, String, String) {
        let email = clean_email(email);
        let password = clean_password(password);
        let nickname = clean_string_param(nickname);
        (email, password, nickname)
    }

    /// 清理登录请求：邮箱小写去空格、密码去空格
    ///
    /// 返回 `(email, password)` 二元组。
    pub fn clean_login_input(email: &str, password: &str) -> (String, String) {
        let email = clean_email(email);
        let password = clean_password(password);
        (email, password)
    }
}

/// 生成邀请码 —— 12 位随机字母数字串
pub fn generate_invite_code() -> String {
    use rand::{Rng, distributions::Alphanumeric};
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(12)
        .map(char::from)
        .collect()
}

/// 生成指定位数的不含数字 `0` 的随机数字串
///
/// # 参数
///
/// * `n` - 位数（若为 0 则返回空字符串）
///
/// # 示例
///
/// ```
/// let s = alun_utils::str::generate_random_digits(6);
/// assert_eq!(s.len(), 6);
/// assert!(!s.chars().any(|c| c == '0'));
/// ```
pub fn generate_random_digits(n: usize) -> String {
    if n == 0 {
        return String::new();
    }
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..n)
        .map(|_| (rng.gen_range(1..=9) + b'0') as char)
        .collect()
}

/// 生成由大小写字母和数字（不含 `0` 和 `O`/`I`/`l` 等易混淆字符）组成的随机字符串
///
/// # 参数
///
/// * `length` - 字符串长度（若为 0 则返回空字符串）
///
/// # 示例
///
/// ```
/// let s = alun_utils::str::generate_random_alphanum(8);
/// assert_eq!(s.len(), 8);
/// assert!(!s.chars().any(|c| c == '0' || c == 'O' || c == 'I' || c == 'l'));
/// ```
pub fn generate_random_alphanum(length: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ\
                              abcdefghjkmnpqrstuvwxyz\
                              123456789";
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut result = String::with_capacity(length);
    for _ in 0..length {
        let idx = rng.gen_range(0..CHARSET.len());
        result.push(CHARSET[idx] as char);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camel() { assert_eq!("user_name".to_camel(), "userName"); }
    #[test]
    fn test_snake() { assert_eq!("UserName".to_snake(), "user_name"); }
    #[test]
    fn test_blank() { assert!("  ".is_blank()); assert!(!"abc".is_blank()); }
}

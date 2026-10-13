//! 通用迁移版本号。

use std::cmp::Ordering;
use std::fmt;

/// 通用迁移版本号（Flyway 风格）。
///
/// 版本号是**不透明字符串**：支持 `1` / `1.2` / `20250101_120000` 等形态，
/// 不限定为整数。排序按**数值分量**而非字典序，因此 `10` 排在 `9` 之后
/// （字典序下 `"10" < "9"`，正是需要避免的陷阱）。
///
/// 构造时对纯数字分量做前导零归一（`01` → `1`），使 `V01__x` 与 `V1__x`
/// 视为同一版本，避免同一版本在记录表里出现两种写法。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version(String);

impl Version {
    /// 由字符串构造版本号，并按数值分量归一化前导零。
    ///
    /// # 参数
    ///
    /// - `raw`：原始版本串（如 `"01"`、`"1.2"`、`"20250101_120000"`）。
    pub fn new(raw: impl AsRef<str>) -> Self {
        Version(normalize_version(raw.as_ref()))
    }

    /// 以字符串切片形式返回归一化后的版本号。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 版本号是否为空（空版本非法，由调用方拒绝）。
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for Version {
    fn from(value: &str) -> Self {
        Version::new(value)
    }
}

impl From<String> for Version {
    fn from(value: String) -> Self {
        Version::new(value)
    }
}

impl From<i64> for Version {
    fn from(value: i64) -> Self {
        Version::new(value.to_string())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_versions(&self.0, &other.0)
    }
}

/// 版本号的一个排序分量。
///
/// 数字连续段作为数值分量（`10` > `9`），字母连续段作为文本分量；
/// `.`、`_`、`-` 等分隔符不参与比较，仅用于切分。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Component {
    /// 数字连续段。
    Num(u128),
    /// 字母连续段。
    Text(String),
}

/// 按数值分量比较两个版本串。
fn compare_versions(left: &str, right: &str) -> Ordering {
    let left = components(left);
    let right = components(right);
    for (x, y) in left.iter().zip(right.iter()) {
        let ord = match (x, y) {
            (Component::Num(x), Component::Num(y)) => x.cmp(y),
            (Component::Text(x), Component::Text(y)) => x.cmp(y),
            // 数字分量与文本分量不可比，按「数字在前」给出确定的偏序，
            // 保证 `Ord` 全序成立（不会因不可比而破坏 BTreeMap 语义）。
            (Component::Num(_), Component::Text(_)) => Ordering::Less,
            (Component::Text(_), Component::Num(_)) => Ordering::Greater,
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    left.len().cmp(&right.len())
}

/// 把版本串切分为排序分量序列。
fn components(version: &str) -> Vec<Component> {
    let mut out = Vec::new();
    let mut digits = String::new();
    let mut text = String::new();

    /// 把已累积的数字段与文本段依次落入结果。
    fn flush(digits: &mut String, text: &mut String, out: &mut Vec<Component>) {
        if !digits.is_empty() {
            // 超长数字段（>38 位）溢出时饱和到最大值，保证不 panic
            out.push(Component::Num(digits.parse().unwrap_or(u128::MAX)));
            digits.clear();
        }
        if !text.is_empty() {
            out.push(Component::Text(std::mem::take(text)));
        }
    }

    for ch in version.chars() {
        if ch.is_ascii_digit() {
            if !text.is_empty() {
                out.push(Component::Text(std::mem::take(&mut text)));
            }
            digits.push(ch);
        } else if ch.is_alphabetic() {
            if !digits.is_empty() {
                out.push(Component::Num(digits.parse().unwrap_or(u128::MAX)));
                digits.clear();
            }
            text.push(ch);
        } else {
            flush(&mut digits, &mut text, &mut out);
        }
    }
    flush(&mut digits, &mut text, &mut out);
    out
}

/// 归一化版本串：剥掉每个数字连续段的前导零（`0` 本身保留）。
fn normalize_version(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut digits = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        if !digits.is_empty() {
            out.push_str(&strip_leading_zeros(&digits));
            digits.clear();
        }
        out.push(ch);
    }
    if !digits.is_empty() {
        out.push_str(&strip_leading_zeros(&digits));
    }
    out
}

/// 剥掉一段纯数字的前导零，全零时返回 `"0"`。
fn strip_leading_zeros(digits: &str) -> String {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 前导零归一：`01` 与 `1` 是同一版本。
    #[test]
    fn normalizes_leading_zeros() {
        assert_eq!(Version::new("01"), Version::new("1"));
        assert_eq!(Version::new("V0002").as_str(), "V2");
        assert_eq!(Version::new("0").as_str(), "0");
        assert_eq!(Version::new("000").as_str(), "0");
        assert_eq!(Version::new("20250101_120000").as_str(), "20250101_120000");
    }

    /// 数值分量排序：`10` 必须排在 `9` 之后（而非字典序）。
    #[test]
    fn orders_numerically_not_lexicographically() {
        assert!(Version::new("10") > Version::new("9"));
        let mut versions = [
            Version::new("10"),
            Version::new("9"),
            Version::new("100"),
            Version::new("1"),
        ];
        versions.sort();
        let ordered: Vec<&str> = versions.iter().map(Version::as_str).collect();
        assert_eq!(ordered, vec!["1", "9", "10", "100"]);
    }

    /// 多分量版本（点分 / 下划线分）逐分量比较。
    #[test]
    fn orders_multi_component_versions() {
        assert!(Version::new("1.10") > Version::new("1.9"));
        assert!(Version::new("1.2") > Version::new("1"));
        assert!(Version::new("20250101_120000") > Version::new("20250101_090000"));
    }
}

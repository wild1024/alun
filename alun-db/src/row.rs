/// Row —— alun 的数据载体
///
/// 设计要点：
/// - `table` + `primary_key` → 实体元数据内聚
/// - `data` → 字段值（HashMap，支持 serde 零成本序列化）
/// - `changes` → 变更追踪（Set，用于 UPDATE SET 精确字段）
///
/// 特性：
/// - serde 序列化支持
/// - Builder 模式设置表名和主键
/// - `id(value)` 快捷设主键值
use serde::{Serialize, Deserialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use crate::IdKind;

/// Row：携带表名、主键名、数据和变更追踪
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Row {
    /// 表名
    #[serde(skip)]
    pub table: Option<String>,

    /// 主键字段名列表（默认 `["id"]`）
    #[serde(skip)]
    pub primary_keys: Vec<String>,

    /// 数据字段
    pub data: HashMap<String, Value>,

    /// 变更追踪：记录哪些字段被修改过
    #[serde(skip)]
    pub changes: HashSet<String>,
}

impl Row {
    /// 创建指定表的 Row
    ///
    /// ```ignore
    /// let user = Row::table("user");
    /// ```
    pub fn table(name: impl Into<String>) -> Self {
        Self {
            table: Some(name.into()),
            primary_keys: vec!["id".to_string()],
            ..Default::default()
        }
    }

    /// 设置主键字段
    ///
    /// ```ignore
    /// Row::table("user").primary_key("uid")
    /// ```
    pub fn primary_key(mut self, key: impl Into<String>) -> Self {
        self.primary_keys = vec![key.into()];
        self
    }

    /// 设置复合主键
    ///
    /// # 示例
    ///
    /// ```ignore
    /// Row::table("order_item").primary_keys(&["order_id", "item_id"])
    /// ```
    pub fn primary_keys(mut self, keys: &[&str]) -> Self {
        self.primary_keys = keys.iter().map(|k| k.to_string()).collect();
        self
    }

    /// 快捷设置主键值（取 `primary_keys[0]`）
    ///
    /// ```ignore
    /// Row::table("user").id(123)
    /// ```
    pub fn id<V: Into<Value>>(mut self, id: V) -> Self {
        let pk = self.primary_keys.first()
            .cloned()
            .unwrap_or_else(|| "id".to_string());
        self.set(&pk, id);
        self
    }

    /// 设置字段值
    ///
    /// ```ignore
    /// row.set("name", "Alice")
    ///     .set("age", 25);
    /// ```
    pub fn set<V: Into<Value>>(&mut self, key: &str, value: V) -> &mut Self {
        self.data.insert(key.to_string(), value.into());
        self.changes.insert(key.to_string());
        self
    }

    /// 获取字段值（`serde_json::Value`），不存在返回 `None`
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    /// 获取字段值并反序列化
    ///
    /// ```ignore
    /// let name: String = row.get_as("name").unwrap();
    /// ```
    pub fn get_as<'a, T: Deserialize<'a>>(&'a self, key: &str) -> Option<T> {
        self.data.get(key)
            .and_then(|v| T::deserialize(v).ok())
    }

    /// 获取主键值（取 `primary_keys[0]` 对应的字段值）
    pub fn get_id(&self) -> Option<&Value> {
        self.primary_keys
            .first()
            .and_then(|pk| self.data.get(pk))
    }

    /// 标记所有字段为已修改
    pub fn mark_all_changed(&mut self) {
        self.changes = self.data.keys().cloned().collect();
    }

    /// 判断某个字段是否存在（含值为 null）
    pub fn has(&self, key: &str) -> bool {
        self.data.contains_key(key)
    }

    /// 转为 JSON 字符串（含 `table` 和 `data`）
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }

    /// 从 JSON 字符串反序列化
    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }

    /// 自动检测主键值的 `IdKind`（UUID / i64 / 字符串等）
    pub fn detect_id_kind(&self) -> Option<IdKind> {
        self.get_id().map(|v| IdKind::detect(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_row_builder() {
        let mut row = Row::table("user")
            .primary_key("uid")
            .id("u-123");

        row.set("name", "Alice")
           .set("age", 25);

        assert_eq!(row.table.as_deref(), Some("user"));
        assert_eq!(row.primary_keys, vec!["uid"]);
        assert_eq!(row.get("name").and_then(|v| v.as_str()), Some("Alice"));
        assert!(row.changes.contains("name"));
        assert!(row.changes.contains("age"));
    }

    #[test]
    fn test_get_as() {
        let mut row = Row::table("user").id(42);
        row.set("name", "Bob");

        let name: Option<String> = row.get_as("name");
        assert_eq!(name, Some("Bob".into()));

        let id: Option<i64> = row.get_as("id");
        assert_eq!(id, Some(42));
    }
}

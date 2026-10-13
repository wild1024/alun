//! `Row` —— 动态行视图。

use std::sync::Arc;

use crate::error::{DbError, Result};
use crate::stmt::Value;
use crate::traits::TypeMapper;

/// 一行数据的动态表示。
///
/// 字段顺序按 SELECT 列顺序保留。字段名唯一。
///
/// ## 设计决策
///
/// 使用 `Vec<(Arc<str>, Value)>` 而非 `HashMap` 的原因：
///
/// 1. 行字段数通常 < 50，线性查找比哈希查找更快
/// 2. 保留字段顺序，便于调试和 JSON 序列化
/// 3. 内存占用更小（`HashMap` 的哈希表开销在字段少时不划算）
///
/// 字段名用 `Arc<str>` 而非 `String`：同一 SQL 的列名恒定，驱动可借列元数据
/// 缓存跨行共享同一份名字（见 [`Row::push_shared`]），使「每行 N 次列名字符串
/// 分配」降为「每行 N 次引用计数自增」——分页 / 全表扫描时这是行内存的大头。
#[derive(Debug, Clone, Default)]
pub struct Row {
    /// 字段列表，保持插入顺序。字段名唯一。
    entries: Vec<(Arc<str>, Value)>,
    /// 实体表名。仅动态写操作（`Db::insert_row` 等）依赖，查询结果默认无。
    table: Option<String>,
    /// 主键字段名。动态写操作默认使用 `id`。
    primary_key: Option<String>,
}

impl Row {
    /// 创建空行。
    pub fn new() -> Self {
        Row {
            entries: Vec::new(),
            table: None,
            primary_key: None,
        }
    }

    /// 预分配指定容量的空行。
    pub fn with_capacity(cap: usize) -> Self {
        Row {
            entries: Vec::with_capacity(cap),
            table: None,
            primary_key: None,
        }
    }

    /// 标记实体元数据（表名与主键字段名），供动态写操作使用。
    ///
    /// # 参数
    ///
    /// - `table`：目标表名
    /// - `primary_key`：主键字段名（默认约定为 `id`）
    pub fn with_entity(mut self, table: impl Into<String>, primary_key: impl Into<String>) -> Self {
        self.table = Some(table.into());
        self.primary_key = Some(primary_key.into());
        self
    }

    /// 设置表名。返回 `&mut self` 便于链式调用。
    pub fn set_table(&mut self, table: impl Into<String>) -> &mut Self {
        self.table = Some(table.into());
        self
    }

    /// 设置主键字段名。返回 `&mut self` 便于链式调用。
    pub fn set_primary_key(&mut self, primary_key: impl Into<String>) -> &mut Self {
        self.primary_key = Some(primary_key.into());
        self
    }

    /// 读取表名。
    pub fn table(&self) -> Option<&str> {
        self.table.as_deref()
    }

    /// 读取主键字段名（未设置时默认 `id`）。
    pub fn primary_key(&self) -> &str {
        self.primary_key.as_deref().unwrap_or("id")
    }

    /// 读取主键值。
    pub fn pk_value(&self) -> Option<&Value> {
        self.get(self.primary_key())
    }

    /// 插入或覆盖字段。
    ///
    /// 若 `name` 已存在，则覆盖旧值；否则追加到末尾。
    ///
    /// 参数为 `impl AsRef<str>` 而非 `impl Into<String>`：`Row` 内部按 `Arc<str>`
    /// 存名字，先 `Into<String>` 再 `Arc::from` 会多出一次中间字符串分配。
    pub fn insert(&mut self, name: impl AsRef<str>, value: impl Into<Value>) {
        let name = name.as_ref();
        let value = value.into();
        if let Some(slot) = self.entries.iter_mut().find(|(n, _)| n.as_ref() == name) {
            slot.1 = value;
        } else {
            self.entries.push((Arc::from(name), value));
        }
    }

    /// 追加字段：复用调用方已有的列名 `Arc`，且**不做重名检查**。
    ///
    /// 供驱动读取结果集时使用——同一 SQL 的列名由列元数据缓存持有，逐行
    /// `Arc::clone` 即可共享；列名唯一性与顺序由结果集列序天然保证，故省去
    /// [`insert`](Self::insert) 的 O(列数) 重名线性查找。
    pub fn push_shared(&mut self, name: Arc<str>, value: impl Into<Value>) {
        self.entries.push((name, value.into()));
    }

    /// 按字段名读取原始值。
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(n, _)| n.as_ref() == name)
            .map(|(_, v)| v)
    }

    /// 按字段顺序索引读取原始值。
    ///
    /// `index` 超出范围（含列数少于模型字段数时的多余读取）返回 `None`。
    pub fn get_at(&self, index: usize) -> Option<&Value> {
        self.entries.get(index).map(|(_, v)| v)
    }

    /// 按字段顺序索引读取字段名。
    ///
    /// `index` 超出范围返回 `None`。
    pub fn name_at(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(|(n, _)| &**n)
    }

    /// 从指定索引起点读取并转换为指定 Rust 类型。
    ///
    /// 缺失字段（索引越界）返回 `DbError::ColumnNotFound`，转换失败由 `T: TypeMapper` 处理。
    pub fn get_typed_at<T: TypeMapper>(&self, name: &str, index: usize) -> Result<T> {
        let v = self
            .get_at(index)
            .ok_or_else(|| DbError::ColumnNotFound(name.to_string()))?;
        T::from_value(v)
    }

    /// 判断行字段序是否与给定字段名列表完全一致。
    ///
    /// 仅当列数相等且逐列名字符串相等（含顺序）时返回 `true`。
    /// 用于 `from_row` 的索引快路径判定：字段集相同且顺序一致时，
    /// 可直接按索引取值，避免每次 O(列数) 的字符串查找。
    pub fn columns_match(&self, expected: &[&str]) -> bool {
        if self.entries.len() != expected.len() {
            return false;
        }
        expected
            .iter()
            .enumerate()
            .all(|(i, e)| self.name_at(i) == Some(*e))
    }

    /// 按字段名读取并转换为指定 Rust 类型。
    ///
    /// 转换由 `T: TypeMapper` 负责。
    pub fn get_typed<T: TypeMapper>(&self, name: &str) -> Result<T> {
        let v = self
            .get(name)
            .ok_or_else(|| DbError::ColumnNotFound(name.to_string()))?;
        T::from_value(v)
    }

    /// 字段数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 是否为空行。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 按插入顺序迭代字段。
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(n, v)| (&**n, v))
    }

    /// 按插入顺序迭代字段名。
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(n, _)| &**n)
    }

    /// 按插入顺序迭代字段值。
    pub fn values(&self) -> impl Iterator<Item = &Value> {
        self.entries.iter().map(|(_, v)| v)
    }

    /// 判断字段是否存在。
    pub fn contains(&self, name: &str) -> bool {
        self.entries.iter().any(|(n, _)| n.as_ref() == name)
    }
}

impl FromIterator<(String, Value)> for Row {
    fn from_iter<T: IntoIterator<Item = (String, Value)>>(iter: T) -> Self {
        Row {
            entries: iter
                .into_iter()
                .map(|(name, value)| (Arc::from(name), value))
                .collect(),
            table: None,
            primary_key: None,
        }
    }
}

//! `Page<T>` —— 分页容器，以及 keyset 游标分页所需的 `Cursor`。

use crate::stmt::Value;

/// Keyset 游标的单值。
///
/// 记录上一页最后一行的**排序列值**与**主键值**，用于在下一轮查询中通过
/// `WHERE (sort_col, pk) > (?, ?)` 行比较定位下一页，避免大表深翻页时的
/// `OFFSET` 退化。
///
/// ## 不变量
///
/// `key` 与 `pk` 必须来自同一行、且不得为 `Null`。
#[derive(Debug, Clone, PartialEq)]
pub struct Cursor {
    /// 排序列的游标值（即上一页最后一行的排序列值）。
    pub key: Value,
    /// 主键游标值（即上一页最后一行的主键值），用于并列排序时保证稳定与唯一。
    pub pk: Value,
}

/// 分页结果。
///
/// ## 不变量
///
/// 1. `pages = ceil(total / size)`；`total == 0` 时 `pages == 0`
/// 2. `page` 从 1 开始计数
/// 3. `list.len() <= size`
#[derive(Debug, Clone)]
pub struct Page<T> {
    /// 当前页数据。
    pub list: Vec<T>,
    /// 总记录数。
    pub total: u64,
    /// 当前页码（从 1 开始）。
    pub page: u64,
    /// 每页大小。
    pub size: u64,
    /// 总页数。
    pub pages: u64,
}

impl<T> Page<T> {
    /// 是否有下一页。
    pub fn has_next(&self) -> bool {
        self.page < self.pages
    }

    /// 是否有上一页。
    pub fn has_prev(&self) -> bool {
        self.page > 1
    }

    /// 当前页是否为空。
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// 转换元素类型，保留分页元数据。
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> Page<U> {
        Page {
            list: self.list.into_iter().map(f).collect(),
            total: self.total,
            page: self.page,
            size: self.size,
            pages: self.pages,
        }
    }

    /// 根据 `total` 和 `size` 计算页数。
    pub fn calc_pages(total: u64, size: u64) -> u64 {
        if size == 0 || total == 0 {
            0
        } else {
            total.div_ceil(size)
        }
    }
}

impl<T> IntoIterator for Page<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.list.into_iter()
    }
}

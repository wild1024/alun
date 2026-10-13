//! 执行结果类型。
//!
//! 异步执行抽象由 facade 层定义（dyn-safe 的 `RawExecutor`），
//! core 层保持零异步依赖，只承载执行结果数据结构。

use crate::row::Row;

/// 执行结果。
#[derive(Debug, Clone)]
pub struct ExecuteResult {
    /// 受影响的行数。
    pub rows_affected: u64,
    /// 自增主键的最后插入值（方言支持时）。
    pub last_insert_id: Option<i64>,
}

/// 查询结果集。
#[derive(Debug, Clone)]
pub struct Rows(pub Vec<Row>);

impl Rows {
    /// 结果集行数。
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 取出第一行。
    pub fn into_first(self) -> Option<Row> {
        self.0.into_iter().next()
    }
}

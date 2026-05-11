//! SQL 模板：Jinja2 风格的动态 SQL 拼接
//!
//! 采用 Jinja2 通用模板语法，零学习成本：
//!
//! ```sql
//! -- queries/user.sql
//! ## find_by_name
//! SELECT * FROM user WHERE name = {{ name }}
//!
//! ## find_by_condition
//! SELECT * FROM user WHERE 1=1
//! {% if name %} AND name = {{ name | sql_safe }} {% endif %}
//! {% if age %} AND age >= {{ age }} {% endif %}
//! {% if order_by %} ORDER BY {{ order_by }} {% endif %}
//! ```
use std::collections::HashMap;
use crate::{DbResult, DbError};

/// SQL 模板引擎
///
/// 支持 Jinja2 风格的动态 SQL 拼接。可通过 `add()` 注册模板片段，
/// 调用 `render()` 时传入参数完成变量替换。
pub struct SqlTemplate {
    /// 模板名称 → SQL 模板字符串
    templates: HashMap<String, String>,
}

impl SqlTemplate {
    /// 创建空的 SQL 模板集合
    pub fn new() -> Self {
        Self { templates: HashMap::new() }
    }

    /// 添加 SQL 模板（链式调用）
    ///
    /// ```ignore
    /// sql.add("find_user", "SELECT * FROM user WHERE id = {{ id }}");
    /// ```
    pub fn add(&mut self, name: &str, sql: impl Into<String>) -> &mut Self {
        self.templates.insert(name.to_string(), sql.into());
        self
    }

    /// 获取未经渲染的 SQL 模板原始内容，不存在返回 `None`
    pub fn get_raw(&self, name: &str) -> Option<&str> {
        self.templates.get(name).map(|s| s.as_str())
    }

    /// 渲染 SQL：将模板中的 `{{ key }}` 替换为 params 中对应的值
    ///
    /// # 错误
    ///
    /// 模板名称不存在时返回 `Argument` 错误。
    pub fn render(&self, name: &str, params: &HashMap<String, String>) -> DbResult<String> {
        let template = self.templates
            .get(name)
            .ok_or_else(|| DbError::Argument(format!("SQL 模板不存在: {}", name)))?;

        let mut result = template.clone();
        for (key, value) in params {
            let placeholder = format!("{{{{ {} }}}}", key);
            result = result.replace(&placeholder, value);
        }

        Ok(result)
    }
}

impl Default for SqlTemplate {
    fn default() -> Self {
        Self::new()
    }
}

/// SQL 参数对——SQL ID + 渲染后的 SQL + 预编译参数
///
/// 用于将模板渲染结果传递给 `Db::query()` / `Db::execute()`。
#[derive(Debug, Clone)]
pub struct SqlPara {
    /// SQL ID（缓存键，用于日志追踪）
    pub id: String,
    /// 最终 SQL 字符串（已渲染完成）
    pub sql: String,
    /// 预编译参数值（按 `$1`、`$2` 顺序排列）
    pub params: Vec<String>,
}

impl SqlPara {
    /// 创建 SQL 参数对（不含参数值，需后续填充）
    pub fn new(id: impl Into<String>, sql: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            sql: sql.into(),
            params: Vec::new(),
        }
    }
}

//! `Dialect` trait —— SQL 渲染与错误映射。

use crate::error::DbError;
use crate::expr::Expr;
use crate::statement::Statement;
use crate::stmt::{Type, Value};

/// 方言能力集。驱动声明自己支持的能力。
#[derive(Debug, Clone)]
pub struct DialectCapability {
    /// 是否有原生 `BOOLEAN` 类型。
    pub native_bool: bool,
    /// 是否有原生 `UUID` 类型。
    pub native_uuid: bool,
    /// 是否有原生 `JSON` 类型。
    pub native_json: bool,
    /// 是否有原生 `ENUM` 类型。
    pub native_enum: bool,
    /// 是否支持 `RETURNING` 子句。
    pub returning_clause: bool,
    /// 是否支持 `ILIKE` 运算符。
    pub ilike: bool,
    /// 是否支持数组类型。
    pub array_type: bool,
    /// 是否支持 `ON CONFLICT` 子句。
    pub on_conflict: bool,
    /// 是否支持 `NULLS FIRST` / `NULLS LAST`。
    pub nulls_order: bool,
    /// 占位符风格。
    pub placeholder: Placeholder,
}

/// 占位符风格。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Placeholder {
    /// `?`。MySQL、SQLite。
    Question,
    /// `$1`、`$2`。PostgreSQL。
    Dollar,
}

/// 渲染后的 SQL 与绑定参数。
#[derive(Debug, Clone)]
pub struct RenderedSql {
    /// SQL 文本。
    pub sql: String,
    /// 按占位符顺序排列的绑定参数。
    pub params: Vec<Value>,
}

/// 渲染上下文。渲染过程中追踪参数索引与能力。
pub struct RenderCtx<'a> {
    /// 已绑定的参数列表。
    pub params: Vec<Value>,
    /// 下一个参数索引（从 1 开始，用于 PostgreSQL 的 `$n`）。
    pub next_index: usize,
    /// 方言引用。
    pub dialect: &'a dyn Dialect,
}

impl<'a> RenderCtx<'a> {
    /// 创建新的渲染上下文。
    pub fn new(dialect: &'a dyn Dialect) -> Self {
        RenderCtx {
            params: Vec::new(),
            next_index: 1,
            dialect,
        }
    }

    /// 绑定一个参数，返回对应的占位符字符串。
    pub fn bind(&mut self, value: Value) -> String {
        let idx = self.next_index;
        self.params.push(value);
        self.next_index += 1;
        match self.dialect.capability().placeholder {
            Placeholder::Question => "?".to_string(),
            Placeholder::Dollar => format!("${idx}"),
        }
    }
}

/// 方言 trait。每个数据库后端实现一次。
pub trait Dialect: Send + Sync + 'static {
    /// 方言名称。
    fn name(&self) -> &'static str;

    /// 能力集。
    fn capability(&self) -> &DialectCapability;

    /// 将中间类型渲染为方言特定的列类型字符串。
    fn render_type(&self, ty: &Type) -> String;

    /// 将表达式渲染为 SQL 片段。
    fn render_expr(&self, expr: &Expr, ctx: &mut RenderCtx<'_>) -> String;

    /// 将语句渲染为完整的 SQL 与绑定参数。
    fn render_statement(&self, stmt: &Statement) -> RenderedSql;

    /// 将驱动的原始错误映射为统一的 `DbError`。
    fn map_error(&self, raw: RawDbError) -> DbError;

    /// 渲染当前时间的 SQL 函数。
    ///
    /// - SQLite: `datetime('now')`
    /// - PostgreSQL: `NOW()`
    /// - MySQL: `NOW()`
    fn render_now(&self) -> String;

    /// 引用标识符（表名、列名）。
    ///
    /// - SQLite / PostgreSQL: `"name"`
    /// - MySQL: `` `name` ``
    fn quote_identifier(&self, name: &str) -> String;

    /// 渲染 `LIMIT` / `OFFSET` 子句并追加到 `out`。
    ///
    /// ## 为什么内联字面量而不是绑定参数
    ///
    /// `LIMIT` / `OFFSET` 的值都是无符号整数，内联没有注入面。而**参数化
    /// LIMIT** 会让 SQLite 在准备期无法确定取几行：实测同一主键点查
    /// （10 万行库、1000 次、多线程运行时）`LIMIT 1` 为 0.93 µs/次，
    /// `LIMIT ?` 为 3.61 µs/次——每次查询多花约 2.7 µs，占一次点查总耗时的
    /// 三分之二。故此处一律内联字面量，方言不得改回参数绑定。
    ///
    /// 追加形式（而非返回新 `String`）让调用方直接写进已有的 SQL 缓冲。
    ///
    /// SQL Server 等需要 `OFFSET ... FETCH NEXT` 的方言可覆盖本方法。
    fn push_limit_offset(&self, limit: Option<u64>, offset: Option<u64>, out: &mut String) {
        if let Some(limit) = limit {
            out.push_str(" LIMIT ");
            push_u64(out, limit);
        }
        if let Some(offset) = offset {
            out.push_str(" OFFSET ");
            push_u64(out, offset);
        }
    }
}

/// 以十进制把 `u64` 写入字符串（栈缓冲，无堆分配）。
fn push_u64(out: &mut String, value: u64) {
    // u64 十进制最长 20 位
    let mut buf = [0u8; 20];
    let mut pos = buf.len();
    let mut rest = value;
    loop {
        pos -= 1;
        buf[pos] = b'0' + (rest % 10) as u8;
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    // ASCII 数字逐字节转 char 恒等，无失败路径
    for digit in &buf[pos..] {
        out.push(char::from(*digit));
    }
}

/// 驱动的原始错误。
#[derive(Debug)]
pub struct RawDbError {
    /// 数据库返回的错误码（如 PostgreSQL 的 `23505`）。
    pub code: Option<String>,
    /// 错误消息。
    pub message: String,
    /// 原始错误对象。
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

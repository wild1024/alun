//! `Expr` —— 表达式树。
//!
//! 表达式树是查询构建器和 SQL 模板共同产生的中间表示。方言渲染器
//! 将 `Expr` 转换为方言特定的 SQL 片段。

use crate::stmt::Value;

/// 表达式树。用于 WHERE、HAVING、JOIN ON、SELECT 列计算等场景。
///
/// ## 不变量
///
/// 1. `And(vec![])` 和 `Or(vec![])` 非法；空时应使用 `AlwaysTrue`。
/// 2. `Raw` 载荷 [`RawSql`] 字段私有，**只能**经 [`Expr::raw_trusted`] /
///    [`Expr::raw_trusted_bound`] 构造，是唯一的 SQL 拼接入口（F15 密封）。
/// 3. `Like` / `NotLike` 的 `right` 必须是 `Value(Text(_))`。
/// 4. `In` 的 `right` 必须是 `Value(Array(_))`。
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    // ==================== 基础 ====================
    /// 列引用：`table.column`。
    ///
    /// `table` 和 `column` 都是编译期已知的静态字符串，不存在注入风险。
    Column {
        /// 表名。
        table: &'static str,
        /// 列名。
        column: &'static str,
    },

    /// 字面值。
    Value(Value),

    // ==================== 组合 ====================
    /// 二元运算。
    Binary {
        /// 左操作数。
        left: Box<Expr>,
        /// 运算符。
        op: BinaryOp,
        /// 右操作数。
        right: Box<Expr>,
    },

    /// 一元运算。
    Unary {
        /// 运算符。
        op: UnaryOp,
        /// 操作数。
        expr: Box<Expr>,
    },

    /// 函数调用。
    Call {
        /// 函数。
        func: Function,
        /// 参数列表。
        args: Vec<Expr>,
    },

    /// JSON 路径访问。方言无关，由渲染器转换为方言特定的 JSON 操作符。
    JsonGet {
        /// 源表达式，通常是 `Column`。
        expr: Box<Expr>,
        /// JSON 路径，如 `"bio"` 或 `"user.name"`。
        path: String,
    },

    /// 逻辑与：`exprs[0] AND exprs[1] AND ...`。
    ///
    /// ## 不变量
    ///
    /// `exprs` 非空。空时应使用 `AlwaysTrue`。
    And(Vec<Expr>),

    /// 逻辑或：`exprs[0] OR exprs[1] OR ...`。
    ///
    /// ## 不变量
    ///
    /// `exprs` 非空。
    Or(Vec<Expr>),

    // ==================== 特殊 ====================
    /// 恒真条件。用于 `#where` 参数为空时的降级。
    ///
    /// 渲染为 `1=1`，在某些方言中可能被优化为无 WHERE 子句。
    AlwaysTrue,

    /// 逃逸口：受信任的原始 SQL 片段。
    ///
    /// 载荷是 [`RawSql`]，其字段**私有**，只能经 [`Expr::raw_trusted`] /
    /// [`Expr::raw_trusted_bound`] 构造。这是刻意的**密封**：原始 SQL 是唯一的
    /// 拼接入口，绕过列引用与值绑定两道防线，因此本变体不能被直接构造——
    /// 否则 `grep raw_trusted` 的审计能力会被架空（F15）。
    Raw(RawSql),
}

/// 受信任的原始 SQL 片段。
///
/// # 密封设计
///
/// 字段全部私有：外部**无法**用结构体字面量构造，只能用
/// [`Expr::raw_trusted`] / [`Expr::raw_trusted_bound`]（带 `#[track_caller]`），
/// 从而把「这段 SQL 来自代码常量、不来自用户输入」这一前提写进调用点。
/// 渲染层通过 [`sql`](Self::sql) / [`binds`](Self::binds) 只读访问。
#[derive(Debug, Clone, PartialEq)]
pub struct RawSql {
    /// 原始 SQL 文本。
    sql: String,
    /// 按 `?` 顺序对应的参数表达式。
    binds: Vec<Expr>,
}

impl RawSql {
    /// 原始 SQL 文本。
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// 绑定的参数表达式（按 `?` 出现顺序）。
    pub fn binds(&self) -> &[Expr] {
        &self.binds
    }
}

/// 二元运算符。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BinaryOp {
    // ---- 比较 ----
    /// 等于 `=`。
    Eq,
    /// 不等于 `<>`。
    Ne,
    /// 小于 `<`。
    Lt,
    /// 小于等于 `<=`。
    Le,
    /// 大于 `>`。
    Gt,
    /// 大于等于 `>=`。
    Ge,

    // ---- 模式匹配 ----
    /// 模糊匹配 `LIKE`。
    Like,
    /// 不匹配 `NOT LIKE`。
    NotLike,
    /// 大小写不敏感匹配 `ILIKE`。方言不支持时降级或报错。
    ILike,

    // ---- 集合 ----
    /// 属于集合 `IN`。
    In,
    /// 不属于集合 `NOT IN`。
    NotIn,

    // ---- 空值 ----
    /// 为空 `IS NULL`。`right` 忽略。
    IsNull,
    /// 非空 `IS NOT NULL`。`right` 忽略。
    IsNotNull,

    // ---- 算术 ----
    /// 加法 `+`。
    Add,
    /// 减法 `-`。
    Sub,
    /// 乘法 `*`。
    Mul,
    /// 除法 `/`。
    Div,
    /// 取模 `%`。
    Mod,

    // ---- 字符串 ----
    /// 字符串连接 `||`。
    Concat,
}

/// 一元运算符。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum UnaryOp {
    /// 逻辑非 `NOT`。
    Not,
    /// 算术取负 `-`。
    Neg,
}

/// 内置函数。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Function {
    /// 计数。
    Count,
    /// 求和。
    Sum,
    /// 平均值。
    Avg,
    /// 最小值。
    Min,
    /// 最大值。
    Max,
    /// 空值合并 `COALESCE`。
    Coalesce,
    /// 转小写 `LOWER`。
    Lower,
    /// 转大写 `UPPER`。
    Upper,
    /// 字符串长度。
    Length,
    /// 去除首尾空白。
    Trim,
    /// 当前时间。方言特定。
    Now,
    /// 自定义函数名。
    Custom(&'static str),
}

impl Expr {
    /// 将多个表达式组合为 `AND`。
    ///
    /// - 自动过滤 `AlwaysTrue`
    /// - 空输入返回 `AlwaysTrue`
    /// - 单元素返回该元素
    pub fn and_all(exprs: impl IntoIterator<Item = Expr>) -> Expr {
        let v: Vec<_> = exprs
            .into_iter()
            .filter(|e| !matches!(e, Expr::AlwaysTrue))
            .collect();
        match v.len() {
            0 => Expr::AlwaysTrue,
            1 => v.into_iter().next().unwrap(),
            _ => Expr::And(v),
        }
    }

    /// 将多个表达式组合为 `OR`。
    pub fn or_all(exprs: impl IntoIterator<Item = Expr>) -> Expr {
        let v: Vec<_> = exprs
            .into_iter()
            .filter(|e| !matches!(e, Expr::AlwaysTrue))
            .collect();
        match v.len() {
            0 => Expr::AlwaysTrue,
            1 => v.into_iter().next().unwrap(),
            _ => Expr::Or(v),
        }
    }

    /// 构造一段**受信任**的原始 SQL 片段（无绑定参数）。
    ///
    /// 原始 SQL 是唯一的拼接入口，绕过列引用与值绑定两道防线。本方法把
    /// 「这段 SQL 来自代码常量、不来自用户输入」这一前提**写进调用点**，
    /// 便于代码评审与静态检索（`grep raw_trusted` 即可盘点全部拼接点）。
    ///
    /// # 安全
    ///
    /// 调用方必须保证 `sql` **不包含任何用户输入**。若需拼接变量值，请用
    /// [`Expr::raw_trusted_bound`] 并把它放进 `binds`（占位符用 `?`），
    /// 由驱动程序完成参数化绑定。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// // 常量片段：安全
    /// Expr::raw_trusted("lower(name) LIKE 'a%'");
    /// // 带参数：安全（值走绑定，不参与拼接）
    /// Expr::raw_trusted_bound("lower(name) LIKE ?", vec![Expr::Value(Value::Text("a%".into()))]);
    /// ```
    #[track_caller]
    pub fn raw_trusted(sql: impl Into<String>) -> Expr {
        // F10：受信任原始 SQL 的**调用计数**。
        //
        // 它回答运维/安全巡检里最关键的一问："这套系统里到底有没有人绕过了
        // 列引用与值绑定？用了多少次？"——`alun_db_raw_sql_total` 从 0 变正的
        // 那一刻，就是一次值得复盘的信号。与 F15（把 `Expr::Raw` 密封为唯一入口）
        // 联动：入口唯一 ⇒ 计数必然完备。
        #[cfg(feature = "metrics")]
        {
            crate::metrics::describe_once();
            metrics::counter!("alun_db_raw_sql_total", "kind" => "trusted").increment(1);
        }

        Expr::Raw(RawSql {
            sql: sql.into(),
            binds: Vec::new(),
        })
    }

    /// 构造一段**受信任**的原始 SQL 片段（带绑定参数）。
    ///
    /// `sql` 中的 `?` 按顺序对应 `binds`，值不参与字符串拼接。
    ///
    /// # 安全
    ///
    /// 同 [`Expr::raw_trusted`]：`sql` 本身必须来自代码常量。
    #[track_caller]
    pub fn raw_trusted_bound(sql: impl Into<String>, binds: Vec<Expr>) -> Expr {
        // 同 `raw_trusted`：带绑定变体同样计数（`kind = "trusted_bound"`）。
        #[cfg(feature = "metrics")]
        {
            crate::metrics::describe_once();
            metrics::counter!("alun_db_raw_sql_total", "kind" => "trusted_bound").increment(1);
        }

        Expr::Raw(RawSql {
            sql: sql.into(),
            binds,
        })
    }
}

//! `Statement` —— 语句 AST。

use crate::expr::Expr;
use crate::stmt::Value;

/// 语句 AST。由查询构建器构造，由方言渲染器转换为 SQL。
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// SELECT 查询。
    Select(SelectStmt),
    /// INSERT 插入。
    Insert(InsertStmt),
    /// UPDATE 更新。
    Update(UpdateStmt),
    /// DELETE 删除。
    Delete(DeleteStmt),
    /// 已渲染的原始 SQL。
    Raw(RawStmt),
}

// ==================== SELECT ====================

/// 租户作用域标记。
///
/// 区分「未设置」「已限定」「显式跨租户」三种状态，使执行前的 fail-closed
/// 守卫能区分「忘记注入」与「合法越权」：`Unset` 表示注入层未处理该语句，
/// 执行前会被拒绝；`Cross` 表示调用方已显式声明跨租户。
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ScopeMode {
    /// 尚未注入。执行前若仍为此状态 → 拒绝执行（fail-closed）。
    #[default]
    Unset,
    /// 已限定到某租户。
    Scoped(ScopeFilter),
    /// 显式跨租户（管理后台 / 迁移任务）。
    Cross,
}

/// 租户过滤条件。
///
/// 字段全部内联（`&'static str` + `Value`），构造时零堆分配。渲染时值经参数
/// 绑定（`?` / `$n`）而非内联，避免 SQLite 语句缓存按租户值炸开。
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeFilter {
    /// 参与过滤的表名（主表或 JOIN 从表）。
    pub table: &'static str,
    /// 租户列名（来自 `Model::TENANT`，已受标识符校验保护）。
    pub column: &'static str,
    /// 当前租户 ID。
    pub value: Value,
}

/// SELECT 语句。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SelectStmt {
    /// 主表名。
    pub table: &'static str,
    /// 选择列。为空时表示 `SELECT *`。
    pub columns: Vec<ColumnRef>,
    /// WHERE 条件。
    pub filter: Option<Expr>,
    /// 租户作用域。由注入层设置，渲染器负责拼入 WHERE 最外层。
    pub scope: ScopeMode,
    /// ORDER BY 子句。
    pub order_by: Vec<OrderBy>,
    /// GROUP BY 子句。
    pub group_by: Vec<ColumnRef>,
    /// HAVING 条件。
    pub having: Option<Expr>,
    /// LIMIT 子句。
    pub limit: Option<u64>,
    /// OFFSET 子句。
    pub offset: Option<u64>,
    /// JOIN 子句。
    pub joins: Vec<Join>,
}

// ==================== 列引用 ====================

/// SELECT 列引用。
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnRef {
    /// 单列引用：`name` 或 `table.name`。
    Column(&'static str),
    /// 带别名：`name AS alias`。
    Aliased {
        /// 原始列名。
        name: &'static str,
        /// 别名。
        alias: String,
    },
    /// 通配符：`*` 或 `table.*`。
    Star(Option<&'static str>),
    /// 表达式列：`COUNT(*) AS total`。
    Expr {
        /// 表达式。
        expr: Expr,
        /// 别名（必填）。
        alias: String,
    },
}

// ==================== 排序 ====================

/// ORDER BY 子句。
#[derive(Debug, Clone, PartialEq)]
pub struct OrderBy {
    /// 排序列（或表达式）。
    pub expr: ColumnRef,
    /// 排序方向。
    pub direction: Order,
    /// NULL 值排序位置。方言支持时才生效。
    pub nulls: Option<NullsOrder>,
}

/// 排序方向。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Order {
    /// 升序。
    Asc,
    /// 降序。
    Desc,
}

/// NULL 值的排序位置。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NullsOrder {
    /// NULL 在前。
    First,
    /// NULL 在后。
    Last,
}

// ==================== JOIN ====================

/// JOIN 子句。
#[derive(Debug, Clone, PartialEq)]
pub struct Join {
    /// JOIN 类型。
    pub kind: JoinKind,
    /// 被 JOIN 的表名。
    pub table: &'static str,
    /// ON 条件。
    pub on: Expr,
    /// 从表租户作用域。`Some` 时渲染器在 ON 之后追加该从表的租户条件，
    /// 防止跨租户联查；`None` 表示该从表不参与隔离。
    pub scope: Option<ScopeFilter>,
}

/// JOIN 类型。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum JoinKind {
    /// 内连接。
    Inner,
    /// 左外连接。
    Left,
    /// 右外连接。
    Right,
    /// 全外连接。
    Full,
}

// ==================== INSERT ====================

/// INSERT 语句。
///
/// ## 不变量
///
/// - `columns.len()` 与每个 `values[i].len()` 必须一致
/// - `values` 至少包含一行
#[derive(Debug, Clone, PartialEq)]
pub struct InsertStmt {
    /// 表名。
    pub table: &'static str,
    /// 插入的列名列表。
    pub columns: Vec<&'static str>,
    /// 每一行的值。外层为行，内层为列。
    pub values: Vec<Vec<Expr>>,
    /// RETURNING 子句（方言支持时）。
    pub returning: Option<Vec<&'static str>>,
    /// ON CONFLICT 子句（方言支持时）。
    pub on_conflict: Option<OnConflict>,
}

/// ON CONFLICT 子句。
#[derive(Debug, Clone, PartialEq)]
pub struct OnConflict {
    /// 冲突目标列。
    pub target: Vec<&'static str>,
    /// 冲突时的动作。
    pub action: ConflictAction,
}

/// 冲突时的动作。
#[derive(Debug, Clone, PartialEq)]
pub enum ConflictAction {
    /// 忽略冲突 `DO NOTHING`。
    DoNothing,
    /// 更新指定列 `DO UPDATE SET ...`。
    DoUpdate(Vec<(&'static str, Expr)>),
}

// ==================== UPDATE ====================

/// UPDATE 语句。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UpdateStmt {
    /// 表名。
    pub table: &'static str,
    /// SET 子句：`(列名, 新值)`。
    pub sets: Vec<(&'static str, Expr)>,
    /// WHERE 条件。
    pub filter: Option<Expr>,
    /// 租户作用域。由注入层设置，渲染器负责拼入 WHERE 最外层。
    pub scope: ScopeMode,
    /// RETURNING 子句。
    pub returning: Option<Vec<&'static str>>,
}

// ==================== DELETE ====================

/// DELETE 语句。
///
/// ## 不变量
///
/// 无条件删除（`filter == None`）需要调用方显式声明。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DeleteStmt {
    /// 表名。
    pub table: &'static str,
    /// WHERE 条件。
    pub filter: Option<Expr>,
    /// 租户作用域。由注入层设置，渲染器负责拼入 WHERE 最外层。
    pub scope: ScopeMode,
    /// RETURNING 子句。
    pub returning: Option<Vec<&'static str>>,
}

/// 原始 SQL 语句。
#[derive(Debug, Clone, PartialEq)]
pub struct RawStmt {
    /// SQL 文本，含占位符（`?` 或 `$n`）。
    pub sql: String,
    /// 按占位符顺序排列的绑定参数。
    pub params: Vec<Value>,
}

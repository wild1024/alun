//! 多租户上下文与作用域注入。
//!
//! 租户上下文保存在 [`tokio::task_local!`] 中而非 `thread_local!`：async 任务在
//! `await` 点会跨线程迁移，`thread_local!` 会让 A 请求的租户值被 B 请求读到，
//! 是最危险的一类串租户缺陷。
//!
//! ## 与 `soft_filter` 的关系
//!
//! 租户隔离复刻软删除的「声明 → 注入 → 豁免」范式，注入点与
//! [`soft_filter`](crate::model_query::soft_filter) 完全重叠，区别在于租户条件
//! 走语句的结构化字段（每请求值都变、不可缓存），而非可复用的 `Expr`。

use std::future::Future;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use alun_db_core::error::{DbError, Result};
use alun_db_core::field::FieldMeta;
use alun_db_core::row::Row;
use alun_db_core::statement::{ScopeFilter, ScopeMode};
use alun_db_core::stmt::Value;
use alun_db_core::traits::Model;
use alun_db_core::traits::dialect::Dialect;

tokio::task_local! {
    /// 当前任务的租户上下文。
    static TENANT_CTX: TenantContext;
}

/// 多租户隔离**总开关**（进程级，默认开启）。
///
/// `[tenant].enabled = false`（配置默认值）时由 `alun-web` 在启动阶段置为
/// `false`：此时 `#[tenant]` 属性完全惰性——既不注入条件，也不做 fail-closed
/// 守卫，存量系统保持零回归。纯 `alun-db` 使用者不设置该开关，默认保持开启。
static TENANT_ENABLED: AtomicBool = AtomicBool::new(true);

/// **写路径**租户补齐 / 守卫开关（进程级，默认开启）。
///
/// 迁移观察期可单独关闭（`[tenant].write_enabled = false`）：读路径照常隔离，
/// 写路径退化为「不补列、不校验、不追加 WHERE」，与未开启隔离时逐字一致，
/// 便于与既有服务层手写的 `AND tenant_id = ?` 并存、逐步切换。
static TENANT_WRITE_ENABLED: AtomicBool = AtomicBool::new(true);

/// 设置多租户隔离总开关。
///
/// 由 `alun-web` 依据 `[tenant].enabled` 在应用启动阶段调用；业务代码通常无需关心。
pub fn set_tenant_enabled(is_enabled: bool) {
    TENANT_ENABLED.store(is_enabled, Ordering::Relaxed);
}

/// 多租户隔离总开关是否开启（默认 `true`）。
pub fn is_tenant_enabled() -> bool {
    TENANT_ENABLED.load(Ordering::Relaxed)
}

/// 设置写路径租户补齐 / 守卫开关。
///
/// 由 `alun-web` 依据 `[tenant].write_enabled` 在应用启动阶段调用；业务代码通常无需关心。
pub fn set_tenant_write_enabled(is_enabled: bool) {
    TENANT_WRITE_ENABLED.store(is_enabled, Ordering::Relaxed);
}

/// 写路径租户补齐 / 守卫开关是否开启（默认 `true`）。
pub fn is_tenant_write_enabled() -> bool {
    TENANT_WRITE_ENABLED.load(Ordering::Relaxed)
}

/// 默认租户 schema 名前缀（`[tenant].schema_prefix` 缺省值）。
const DEFAULT_TENANT_SCHEMA_PREFIX: &str = "tenant_";

/// PostgreSQL schema-per-tenant 开关（进程级，默认关闭）。
///
/// `[tenant].schema_per_tenant = true` 时由 `alun-web` 在启动阶段置为 `true`。
/// 开启后，借出 PG 连接时按当前租户执行 `SET search_path TO "<前缀><租户ID>"`，
/// 实现 schema 级（库级）隔离；仅 PostgreSQL 生效，SQLite / MySQL 忽略。
static TENANT_SCHEMA_ENABLED: AtomicBool = AtomicBool::new(false);

/// 租户 schema 名前缀（进程级，默认 `tenant_`）。空串视为使用默认值。
static TENANT_SCHEMA_PREFIX: RwLock<String> = RwLock::new(String::new());

/// 设置 PostgreSQL schema-per-tenant 开关。
///
/// 由 `alun-web` 依据 `[tenant].schema_per_tenant` 在应用启动阶段调用。
pub fn set_tenant_schema_enabled(is_enabled: bool) {
    TENANT_SCHEMA_ENABLED.store(is_enabled, Ordering::Relaxed);
}

/// PostgreSQL schema-per-tenant 开关是否开启（默认 `false`）。
pub fn is_tenant_schema_enabled() -> bool {
    TENANT_SCHEMA_ENABLED.load(Ordering::Relaxed)
}

/// 设置租户 schema 名前缀（空串表示使用默认 `tenant_`）。
pub fn set_tenant_schema_prefix(prefix: &str) {
    let mut guard = TENANT_SCHEMA_PREFIX
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = prefix.to_string();
}

/// 当前租户 schema 名前缀（未设置时返回默认 `tenant_`）。
pub fn tenant_schema_prefix() -> String {
    let guard = TENANT_SCHEMA_PREFIX
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.is_empty() {
        DEFAULT_TENANT_SCHEMA_PREFIX.to_string()
    } else {
        guard.clone()
    }
}

/// 租户上下文。
///
/// 用枚举而非 `Option<Value>`：区分「限定到某租户」与「显式跨租户」，
/// 避免 `None` 被误读为「无租户 = 不过滤」。
#[derive(Debug, Clone, PartialEq)]
pub enum TenantContext {
    /// 限定到具体租户。
    Scoped(Value),
    /// 显式跨租户（管理后台、迁移、巡检）。
    Cross,
}

/// 在指定租户上下文中执行 future。
///
/// 用于无 HTTP 请求上下文的场景（定时任务、CLI、MQ 消费、测试）。
///
/// # 示例
///
/// ```ignore
/// tenant_scope(TenantContext::Scoped(Value::I64(42)), async {
///     let rows = Invoice::query().find(db()).await?;
///     Ok::<_, DbError>(())
/// }).await
/// ```
pub async fn tenant_scope<F, T>(ctx: TenantContext, fut: F) -> T
where
    F: Future<Output = T>,
{
    TENANT_CTX.scope(ctx, fut).await
}

/// 读取当前租户上下文。未设置时返回 `None`。
pub fn current_tenant() -> Option<TenantContext> {
    TENANT_CTX.try_with(|ctx| ctx.clone()).ok()
}

/// 把租户值渲染为可安全用于**资源命名空间**的字符串。
///
/// 用于缓存 key 前缀、对象存储路径（`t/{id}/...`）、日志标签等需要
/// 「同一租户的所有资源聚在同一命名空间下」的场景。
///
/// # 校验（fail-closed）
///
/// 仅接受整数、文本、枚举、UUID / ULID 形态，且渲染结果必须匹配
/// `[A-Za-z0-9_-]{1,64}`。含 `:`、`/`、空格的租户值会被拒绝——否则
/// 租户 A 的 ID 可能被构造成越出自己命名空间的路径（如 `a/../../b`）。
///
/// # 错误
///
/// 租户值类型不受支持，或渲染结果含非法字符时返回 [`DbError::InvalidValue`]。
pub fn tenant_value_key(value: &Value) -> Result<String> {
    let raw = match value {
        Value::I8(v) => v.to_string(),
        Value::I16(v) => v.to_string(),
        Value::I32(v) => v.to_string(),
        Value::I64(v) => v.to_string(),
        Value::U8(v) => v.to_string(),
        Value::U16(v) => v.to_string(),
        Value::U32(v) => v.to_string(),
        Value::U64(v) => v.to_string(),
        Value::Text(v) => v.clone(),
        Value::Enum(v) => v.clone(),
        // UUID / ULID 以 16 字节紧凑形式存储，无 `Display`：渲染为
        // 标准 8-4-4-4-12 文本形式（连字符在允许字符集内）。
        Value::Uuid(v) | Value::Ulid(v) => uuid_text(&v.0),
        other => {
            return Err(DbError::InvalidValue(format!(
                "tenant value of type {} cannot be used as a resource namespace; \
                 use an integer, text, enum, uuid or ulid tenant id",
                std::any::type_name_of_val(other)
            )));
        }
    };

    if raw.is_empty() || raw.len() > 64 {
        return Err(DbError::InvalidValue(format!(
            "tenant id `{raw}` must be 1..=64 characters to be used as a resource namespace"
        )));
    }
    if !raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(DbError::InvalidValue(format!(
            "tenant id `{raw}` contains characters outside [A-Za-z0-9_-]; \
             refusing to build a resource namespace from it"
        )));
    }
    Ok(raw)
}

/// 把 16 字节紧凑标识渲染为 `8-4-4-4-12` 的 UUID/ULID 文本形式。
fn uuid_text(bytes: &[u8; 16]) -> String {
    let hex = |range: std::ops::Range<usize>| -> String {
        bytes[range].iter().map(|b| format!("{b:02x}")).collect()
    };
    format!(
        "{}-{}-{}-{}-{}",
        hex(0..4),
        hex(4..6),
        hex(6..8),
        hex(8..10),
        hex(10..16)
    )
}

/// 当前租户的**资源命名空间键**。
///
/// # 返回
///
/// - 限定租户 → `Ok(Some("<租户键>"))`
/// - 无上下文 / 显式跨租户（[`TenantContext::Cross`]）→ `Ok(None)`
///   （跨租户没有「单一命名空间」的语义，由调用方决定如何处理）
///
/// # 错误
///
/// 租户值无法作为命名空间片段时返回 [`DbError::InvalidValue`]。
///
/// # 示例
///
/// ```ignore
/// use alun_db::prelude::current_tenant_key;
///
/// let key = current_tenant_key()?.expect("租户上下文");
/// let cache_key = format!("dict:{}:types", key);
/// let object_path = format!("t/{key}/2026/10/a.png");
/// ```
pub fn current_tenant_key() -> Result<Option<String>> {
    match current_tenant() {
        Some(TenantContext::Scoped(value)) => tenant_value_key(&value).map(Some),
        Some(TenantContext::Cross) | None => Ok(None),
    }
}

/// 为模型 `T` 计算租户作用域（读路径）。
///
/// # 返回
///
/// - 总开关关闭（[`is_tenant_enabled`] 为 `false`）→ [`ScopeMode::Cross`]（属性惰性，零回归）
/// - `T` 声明了 `#[tenant]` 且上下文为 `Scoped(id)` → [`ScopeMode::Scoped`]
/// - 上下文为 `Cross` → [`ScopeMode::Cross`]
/// - `T` 未声明租户列（含 `#[model(cross_tenant)]`）→ [`ScopeMode::Cross`]（模型级豁免）
///
/// # 错误
///
/// `T` 需要隔离（`TENANT.is_some()`）但当前无租户上下文时返回
/// [`DbError::InvalidValue`]——fail-closed，绝不静默放行全量数据。
pub(crate) fn scope_for<T: Model>() -> Result<ScopeMode> {
    // 总开关关闭：`#[tenant]` 属性完全惰性，不隔离也不守卫。
    if !is_tenant_enabled() {
        return Ok(ScopeMode::Cross);
    }
    let Some(column) = T::TENANT else {
        // 未声明租户列（或 `#[model(cross_tenant)]`）→ 模型级豁免，不做隔离
        return Ok(ScopeMode::Cross);
    };
    match current_tenant() {
        Some(TenantContext::Scoped(value)) => Ok(ScopeMode::Scoped(ScopeFilter {
            table: T::TABLE,
            column,
            value,
        })),
        Some(TenantContext::Cross) => Ok(ScopeMode::Cross),
        None => {
            // F10 安全指标：fail-closed 拦下一次"租户模型 + 无租户上下文"。
            //
            // 单次出现不一定是 bug（也可能是调用方预期内的报错），但**持续增长**
            // 必然指向一处漏配的 `tenant_scope` 或漏标的 `cross_tenant`——
            // 日志里它只是一行 error，指标里它是可告警的斜率。
            #[cfg(feature = "metrics")]
            crate::metrics::observe_tenant_scope_missing(T::TABLE, "read");
            Err(DbError::InvalidValue(format!(
                "model `{}` requires a tenant scope but no tenant context is set; \
                 wrap the call in `tenant_scope(..)` or declare `#[model(cross_tenant)]`",
                T::TABLE
            )))
        }
    }
}

/// 为**从表** `U` 计算 JOIN 场景下的租户作用域。
///
/// 与 [`scope_for`] 语义完全一致，区别只在返回形态：JOIN 的从表作用域是
/// `Option<ScopeFilter>`（`None` 表示该从表不参与隔离）。
///
/// 这是「结构化联查租户注入」的解析入口：由 `ModelQuery::join::<U>()` 调用，
/// 结果交给 `FindBuilder::join_scoped` 追加到该 JOIN 的 `ON` 之后。相比原生
/// SQL 的 `/*tenant:别名*/` 占位，它由框架**枚举从表模型**，不存在"漏标一张表"
/// 导致的静默跨租户泄漏。
///
/// # 错误
///
/// 同 [`scope_for`]：`U` 需要隔离但当前无租户上下文时返回
/// [`DbError::InvalidValue`]（fail-closed）。
pub(crate) fn join_scope_for<U: Model>() -> Result<Option<ScopeFilter>> {
    match scope_for::<U>()? {
        ScopeMode::Scoped(filter) => Ok(Some(filter)),
        // `Cross`（显式跨租户 / 未声明租户列 / 总开关关闭）→ 不追加条件
        _ => Ok(None),
    }
}

/// 为模型 `T` 计算租户作用域（**写路径**）。
///
/// 在 [`scope_for`] 之上叠加写路径灰度开关：`[tenant].write_enabled = false` 时
/// 写路径完全惰性（`Cross`），与未开启隔离时逐字一致，便于迁移观察期与既有
/// 服务层手写租户条件并存。读路径不受此开关影响。
fn scope_for_write<T: Model>() -> Result<ScopeMode> {
    if !is_tenant_write_enabled() {
        return Ok(ScopeMode::Cross);
    }
    scope_for::<T>()
}

/// 计算「手写 SQL 写路径」需要追加的租户谓词与绑定值。
///
/// 供无法走 Builder 渲染链的写路径（整行 `update`、`delete_by_pk`、
/// `ChangeSet`、批量插入的裸渲染等）复用：返回 `("{列}" = ?, 租户值)`，
/// 列名经标识符校验后方言引用。
///
/// # 返回
///
/// - 需要隔离 → `Some((谓词, 租户值))`
/// - 不需要隔离（模型无租户列 / 已跨租户 / 写路径灰度关闭）→ `None`
///
/// # 错误
///
/// 模型需要隔离但无上下文时返回错误（同 [`scope_for`]）。
pub(crate) fn scoped_predicate<T: Model>(dialect: &dyn Dialect) -> Result<Option<(String, Value)>> {
    match scope_for_write::<T>()? {
        ScopeMode::Scoped(filter) => {
            alun_db_sql::render::validate_identifier(filter.column)?;
            let column = dialect.quote_identifier(filter.column);
            Ok(Some((format!("{column} = ?"), filter.value)))
        }
        _ => Ok(None),
    }
}

/// 校验并按需补齐行内租户列的值（INSERT 路径）。
///
/// 规则：
/// - 行内租户值为占位（`Null` / 整数 0）→ 用上下文租户值补齐；
/// - 行内租户值与上下文一致 → 保留原值（返回 `None` 表示无需替换）；
/// - 不一致 → 返回 [`DbError::InvalidValue`]（防越权写入）。
///
/// # 返回
///
/// `Some(值)` 表示应把行内该列替换为该值；`None` 表示保持原值
/// （模型无租户列 / 已跨租户 / 写路径灰度关闭）。
///
/// # 错误
///
/// 行内租户值与上下文不一致，或模型需要隔离但无上下文时返回错误。
pub(crate) fn resolve_tenant_value<T: Model>(current: &Value) -> Result<Option<Value>> {
    match scope_for_write::<T>()? {
        ScopeMode::Scoped(filter) => {
            if current.is_unset_pk() {
                Ok(Some(filter.value))
            } else if *current == filter.value {
                Ok(None)
            } else {
                Err(DbError::InvalidValue(format!(
                    "tenant value on `{}` does not match the current tenant context; \
                     refusing a cross-tenant write",
                    T::TABLE
                )))
            }
        }
        _ => Ok(None),
    }
}

/// 定位模型的租户字段元数据（标注 `#[tenant]` 的字段）。
///
/// 模型不参与隔离时返回 `None`。
pub(crate) fn tenant_field<T: Model>() -> Option<&'static FieldMeta> {
    let column = T::TENANT?;
    T::fields().iter().find(|f| f.column == column)
}

/// 写路径上按模型租户列校验 / 补齐行内租户值。
///
/// 供 INSERT / UPDATE 在渲染前统一调用，规则同 [`resolve_tenant_value`]：
/// 占位值补齐、一致则保留、不一致则报错。模型无租户列时不做任何事。
///
/// # 错误
///
/// 行内租户值与上下文冲突，或模型需要隔离但无上下文时返回错误。
pub(crate) fn fill_row_tenant<T: Model>(row: &mut Row) -> Result<()> {
    let Some(field) = tenant_field::<T>() else {
        return Ok(());
    };
    let current = row.get(field.name).cloned().unwrap_or(Value::Null);
    if let Some(filled) = resolve_tenant_value::<T>(&current)? {
        row.insert(field.name, filled);
    }
    Ok(())
}

/// 校验原生 SQL（`ModelQuery::from_sql` / `T::sql(..)`）路径的租户合规性。
///
/// 原生 SQL 绕过 AST 注入，无法自动追加租户条件，因此对租户模型：
///
/// - 模型不参与隔离（无 `#[tenant]` / `#[model(cross_tenant)]`）→ 放行；
/// - 上下文为 [`TenantContext::Cross`]（显式跨租户）→ 放行；
/// - 上下文为 `Scoped`，且 **既注入了占位（`tenant_bound`）又由调用方显式
///   承诺完整性（`annotated`）** → 放行；
/// - 其余 → **拒绝**。
///
/// # 设计取向：从「出现占位即放行」改为「必须显式承诺完整性」
///
/// 早期实现只要 SQL 里出现**任意一个** `/*tenant:别名*/` 就放行。这是一个
/// **静默跨租户泄漏**：框架无法从 SQL 文本知道有几张租户表，只标注了主表
/// `o`、漏标 JOIN 从表 `c` 时，`c` 的行完全不过滤，守卫却因 `o` 的占位而放行。
///
/// 现在要求调用方额外调用 `ModelQuery::raw_tenant_annotated()` 显式承诺
/// "每张租户表都已标注"。这把无法自动校验的"完整性"从框架的隐式假设，
/// 变成了**调用点可见、可 `grep`、可纳入评审**的显式声明。
///
/// # 首选替代
///
/// 结构化联查（`ModelQuery::join::<U>`）由框架枚举从表模型并注入条件，
/// **不存在漏标风险**，应优先使用；原生 SQL 占位通道仅用于结构化表达不了的
/// 场景（`UNION` / CTE / 窗口函数 / 方言特性）。
///
/// # 错误
///
/// 租户模型在 `Scoped` 上下文下使用原生 SQL 但未满足「占位 + 显式承诺」时，
/// 或没有任何租户上下文时，返回 [`DbError::InvalidValue`]。
pub(crate) fn ensure_raw_allowed<T: Model>(tenant_bound: bool, annotated: bool) -> Result<()> {
    // 总开关关闭时属性惰性，原生 SQL 不受守卫约束。
    if !is_tenant_enabled() {
        return Ok(());
    }
    if T::TENANT.is_none() || T::CROSS_TENANT {
        return Ok(());
    }
    match current_tenant() {
        // 显式跨租户：调用方已声明，放行
        Some(TenantContext::Cross) => Ok(()),
        // 占位已注入 **且** 调用方显式承诺完整性，放行
        Some(TenantContext::Scoped(_)) if tenant_bound && annotated => Ok(()),
        Some(TenantContext::Scoped(_)) => {
            // F10 安全指标：原生 SQL 撞上"占位 / 完整性承诺"守卫。
            //
            // 这条线比上面那条更值得告警——它意味着有人在多租户上下文里写了
            // 框架无法校验的 SQL。正确做法是改结构化联查；指标给出"哪里还在用
            // 逃生通道、用了多少次"的量化依据。
            #[cfg(feature = "metrics")]
            crate::metrics::observe_raw_sql_unbound(T::TABLE);
            Err(DbError::InvalidValue(format!(
                "model `{table}` is tenant-isolated, but its raw SQL is not verifiably bound to the \
             current tenant (tenant_bound={tenant_bound}, annotated={annotated}). Fix it by:\n  \
             1. preferring a structured join: `{table}::query().join::<JoinModel>(JoinKind::Inner, on)` \
             — the framework injects the tenant predicate for every `#[tenant]` join target, so a \
             forgotten join table cannot leak;\n  \
             2. if raw SQL is unavoidable: annotate EVERY tenant table with `{marker}:别名*/` \
             (joined with `AND`) AND assert completeness via `.raw_tenant_annotated()`;\n  \
             or run inside `tenant_scope(TenantContext::Cross, ..)` to declare a deliberate \
             cross-tenant access",
                table = T::TABLE,
                marker = TENANT_MARKER,
                tenant_bound = tenant_bound,
                annotated = annotated,
            )))
        }
        None => {
            #[cfg(feature = "metrics")]
            crate::metrics::observe_tenant_scope_missing(T::TABLE, "raw");
            Err(DbError::InvalidValue(format!(
                "model `{}` requires a tenant scope but no tenant context is set; \
                 wrap the call in `tenant_scope(..)` or declare `#[model(cross_tenant)]`",
                T::TABLE
            )))
        }
    }
}

// ─────────────────────────── 原生 SQL 租户绑定 ───────────────────────────

/// 原生 SQL 中的**租户绑定占位标记**前缀。
///
/// 写成块注释形式，好处是 SQL 本身仍可直接粘贴到数据库客户端执行
/// （标记只是一段注释），且不依赖 `sql!` 宏——手写的 `SqlQuery` 同样适用。
///
/// - `/*tenant:别名*/` → `("别名"."租户列" = ?)`（推荐：JOIN 场景必须带别名）
/// - `/*tenant*/` → `("租户列" = ?)`（单表、无别名时使用）
///
/// # 展开为「独立谓词」，需自行用 `AND` 连接
///
/// 占位展开后是一个**带括号的完整布尔表达式**，因此调用方要把它接在
/// `AND` 之后，不要紧贴另一个条件：
///
/// ```sql
/// WHERE o.status = ?  AND /*tenant:o*/  AND /*tenant:c*/    -- ✅
/// WHERE o.status = ?  /*tenant:o*/                          -- ❌ 语法错误
/// ```
///
/// # 完整性必须显式承诺（防漏标泄漏）
///
/// 框架**无法**从 SQL 文本判断"租户表是否都标注了"。因此：
///
/// - 用占位标注每张租户表（漏标一张 = 该表不过滤 = 静默跨租户泄漏）；
/// - 且**必须**再调用 `ModelQuery::raw_tenant_annotated()` 显式承诺完整性；
///
/// 二者缺一，`ensure_raw_allowed` 都会拒绝。能用结构化联查
/// （`ModelQuery::join::<U>`）就不要用原生 SQL——后者由框架枚举从表模型，
/// 漏不了。
///
/// # 示例
///
/// ```ignore
/// // 注意 `sql!` 的插值参数是独立实参，不写在大括号里
/// let sql = sql!(
///     "SELECT o.id, o.amount, c.name
///        FROM biz_order o
///        JOIN biz_customer c ON c.id = o.customer_id
///       WHERE o.status = ", { status },
///     " AND /*tenant:o*/ AND /*tenant:c*/
///       ORDER BY o.created_at DESC"
/// );
/// let rows = Order::sql(sql).raw_tenant_annotated().find(db()).await?;
/// ```
pub const TENANT_MARKER: &str = "/*tenant";

/// 把原生 SQL 中的租户占位替换为真实谓词，并注入租户绑定值。
///
/// # 返回
///
/// `(重写后的 SqlQuery, 是否真的注入了租户条件)`
///
/// 第二个值回传给 [`ensure_raw_allowed`] 作为「已显式绑定」的证据。
///
/// # 行为
///
/// | 情形 | 占位替换为 |
/// |---|---|
/// | 租户开启 + `Scoped(v)` + 模型有租户列 | `别名."租户列" = ?`（绑定 `v`） |
/// | 上下文为 `Cross` / 模型不参与隔离 / 总开关关闭 | `1 = 1`（不过滤，SQL 仍合法） |
/// | 需要隔离但**无租户上下文** | 报错（fail-closed） |
///
/// # 错误
///
/// - 占位语法错误（如 `/*tenant: 带空格*/`、缺少 `*/`）
/// - 别名或租户列名未通过标识符校验
/// - 需要隔离但无租户上下文
pub(crate) fn rewrite_raw_tenant<T: Model>(
    sql: alun_db_core::query::SqlQuery,
    dialect: &dyn Dialect,
) -> Result<(alun_db_core::query::SqlQuery, bool)> {
    use alun_db_core::query::SqlQuery;

    let (text, mut params) = sql.into_parts();

    // 常见路径：没有占位 → 原样返回（由 ensure_raw_allowed 决定是否放行）
    if !text.contains(TENANT_MARKER) {
        return Ok((SqlQuery::new(text, params), false));
    }

    // 决定本次是否真的注入租户条件
    let pending_value = if !is_tenant_enabled() || T::TENANT.is_none() || T::CROSS_TENANT {
        None
    } else {
        match current_tenant() {
            Some(TenantContext::Scoped(value)) => Some(value),
            Some(TenantContext::Cross) => None,
            None => {
                return Err(DbError::InvalidValue(format!(
                    "raw SQL for model `{}` uses a `{}*/` tenant marker but no tenant \
                     context is set; run it inside `tenant_scope(..)`, or use \
                     `tenant_scope(TenantContext::Cross, ..)` to declare a deliberate \
                     cross-tenant access",
                    T::TABLE,
                    TENANT_MARKER
                )));
            }
        }
    };

    let pieces = scan_raw_sql(&text)?;

    let mut out = String::with_capacity(text.len() + 32);
    // `?` 只统计**原有**占位符：占位替换出的新谓词不参与原参数序列的定位
    let mut original_placeholders = 0usize;
    let mut insertions: Vec<(usize, Value)> = Vec::new();
    let mut bound = false;

    for piece in pieces {
        match piece {
            RawPiece::Text(chunk) => out.push_str(chunk),
            RawPiece::Placeholder => {
                out.push('?');
                original_placeholders += 1;
            }
            RawPiece::Marker(alias) => {
                match (pending_value.as_ref(), T::TENANT) {
                    (Some(value), Some(column)) => {
                        // 列名与别名都必须过标识符校验（fail-closed，绝不拼未校验的名字）
                        alun_db_sql::render::validate_identifier(column)?;
                        let quoted_column = dialect.quote_identifier(column);
                        let predicate = match alias {
                            Some(alias) => {
                                alun_db_sql::render::validate_identifier(alias)?;
                                format!(
                                    "({}.{} = ?)",
                                    dialect.quote_identifier(alias),
                                    quoted_column
                                )
                            }
                            None => format!("({quoted_column} = ?)"),
                        };
                        out.push_str(&predicate);
                        // 新值在最终参数序列中的位置 = 已扫过的原有占位符数
                        // + 此前已计划的插入数（插入会把后续元素整体后移）
                        insertions.push((original_placeholders + insertions.len(), value.clone()));
                        bound = true;
                    }
                    // 不需要隔离（跨租户 / 模型无租户列 / 开关关闭）：
                    // 换成恒真谓词，保证 SQL 结构不变且语义正确
                    _ => out.push_str("(1 = 1)"),
                }
            }
        }
    }

    for (index, value) in insertions {
        if index > params.len() {
            return Err(DbError::InvalidValue(format!(
                "internal error: tenant placeholder index {index} exceeds {} bind parameters",
                params.len()
            )));
        }
        params.insert(index, value);
    }

    Ok((SqlQuery::new(out, params), bound))
}

/// 扫描结果片段。
enum RawPiece<'a> {
    /// 原样保留的 SQL 文本
    Text(&'a str),
    /// `?` 占位符
    Placeholder,
    /// `/*tenant*/` 或 `/*tenant:别名*/`
    Marker(Option<&'a str>),
}

/// 单遍扫描：识别字符串字面量 / 注释 / 占位符 / 租户标记。
///
/// 必须跳过引号与注释内的内容，否则字符串里的 `?` 会打乱参数序号、
/// 注释里的“`/*tenant*/`”字样会被误当标记。
fn scan_raw_sql(sql: &str) -> Result<Vec<RawPiece<'_>>> {
    const PREFIX: &str = "/*tenant";

    let bytes = sql.as_bytes();
    let n = bytes.len();
    let mut pieces: Vec<RawPiece<'_>> = Vec::new();
    let mut text_start = 0usize;
    let mut i = 0usize;

    while i < n {
        match bytes[i] {
            // 单引号字符串（`''` 为转义）
            b'\'' => {
                i += 1;
                while i < n {
                    if bytes[i] == b'\'' {
                        if i + 1 < n && bytes[i + 1] == b'\'' {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            // 双引号标识符 / MySQL 反引号标识符
            b'"' | b'`' => {
                let quote = bytes[i];
                i += 1;
                while i < n {
                    if bytes[i] == quote {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            // 行注释 `-- ...`
            b'-' if i + 1 < n && bytes[i + 1] == b'-' => {
                i += 2;
                while i < n && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            // 块注释，或租户标记
            b'/' if i + 1 < n && bytes[i + 1] == b'*' => {
                if sql[i..].starts_with(PREFIX) {
                    let rest = &sql[i + PREFIX.len()..];
                    let (alias, consumed) = parse_marker_tail(rest)?;
                    if i > text_start {
                        pieces.push(RawPiece::Text(&sql[text_start..i]));
                    }
                    pieces.push(RawPiece::Marker(alias));
                    i += PREFIX.len() + consumed;
                    text_start = i;
                } else {
                    i += 2;
                    while i + 1 < n && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                        i += 1;
                    }
                    i = (i + 2).min(n);
                }
            }
            b'?' => {
                if i > text_start {
                    pieces.push(RawPiece::Text(&sql[text_start..i]));
                }
                pieces.push(RawPiece::Placeholder);
                i += 1;
                text_start = i;
            }
            _ => i += 1,
        }
    }

    if text_start < n {
        pieces.push(RawPiece::Text(&sql[text_start..]));
    }
    Ok(pieces)
}

/// 解析标记尾部（`rest` 紧跟 `/*tenant` 之后）。
///
/// # 返回
///
/// `(别名, 已消费字节数)`——别名与结尾的 `*/` 都计入消费字节数。
fn parse_marker_tail(rest: &str) -> Result<(Option<&str>, usize)> {
    let malformed = || {
        DbError::InvalidValue(format!(
            "malformed tenant marker: expected `{TENANT_MARKER}*/` or \
             `{TENANT_MARKER}:别名*/` (no spaces, alias must match [A-Za-z_][A-Za-z0-9_]*)"
        ))
    };

    let bytes = rest.as_bytes();
    // 无别名
    if rest.starts_with("*/") {
        return Ok((None, 2));
    }
    if bytes.first() != Some(&b':') {
        return Err(malformed());
    }

    let mut end = 1usize;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    if end == 1 {
        return Err(malformed());
    }
    let alias = &rest[1..end];
    if !rest[end..].starts_with("*/") {
        return Err(malformed());
    }
    Ok((Some(alias), end + 2))
}

/// 计算当前租户对应的 PostgreSQL schema 名（schema-per-tenant 模式）。
///
/// 由 `PostgreSQL` 执行器在借出连接后调用，用于 `SET search_path`。
///
/// # 返回
///
/// - 总开关或 schema 开关关闭 → `None`
/// - 无租户上下文 / 显式跨租户（[`TenantContext::Cross`]）→ `None`（用默认 `search_path`）
/// - 限定租户（`Scoped`）→ `Some("<前缀><租户ID>")`
///
/// # 错误
///
/// 租户 ID 类型无法作为 schema 名片段（既非整数也非文本），或派生出的 schema 名
/// 含非法字符（仅允许 `[A-Za-z0-9_-]`，长度 ≤ 63）时返回 [`DbError::InvalidValue`]
/// ——fail-closed，绝不用未经校验的名字拼 SQL。
#[cfg(feature = "postgres")]
pub(crate) fn pg_schema_for_current_tenant() -> Result<Option<String>> {
    if !is_tenant_enabled() || !is_tenant_schema_enabled() {
        return Ok(None);
    }
    let Some(TenantContext::Scoped(value)) = current_tenant() else {
        // 无上下文或显式跨租户 → 使用默认 search_path
        return Ok(None);
    };
    let schema = format!("{}{}", tenant_schema_prefix(), schema_suffix(&value)?);
    if !is_valid_schema_name(&schema) {
        return Err(DbError::InvalidValue(format!(
            "invalid tenant schema name `{schema}`: only [A-Za-z0-9_-] within 63 bytes allowed"
        )));
    }
    Ok(Some(schema))
}

/// 将租户 ID 值渲染为 schema 名片段（仅整数与文本可映射）。
#[cfg(feature = "postgres")]
fn schema_suffix(value: &Value) -> Result<String> {
    let suffix = match value {
        Value::I8(v) => v.to_string(),
        Value::I16(v) => v.to_string(),
        Value::I32(v) => v.to_string(),
        Value::I64(v) => v.to_string(),
        Value::U8(v) => v.to_string(),
        Value::U16(v) => v.to_string(),
        Value::U32(v) => v.to_string(),
        Value::U64(v) => v.to_string(),
        Value::Text(v) => v.clone(),
        other => {
            return Err(DbError::InvalidValue(format!(
                "tenant schema isolation requires an integer or text tenant id, got {other:?}"
            )));
        }
    };
    Ok(suffix)
}

/// schema 名是否合法：非空、长度 ≤ 63 字节、字符集 `[A-Za-z0-9_-]`。
#[cfg(feature = "postgres")]
fn is_valid_schema_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(all(test, feature = "postgres"))]
mod tests {
    use super::*;

    /// 覆盖 schema 名派生与非法输入的 fail-closed 校验。
    ///
    /// 两个开关是进程级全局状态，为免同进程内并行测例互相干扰，本模块只放
    /// **一个**测试函数；用例结束时恢复默认值。
    #[tokio::test]
    async fn pg_schema_name_derivation_is_validated() {
        // 关闭：任何上下文都不产生 schema
        set_tenant_enabled(true);
        set_tenant_schema_enabled(false);
        let disabled = tenant_scope(TenantContext::Scoped(Value::I64(42)), async {
            pg_schema_for_current_tenant()
        })
        .await
        .expect("schema 开关关闭时不应报错");
        assert!(disabled.is_none(), "schema 开关关闭时应返回 None");

        // 开启 + 整数租户 → 前缀 + id
        set_tenant_schema_enabled(true);
        set_tenant_schema_prefix("t_");
        let named = tenant_scope(TenantContext::Scoped(Value::I64(42)), async {
            pg_schema_for_current_tenant()
        })
        .await
        .expect("整数租户 ID");
        assert_eq!(named.as_deref(), Some("t_42"));

        // 文本租户同样支持
        let text = tenant_scope(TenantContext::Scoped(Value::Text("acme".into())), async {
            pg_schema_for_current_tenant()
        })
        .await
        .expect("文本租户 ID");
        assert_eq!(text.as_deref(), Some("t_acme"));

        // 显式跨租户 / 无上下文 → None（默认 search_path）
        let cross = tenant_scope(TenantContext::Cross, async {
            pg_schema_for_current_tenant()
        })
        .await
        .expect("跨租户上下文");
        assert!(cross.is_none());
        assert!(
            pg_schema_for_current_tenant().expect("无上下文").is_none(),
            "无上下文时应回退默认 search_path"
        );

        // 注入型租户名 → Err（fail-closed）
        let evil = tenant_scope(
            TenantContext::Scoped(Value::Text("x\"; DROP SCHEMA public; --".into())),
            async { pg_schema_for_current_tenant() },
        )
        .await;
        assert!(evil.is_err(), "非法 schema 名必须报错，实际 {evil:?}");

        // 恢复默认
        set_tenant_schema_prefix("");
        set_tenant_schema_enabled(false);
    }
}

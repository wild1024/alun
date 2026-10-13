//! # alun-db-macro
//!
//! `alun-db` 的过程宏实现。提供：
//!
//! - `#[derive(Model)]` —— 生成 `Model` 实现、字段表达式、Hook 注册
//! - `#[derive(Row)]` —— 生成 `Row` 转换与 serde 支持
//! - `#[derive(TypeMapper)]` —— 为枚举生成 `TypeMapper` 实现
//! - `sql!` —— 编译期校验的 SQL 片段拼接宏
//!
//! ## 设计原则
//!
//! 1. **绝对路径**：生成代码中所有类型引用使用 `::alun_db_core::` 或
//!    `::std::` 绝对路径，避免调用方的导入影响宏展开；其中 `::alun_db_core`
//!    前缀在导出前按调用方实际声明的 crate 重写（见 [`resolve`]），因此
//!    只依赖 `alun` 或 `alun-db` 的项目无需再声明 `alun-db-core`
//! 2. **编译期错误**：所有校验错误通过 `syn::Error::to_compile_error()`
//!    返回，带 span 指向问题位置
//! 3. **零运行时开销**：字段元数据以 `static` 数组形式生成，编译期确定

#![deny(missing_docs)]
#![warn(clippy::all)]

use proc_macro::TokenStream;

mod attr;
mod field;
mod migrate_macro;
mod model;
mod resolve;
mod row;
mod schema;
mod sql_macro;
mod template;
mod type_mapper;

/// `#[derive(Model)]`：为结构体生成 `Model` trait 实现、字段表达式访问器、
/// Hook 注册和 `Crud`/`Persist` 的 blank impl 所需元数据。
///
/// ## 支持的属性
///
/// - `#[table("name")]` —— 指定表名（缺省时由结构体名推导：`SysUser` → `sys_user`）
/// - `#[view("name")]` —— 指定只读视图名
/// - `#[hook(Type)]` —— 注册同步 Hook（`before_*` / `after_*`，语句级）
/// - `#[hook_async(Type)]` —— 注册异步 Hook（仅 `after_*`，**提交后**触发）
/// - `#[key]` —— 标记主键字段（缺省时名为 `id` 的字段即主键）
/// - `#[auto]` —— 标记自增字段（缺省时名为 `id` 的整型主键即自增）
/// - `#[unique]` —— 标记唯一字段（写入 `FieldMeta::is_unique`，供 schema 校验与快照使用）
/// - `#[column("name")]` —— 指定列名
/// - `#[json]` —— 标记 JSON 字段
/// - `#[soft_delete]` —— 标记软删除列：delete 改写为 UPDATE，查询自动过滤已删行
///   （缺省时 `Option<...>` 的 `deleted_at` 字段即软删标记）
/// - `#[model(strict)]` —— 启用编译期 schema 校验
/// - `#[model(hard_delete)]` —— 关闭 `deleted_at` 自动软删，`delete` 走物理删除
/// - `#[model(no_timestamps)]` —— 关闭 `created_at` / `updated_at` 自动挂载
///   `TimestampHook`（时间戳由调用方维护，适用于业务时间列）
/// - `#[model(schema = "path")]` —— 严格校验所用 DDL 文件
///
/// ## 自动挂载
///
/// 含 `created_at` / `updated_at` 字段的模型自动挂载
/// [`TimestampHook`](alun_db_core::hook::TimestampHook)，无需手写
/// `#[hook(TimestampHook)]`；该 Hook **强制覆盖**这两个列（语义对齐 Rails
/// `timestamps`），需自行控制取值的模型用 `#[model(no_timestamps)]` 关闭。
#[proc_macro_derive(
    Model,
    attributes(
        table,
        view,
        hook,
        hook_async,
        key,
        auto,
        unique,
        column,
        json,
        soft_delete,
        tenant,
        model
    )
)]
pub fn derive_model(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as syn::DeriveInput);
    match model::derive(ast) {
        Ok(tokens) => resolve::relocate(tokens).into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `#[derive(Row)]`：为结构体生成与 `alun_db_core::row::Row` 的双向转换。
///
/// 通常与 `#[derive(serde::Serialize, serde::Deserialize)]` 配合使用，
/// 用于 JSON 字段的嵌套结构。
#[proc_macro_derive(Row)]
pub fn derive_row(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as syn::DeriveInput);
    match row::derive(ast) {
        Ok(tokens) => resolve::relocate(tokens).into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `#[derive(TypeMapper)]`：为枚举生成 `TypeMapper` 实现（仅字段为空的枚举）。
///
/// 四种编码模式（同时派生 `From<Enum> for Value` 与 `TryFrom<&Value>`）：
///
/// - 默认（文本）模式 —— 变体名即编码，落 `Value::Text`，读回只接受文本载荷
/// - `#[int]` —— 判别值编码，写 `Value::I32`；读回接受任意宽度整数
///   （`I8`/`I16`/`I32`/`I64`，驱动读整数列时会归一宽度），未知判别值报
///   `DbError::InvalidValue`，越界或非整数载荷报
///   `DbError::TypeMismatch { expected: "I32", .. }`
/// - `#[value_map("A" => VariantA, "B" => VariantB)]` —— 显式编码表，落 `Value::Text`
/// - `#[native_enum("type_name")]` —— 数据库原生 ENUM 类型，写 `Value::Enum`；
///   读回接受 `Value::Enum`（原生枚举列）与 `Value::Text`（方言退化为文本承载的列），
///   标签须与变体名一致，未知标签报 `DbError::InvalidValue`
#[proc_macro_derive(TypeMapper, attributes(int, value_map, native_enum))]
pub fn derive_type_mapper(input: TokenStream) -> TokenStream {
    let ast = syn::parse_macro_input!(input as syn::DeriveInput);
    match type_mapper::derive(ast) {
        Ok(tokens) => resolve::relocate(tokens).into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `sql!` 宏：拼接 SQL 片段并生成 [`SqlQuery`]（SQL 文本 + 绑定参数），
/// 配合终态动词（`find` / `find_first` / `find_one` / `find_one_or_null` /
/// `exists` / `paginate` / `for_each` / `for_each_page` / `scalar` / `execute`）执行。
///
/// ## 安全模型
///
/// - 插值 `{ expr }` 一律转为占位符 `?` + 绑定参数，值永不内联进 SQL
/// - 裸表达式仅接受 `SqlFragment`（如嵌套的 `sql!` 结果）
/// - `#` 指令（`#where` / `#and` / `#or` / `#orderBy` / `#groupBy` / `#limit` /
///   `#if` / `#else` / `#end` / `#para`）写在字符串字面量内，参数在编译期
///   白名单校验，非法输入直接编译失败
///
/// ## 用法
///
/// ```ignore
/// use alun_db::prelude::*;
///
/// // 插值 → 参数化
/// let users = sql!("SELECT * FROM users WHERE email = ", { email })
///     .find(db()).await?;
///
/// // 动态条件（None 时片段省略）+ 模板指令（写在字符串内）
/// let rows = sql!(
///     "SELECT * FROM users WHERE 1=1",
///     if let Some(n) = &name_filter { " AND name LIKE " { n } },
///     " #and(status, '=', status) #orderBy(created_at desc) #limit(20)",
/// ).find(db()).await?;
///
/// // 字面量内的条件块：`#if` + `#para`
/// let rows = sql!(
///     "SELECT * FROM invoice WHERE tenant_id = ", { tid },
///     " #if(status_filter) AND status = #para(status_filter) #end",
/// ).find(db()).await?;
///
/// // 嵌套组合：子查询的 SQL 与参数成对合并
/// let ids = sql!("SELECT user_id FROM orders WHERE amount > ", { 100.0 });
/// let users = sql!("SELECT * FROM users WHERE id IN (", ids, ")")
///     .find(db()).await?;
/// ```
///
/// [`SqlQuery`]: alun_db_core::query::SqlQuery
#[proc_macro]
pub fn sql(input: TokenStream) -> TokenStream {
    match sql_macro::expand(input) {
        Ok(tokens) => resolve::relocate(tokens).into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `migrate!` 宏：把迁移目录在**编译期**编入二进制，运行时不再依赖磁盘上的 SQL 文件。
///
/// ```ignore
/// use alun_db::prelude::*;
///
/// // 迁移文件被 include_str! 编入二进制
/// let migrator = alun_db::migrate!("migrations");
/// migrator.run(db()).await?;
/// ```
///
/// 生成的是 `Migrator`，因此 `run` / `rollback` / `status` / `plan` / `dry_run` /
/// `baseline` / `repair` 等入口与目录来源完全一致；还可继续链式追加来源：
///
/// ```ignore
/// alun_db::migrate!("migrations").source_seed("seed").run(db()).await?;
/// ```
///
/// ## 文件名规则
///
/// 与目录来源共用同一套解析器（宏只搬运、不解析），因此 `V{n}__Name.up.sql`、
/// `NN_name.sql`、`R__` / `S__` / `B{n}__` 全部照常可用，子目录也会递归收集。
///
/// ## 读的是「相对 crate 根目录」的路径
///
/// 路径相对 `CARGO_MANIFEST_DIR`（即调用方 crate 的根），绝对路径亦可。
///
/// ## 已知限制
///
/// **新增迁移文件不会自动触发重编译**：rustc 只把 `include_str!` 到的各个文件记为
/// 依赖，目录本身不在其中。新增文件后请 `touch` 源文件或 `cargo clean -p <crate>`。
/// （`sqlx::migrate!` 有同样的限制。）
#[proc_macro]
pub fn migrate(input: TokenStream) -> TokenStream {
    match migrate_macro::expand(input) {
        Ok(tokens) => resolve::relocate(tokens).into(),
        Err(e) => e.to_compile_error().into(),
    }
}

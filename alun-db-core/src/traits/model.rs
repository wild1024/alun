//! `Model` trait —— 强类型视图。

use super::hook::{AsyncHook, Hook};
use crate::error::Result;
use crate::field::FieldMeta;
use crate::row::Row;
use crate::stmt::Value;

/// 强类型视图。由 `#[derive(Model)]` 为每个表或视图生成。
///
/// ## 与 `Row` 的关系
///
/// `Model` 是 `Row` 的强类型投影。任何 `Model` 都可以通过 `as_row()`
/// 转为 `Row`，任何 `Row` 都可以通过 `from_row()` 尝试构造 `Model`。
pub trait Model: Sized + Send + Sync + 'static {
    /// 表名或视图名。
    ///
    /// 由 `#[derive(Model)]` 生成：`#[table("...")]` / `#[view("...")]` 显式给出，
    /// 缺省时由结构体名蛇形推导（`SysUser` → `sys_user`）。
    const TABLE: &'static str;

    /// 是否只读。视图为 `true`，表为 `false`。
    ///
    /// 只读 Model 的 `save` / `update` / `delete` 在编译期报错（由宏保证）。
    const READONLY: bool;

    /// 主键字段名（Rust 字段名，非列名）。
    ///
    /// 由 `#[derive(Model)]` 生成：显式 `#[key]` 优先，缺省时名为 `id` 的
    /// 字段即主键。
    const PRIMARY_KEY: &'static str;

    /// 主键字段对应的数据库列名。
    ///
    /// 默认由 [`Self::fields`] 解析：主键字段名与列名不一致
    /// （`#[column(...)]` 重命名）时返回真实列名，一致时等同于
    /// [`Self::PRIMARY_KEY`]。
    fn primary_key_column() -> &'static str {
        Self::fields()
            .iter()
            .find(|f| f.name == Self::PRIMARY_KEY)
            .map(|f| f.column)
            .unwrap_or(Self::PRIMARY_KEY)
    }

    /// 软删除标记列名。`None` 表示该模型未启用软删除。
    ///
    /// 启用后：
    /// - `delete` / `delete_by_pk` / `delete_by` / `delete_in*` / `ChangeSet::delete`
    ///   改写为 `UPDATE SET 本列 = now WHERE ...`（保留数据，仅标记删除）
    /// - 查询侧（`query()` 构造的链与 `CrudExt` 静态糖）自动追加
    ///   `本列 IS NULL` 过滤，屏蔽已删除行
    ///
    /// 由 `#[derive(Model)]` 生成：显式 `#[soft_delete]` 优先；缺省时
    /// `Option<...>` 类型的 `deleted_at` 字段即软删标记，可用
    /// `#[model(hard_delete)]` 关闭。
    const SOFT_DELETE: Option<&'static str> = None;

    /// 租户列名。`None` 表示该模型不参与租户隔离。
    ///
    /// 由 `#[derive(Model)]` 生成：字段标注 `#[tenant]` 时为该字段的**列名**，
    /// 结构体标注 `#[model(cross_tenant)]` 或未标注任何 `#[tenant]` 字段时为 `None`。
    ///
    /// 启用后，读路径自动追加 `本列 = 当前租户`、写路径自动补齐本列；
    /// 与之互斥的 [`Self::CROSS_TENANT`] 提供显式豁免。
    const TENANT: Option<&'static str> = None;

    /// 是否显式声明跨租户（全局表 / 管理表）。
    ///
    /// 与 [`Self::TENANT`] 互斥：本常量为 `true` 时 `TENANT` 必须为 `None`，
    /// 由 `#[derive(Model)]` 在编译期校验。声明后该模型的所有读写都不注入
    /// 租户条件（即使请求上下文限定了租户）。
    const CROSS_TENANT: bool = false;

    /// 字段元数据列表。顺序与 `from_row` / `as_row` 的字段顺序一致。
    fn fields() -> &'static [FieldMeta];

    /// 从行数据构造 Model。
    ///
    /// 若行缺少字段或类型不匹配，返回 `DbError`。
    fn from_row(row: Row) -> Result<Self>;

    /// 转为行数据。
    ///
    /// # 错误
    ///
    /// `#[json]` 字段序列化失败（无法编码为 JSON 字符串）时返回
    /// [`crate::error::DbError::InvalidValue`]。
    fn as_row(&self) -> Result<Row>;

    /// 按 [`Self::fields`] 的顺序把自身字段值写入 `out`（**只值、不含字段名**）。
    ///
    /// 批量写入路径要的是「按列顺序的一串值」，若先走 [`Self::as_row`] 再取值，
    /// 每行都要构造完整 `Row`——包括逐字段的**字段名字符串分配**，而写入根本
    /// 用不到名字。本方法让 `#[derive(Model)]` 直接写值，省掉那份开销。
    ///
    /// `out` 先被清空再写入，调用方可复用它以避免逐行分配。
    ///
    /// # 错误
    ///
    /// 同 [`Self::as_row`]：`#[json]` 字段序列化失败时返回
    /// [`crate::error::DbError::InvalidValue`]。
    fn field_values_into(&self, out: &mut Vec<Value>) -> Result<()> {
        let row = self.as_row()?;
        out.clear();
        out.reserve(Self::fields().len());
        for index in 0..Self::fields().len() {
            out.push(row.get_at(index).cloned().unwrap_or(Value::Null));
        }
        Ok(())
    }

    /// 主键值。
    fn pk_value(&self) -> Value;

    /// 构造只包含主键的最小行。用于 `delete_by_id` 等场景。
    fn pk_row(value: Value) -> Row {
        let mut r = Row::with_capacity(1);
        r.insert(Self::PRIMARY_KEY, value);
        r
    }

    /// Hook 列表。默认空。由 `#[hook(...)]` 属性生成。
    fn hooks() -> &'static [&'static dyn Hook] {
        &[]
    }

    /// 异步 Hook 列表。默认空。由 `#[hook_async(...)]` 属性生成。
    ///
    /// 与 [`hooks`](Self::hooks) 的区别：这些钩子在 **CRUD 提交后**触发
    /// （`&Transaction` 下排队至 `commit()` 成功后；`rollback()` 丢弃），
    /// 适合审计 / 缓存失效 / 通知等异步副作用。详见 [`AsyncHook`]。
    fn async_hooks() -> &'static [&'static dyn AsyncHook] {
        &[]
    }
}

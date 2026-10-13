//! 属性解析。
//!
//! 使用 syn 2.0 的 `parse_nested_meta` 解析结构体和字段上的属性。
//! 参考 syn 文档中的属性解析示例。[reference:1]

use proc_macro2::Span;
use syn::{Attribute, LitStr, Path};

/// 结构体级属性。
#[derive(Debug, Default)]
pub struct StructAttrs {
    /// 表名（`#[table("...")]`）。缺省时由结构体名推导。
    pub table: Option<String>,
    /// 视图名（`#[view("...")]`）。
    pub view: Option<String>,
    /// Hook 列表（`#[hook(Type)]`）。
    pub hooks: Vec<Path>,
    /// 异步 Hook 列表（`#[hook_async(Type)]`）。
    ///
    /// 在 CRUD **提交后**触发（事务内排队至 `commit()` 成功后；回滚丢弃）。
    pub async_hooks: Vec<Path>,
    /// 是否启用严格 schema 校验（`#[model(strict)]`）。
    pub strict: bool,
    /// 严格校验所指的 DDL schema 文件路径（`#[model(strict, schema = "...")]`）。
    pub schema: Option<String>,
    /// 是否禁用 `deleted_at` 自动软删（`#[model(hard_delete)]`）。
    pub hard_delete: bool,
    /// 是否禁用 `created_at` / `updated_at` 自动挂载 `TimestampHook`
    /// （`#[model(no_timestamps)]`）。
    pub no_timestamps: bool,
    /// 是否显式声明跨租户（`#[model(cross_tenant)]`）。
    ///
    /// 声明后该模型不参与行级租户隔离；与任何 `#[tenant]` 字段互斥。
    pub is_cross_tenant: bool,
    /// 是否只读（`#[model(readonly)]`）。
    ///
    /// 声明后 [`Model::READONLY`] 为 `true`，写路径（insert/update/delete）
    /// 一律拒绝（见 `alun-db/src/persist.rs`）。
    /// 视图（`#[view("...")]`）隐式只读，无需重复声明。
    ///
    /// [`Model::READONLY`]: alun_db_core::traits::Model::READONLY
    pub is_readonly: bool,
}

/// 字段级属性。
#[derive(Debug, Default)]
pub struct FieldAttrs {
    /// 是否为主键（`#[key]`）。
    pub is_key: bool,
    /// 是否自增（`#[auto]`）。
    pub is_auto: bool,
    /// 是否唯一（`#[unique]`）。
    pub is_unique: bool,
    /// 列名覆盖（`#[column("...")]`）。
    pub column: Option<String>,
    /// 是否 JSON 字段（`#[json]`）。
    pub is_json: bool,
    /// 是否 JSON 可空（`#[json(nullable)]`）。
    pub json_nullable: bool,
    /// 是否软删除标记列（`#[soft_delete]`）。
    pub soft_delete: bool,
    /// 是否为租户列（`#[tenant]`）。
    ///
    /// 声明后该模型参与行级租户隔离，此字段的列名即隔离列。
    pub is_tenant: bool,
}

/// 解析结构体级属性。
pub fn parse_struct_attrs(attrs: &[Attribute]) -> syn::Result<StructAttrs> {
    let mut result = StructAttrs::default();

    for attr in attrs {
        if attr.path().is_ident("table") {
            let lit: LitStr = attr.parse_args()?;
            result.table = Some(lit.value());
        } else if attr.path().is_ident("view") {
            let lit: LitStr = attr.parse_args()?;
            result.view = Some(lit.value());
        } else if attr.path().is_ident("hook") {
            let path: Path = attr.parse_args()?;
            result.hooks.push(path);
        } else if attr.path().is_ident("hook_async") {
            let path: Path = attr.parse_args()?;
            result.async_hooks.push(path);
        } else if attr.path().is_ident("model") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("strict") {
                    result.strict = true;
                } else if meta.path.is_ident("hard_delete") {
                    result.hard_delete = true;
                } else if meta.path.is_ident("no_timestamps") {
                    result.no_timestamps = true;
                } else if meta.path.is_ident("cross_tenant") {
                    result.is_cross_tenant = true;
                } else if meta.path.is_ident("readonly") {
                    result.is_readonly = true;
                } else if meta.path.is_ident("schema") {
                    let lit: LitStr = meta.value()?.parse()?;
                    result.schema = Some(lit.value());
                } else {
                    return Err(meta.error(
                        "unknown model attribute (expected `strict`, `hard_delete`, \
                         `no_timestamps`, `readonly`, `cross_tenant` or `schema`)",
                    ));
                }
                Ok(())
            })?;
        }
    }

    // 校验：table 和 view 不能同时存在
    if result.table.is_some() && result.view.is_some() {
        return Err(syn::Error::new(
            Span::call_site(),
            "`#[table]` and `#[view]` cannot be used together",
        ));
    }

    Ok(result)
}

/// 解析字段级属性。
pub fn parse_field_attrs(attrs: &[Attribute]) -> syn::Result<FieldAttrs> {
    let mut result = FieldAttrs::default();

    for attr in attrs {
        if attr.path().is_ident("key") {
            result.is_key = true;
        } else if attr.path().is_ident("auto") {
            result.is_auto = true;
        } else if attr.path().is_ident("unique") {
            result.is_unique = true;
        } else if attr.path().is_ident("soft_delete") {
            result.soft_delete = true;
        } else if attr.path().is_ident("tenant") {
            result.is_tenant = true;
        } else if attr.path().is_ident("column") {
            let lit: LitStr = attr.parse_args()?;
            result.column = Some(lit.value());
        } else if attr.path().is_ident("json") {
            // 检查是裸 #[json] 还是 #[json(nullable)]
            let tokens = attr.meta.require_list();
            if tokens.is_ok() {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("nullable") {
                        result.json_nullable = true;
                    } else {
                        return Err(meta.error("unknown json attribute"));
                    }
                    Ok(())
                })?;
            }
            result.is_json = true;
        }
    }

    Ok(result)
}

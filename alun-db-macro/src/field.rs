//! 字段元数据收集。

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Type};

use crate::attr::{FieldAttrs, StructAttrs};

/// 单个字段的完整元数据。
pub struct FieldInfo {
    /// Rust 字段名（Ident）。
    pub ident: syn::Ident,
    /// Rust 字段名（字符串）。
    pub name: String,
    /// 数据库列名。
    pub column: String,
    /// Rust 类型（用于生成字段表达式）。
    pub ty: Type,
    /// 是否可空（`Option<T>`）。
    pub nullable: bool,
    /// 字段属性。
    pub attrs: FieldAttrs,
}

/// 收集结构体的所有字段信息，并应用默认约定：
///
/// - **表名**：`#[table("...")]` 缺省时由结构体名推导（见 [`to_snake_case`]）
/// - **主键**：`#[key]` 缺省时回退到名为 `id` 的字段
/// - **自增**：`#[auto]` 缺省时，名为 `id` 的整型主键默认自增
/// - **软删除**：`#[soft_delete]` 缺省时，`Option<...>` 类型的 `deleted_at`
///   字段默认作为软删标记（`#[model(hard_delete)]` 可关闭）
pub fn collect_fields(
    ast: &DeriveInput,
    struct_attrs: &StructAttrs,
) -> syn::Result<Vec<FieldInfo>> {
    let fields = match &ast.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    ast,
                    "`#[derive(Model)]` only supports structs with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                ast,
                "`#[derive(Model)]` only supports structs",
            ));
        }
    };

    let mut result = Vec::new();
    for field in fields {
        let ident = field.ident.as_ref().unwrap();
        let name = ident.to_string();
        let field_attrs = crate::attr::parse_field_attrs(&field.attrs)?;

        let column = field_attrs.column.clone().unwrap_or_else(|| name.clone());
        let nullable = is_option_type(&field.ty);

        result.push(FieldInfo {
            ident: ident.clone(),
            name,
            column,
            ty: field.ty.clone(),
            nullable,
            attrs: field_attrs,
        });
    }

    // ---- 主键：显式 `#[key]` 优先；缺省回退到名为 `id` 的字段 ----
    let mut keys: Vec<usize> = result
        .iter()
        .enumerate()
        .filter(|(_, f)| f.attrs.is_key)
        .map(|(i, _)| i)
        .collect();
    if keys.is_empty()
        && let Some(idx) = result.iter().position(|f| f.name == "id")
    {
        result[idx].attrs.is_key = true;
        keys.push(idx);
    }
    match keys.len() {
        0 => {
            return Err(syn::Error::new_spanned(
                ast,
                "`#[derive(Model)]` requires a primary key: mark a field with `#[key]`, \
                 or name the primary key field `id`",
            ));
        }
        1 => {}
        _ => {
            return Err(syn::Error::new_spanned(
                &result[keys[1]].ident,
                "only one `#[key]` field is allowed",
            ));
        }
    }

    // ---- 自增：显式 `#[auto]` 优先；缺省在「名为 id 的整型主键」上默认开启 ----
    // 取值时只在主键值为占位（Null / 0）时排除该列（见 persist 层），
    // 因此显式赋值的非零主键（雪花 ID 等）仍会正常写入。
    let key = &result[keys[0]];
    if !key.attrs.is_auto && key.name == "id" && is_integer_type(&key.ty) {
        result[keys[0]].attrs.is_auto = true;
    }

    // 校验：`#[auto]` 只能用于主键
    for field in &result {
        if field.attrs.is_auto && !field.attrs.is_key {
            return Err(syn::Error::new_spanned(
                &field.ident,
                "`#[auto]` can only be used on the `#[key]` field",
            ));
        }
    }

    // ---- 软删除：显式 `#[soft_delete]` 优先；缺省在 `Option` 的 `deleted_at` 上开启 ----
    let soft: Vec<usize> = result
        .iter()
        .enumerate()
        .filter(|(_, f)| f.attrs.soft_delete)
        .map(|(i, _)| i)
        .collect();
    if soft.len() > 1 {
        return Err(syn::Error::new_spanned(
            &result[soft[1]].ident,
            "at most one `#[soft_delete]` field is allowed",
        ));
    }
    if !soft.is_empty() && struct_attrs.hard_delete {
        return Err(syn::Error::new_spanned(
            ast,
            "`#[model(hard_delete)]` conflicts with an explicit `#[soft_delete]` field",
        ));
    }
    if soft.is_empty()
        && !struct_attrs.hard_delete
        && let Some(idx) = result.iter().position(|f| f.name == "deleted_at")
    {
        if !result[idx].nullable {
            return Err(syn::Error::new_spanned(
                &result[idx].ident,
                "a `deleted_at` field is treated as a soft-delete marker and must be \
                 `Option<...>`; add `#[model(hard_delete)]` for hard deletes, or \
                 `#[column(\"...\")]` if the column means something else",
            ));
        }
        result[idx].attrs.soft_delete = true;
    }

    Ok(result)
}

/// 由结构体名推导表名（`SysUser` → `sys_user`）。
///
/// 规则：在小写字母 / 数字之后的大写字母前，以及连续大写字母串的最后一个
/// 大写字母前（其后紧跟小写字母）插入 `_`；其余字符转小写。
/// 需要不同表名时用 `#[table("...")]` 显式覆盖。
pub fn to_snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_uppercase() {
            out.push(c);
            continue;
        }
        let boundary = match chars.get(i.wrapping_sub(1)) {
            // 小写字母 / 数字后的大写（aB、2B）
            Some(p) if p.is_lowercase() || p.is_ascii_digit() => true,
            // 连续大写串的最后一个（ABc → a_bc）
            Some(p) if p.is_uppercase() => chars.get(i + 1).is_some_and(|n| n.is_lowercase()),
            _ => false,
        };
        if boundary && !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// 是否为整数家族类型（自增主键的默认判定依据）。
fn is_integer_type(ty: &Type) -> bool {
    matches!(
        extract_type_name(ty).as_deref(),
        Some("i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize")
    )
}

/// 判断类型是否为 `Option<T>`。
fn is_option_type(ty: &Type) -> bool {
    if let Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
    {
        return segment.ident == "Option";
    }
    false
}

/// 生成每个字段的 `FieldMeta` 构造表达式（运行时构造，允许 `Box` 分配）。
pub fn generate_field_meta(fields: &[FieldInfo]) -> syn::Result<Vec<TokenStream>> {
    let mut entries = Vec::new();

    for field in fields {
        let name = &field.name;
        let column = &field.column;
        let nullable = field.nullable;
        let is_key = field.attrs.is_key;
        let is_auto = field.attrs.is_auto;
        let is_unique = field.attrs.is_unique;

        let ty_tokens = rust_type_to_stmt_type(&field.ty, field.attrs.is_json)?;

        entries.push(quote! {
            ::alun_db_core::field::FieldMeta {
                name: #name,
                column: #column,
                ty: #ty_tokens,
                nullable: #nullable,
                is_key: #is_key,
                is_auto: #is_auto,
                is_unique: #is_unique,
                precision: None,
                scale: None,
            }
        });
    }

    Ok(entries)
}

/// 将 Rust 类型映射为 `stmt::Type` 的构造表达式。
///
/// 注意：这里生成的是**运行时构造**的表达式，但因为 `stmt::Type` 的所有
/// 变体都是无字段或含 `&'static str`/`Box<Type>`，而 `FieldMeta.ty` 是
/// `stmt::Type` 而非 `const`，所以可以在 `static` 初始化中使用。
fn rust_type_to_stmt_type(ty: &Type, is_json: bool) -> syn::Result<TokenStream> {
    if is_json {
        // `#[json]` 字段若外层是 `Option<T>`，列类型仍需 nullable。
        if is_option_type(ty) {
            return Ok(quote! {
                ::alun_db_core::stmt::Type::Json.nullable()
            });
        }
        return Ok(quote! { ::alun_db_core::stmt::Type::Json });
    }

    let type_name = extract_type_name(ty);

    Ok(match type_name.as_deref() {
        Some("bool") => quote! { ::alun_db_core::stmt::Type::Bool },
        Some("i8") => quote! { ::alun_db_core::stmt::Type::I8 },
        Some("i16") => quote! { ::alun_db_core::stmt::Type::I16 },
        Some("i32") => quote! { ::alun_db_core::stmt::Type::I32 },
        Some("i64") => quote! { ::alun_db_core::stmt::Type::I64 },
        Some("u8") => quote! { ::alun_db_core::stmt::Type::U8 },
        Some("u16") => quote! { ::alun_db_core::stmt::Type::U16 },
        Some("u32") => quote! { ::alun_db_core::stmt::Type::U32 },
        Some("u64") => quote! { ::alun_db_core::stmt::Type::U64 },
        Some("f32") => quote! { ::alun_db_core::stmt::Type::F32 },
        Some("f64") => quote! { ::alun_db_core::stmt::Type::F64 },
        Some("String") | Some("str") => quote! { ::alun_db_core::stmt::Type::Text },
        Some("Vec") => quote! { ::alun_db_core::stmt::Type::Bytes }, // Vec<u8> 视为 Bytes
        Some("Option") => {
            // Option<T> 的内层类型
            if let Type::Path(tp) = ty
                && let Some(seg) = tp.path.segments.last()
                && let syn::PathArguments::AngleBracketed(args) = &seg.arguments
                && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
            {
                let inner_ty = rust_type_to_stmt_type(inner, false)?;
                return Ok(quote! { (#inner_ty).nullable() });
            }
            quote! { ::alun_db_core::stmt::Type::Unknown }
        }
        Some("DateTime") | Some("Timestamp") => quote! { ::alun_db_core::stmt::Type::TimestampTz },
        Some("NaiveDateTime") => quote! { ::alun_db_core::stmt::Type::Timestamp },
        Some("NaiveDate") => quote! { ::alun_db_core::stmt::Type::Date },
        Some("NaiveTime") => quote! { ::alun_db_core::stmt::Type::Time },
        Some("Uuid") => quote! { ::alun_db_core::stmt::Type::Uuid },
        Some("Ulid") => quote! { ::alun_db_core::stmt::Type::Ulid },
        Some("Decimal") | Some("BigDecimal") => quote! { ::alun_db_core::stmt::Type::Decimal },
        Some("JsonValue") | Some("Value") => quote! { ::alun_db_core::stmt::Type::Json },
        Some("IpAddr") => quote! { ::alun_db_core::stmt::Type::Inet },
        Some("IpNetwork") => quote! { ::alun_db_core::stmt::Type::Cidr },
        _ => quote! { ::alun_db_core::stmt::Type::Unknown },
    })
}

/// 提取类型名（取路径最后一段）。
fn extract_type_name(ty: &Type) -> Option<String> {
    if let Type::Path(tp) = ty {
        tp.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

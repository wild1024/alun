//! `#[derive(Model)]` 代码生成。

use proc_macro2::TokenStream;
use quote::quote;
use syn::DeriveInput;

use crate::attr::parse_struct_attrs;
use crate::field::{collect_fields, generate_field_meta};

/// 生成 `Model` 实现、字段表达式和 Hook 注册。
pub fn derive(ast: DeriveInput) -> syn::Result<TokenStream> {
    let struct_attrs = parse_struct_attrs(&ast.attrs)?;
    let fields = collect_fields(&ast, &struct_attrs)?;

    let name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    // 表名或视图名：`#[table]` / `#[view]` 显式给出，否则由结构体名推导
    let table_name = match (struct_attrs.table.as_deref(), struct_attrs.view.as_deref()) {
        (Some(t), _) => t.to_string(),
        (_, Some(v)) => v.to_string(),
        _ => crate::field::to_snake_case(&ast.ident.to_string()),
    };
    // 只读：视图（`#[view]`）隐式只读；表亦可经 `#[model(readonly)]` 显式声明
    let is_readonly = struct_attrs.view.is_some() || struct_attrs.is_readonly;

    // 主键字段
    let pk_field = fields.iter().find(|f| f.attrs.is_key).ok_or_else(|| {
        syn::Error::new(ast.ident.span(), "model must have exactly one #[key] field")
    })?;
    let pk_name = &pk_field.name;
    let pk_ident = &pk_field.ident;

    // 软删除标记列：至多一个 `#[soft_delete]` 字段，取其在数据库中的列名
    let soft_delete_fields: Vec<&crate::field::FieldInfo> =
        fields.iter().filter(|f| f.attrs.soft_delete).collect();
    if soft_delete_fields.len() > 1 {
        return Err(syn::Error::new_spanned(
            &soft_delete_fields[1].ident,
            "at most one `#[soft_delete]` field is allowed",
        ));
    }
    let soft_delete_const = match soft_delete_fields.first() {
        Some(f) => {
            let col = &f.column;
            quote! { const SOFT_DELETE: Option<&'static str> = Some(#col); }
        }
        None => quote! { const SOFT_DELETE: Option<&'static str> = None; },
    };

    // ---- 多租户：至多一个 `#[tenant]` 字段，且与 `#[model(cross_tenant)]` 互斥 ----
    // 租户列只需列名（类型无关，值统一以 `Value` 承载），故这里只取 `column`。
    let tenant_fields: Vec<&crate::field::FieldInfo> =
        fields.iter().filter(|f| f.attrs.is_tenant).collect();
    if tenant_fields.len() > 1 {
        return Err(syn::Error::new_spanned(
            &tenant_fields[1].ident,
            "at most one `#[tenant]` field is allowed",
        ));
    }
    if struct_attrs.is_cross_tenant && !tenant_fields.is_empty() {
        return Err(syn::Error::new_spanned(
            &tenant_fields[0].ident,
            "`#[tenant]` conflicts with `#[model(cross_tenant)]`: a model either \
             participates in tenant isolation or is declared cross-tenant",
        ));
    }
    let tenant_const = match tenant_fields.first() {
        Some(f) => {
            let col = &f.column;
            quote! { const TENANT: Option<&'static str> = Some(#col); }
        }
        None => quote! { const TENANT: Option<&'static str> = None; },
    };
    let cross_tenant_const = {
        let is_cross_tenant = struct_attrs.is_cross_tenant;
        quote! { const CROSS_TENANT: bool = #is_cross_tenant; }
    };

    // 编译期 schema 校验：`#[model(strict)]` 时校验列名存在性与列类型族兼容。
    // 优先用显式 `schema = "..."`（相对 CARGO_MANIFEST_DIR）的文件；
    // 未显式给出时，回退读取 build.rs 快照（`ALUN_DB_SCHEMA_SNAPSHOT`，
    // 由 `alun_db_build::build_snapshot` 输出）——快照由迁移文件合并而来。
    if struct_attrs.strict {
        let expectations: Vec<crate::schema::ColumnExpectation> = fields
            .iter()
            .map(|f| crate::schema::ColumnExpectation {
                column: f.column.clone(),
                rust_family: crate::schema::rust_type_family(&f.ty, f.attrs.is_json),
            })
            .collect();
        if let Some(schema_path) = &struct_attrs.schema {
            crate::schema::validate_columns(
                &ast.ident,
                &table_name,
                &expectations,
                pk_name,
                schema_path,
            )?;
        } else {
            let snap_path = std::env::var("ALUN_DB_SCHEMA_SNAPSHOT").map_err(|_| {
                syn::Error::new(
                    ast.ident.span(),
                    "`#[model(strict)]` requires `schema = \"...\"` or a build.rs snapshot \
                     (set env `ALUN_DB_SCHEMA_SNAPSHOT`, e.g. via `alun_db_build::build_snapshot`)",
                )
            })?;
            let path = std::path::PathBuf::from(snap_path);
            crate::schema::validate_columns_at(
                &ast.ident,
                &table_name,
                &expectations,
                pk_name,
                &path,
            )?;
        }
    }

    // 字段元数据
    let field_meta = generate_field_meta(&fields)?;

    // Hook 列表。
    //
    // 含 `created_at` / `updated_at` 字段时自动挂载内置 [`TimestampHook`]
    // （排在最前，`before_*` 正序执行，用户 Hook 可覆盖其写入的时间戳）；
    // 显式 `#[hook(TimestampHook)]` 时不重复挂载，由声明位置决定顺序；
    // `#[model(no_timestamps)]` 关闭自动挂载（时间戳由调用方自行维护，
    // 适用于 `created_at` 承载业务时间而非审计时间的模型）。
    //
    // [`TimestampHook`]: alun_db_core::hook::TimestampHook
    let has_timestamp_field = fields
        .iter()
        .any(|f| f.name == "created_at" || f.name == "updated_at");
    let explicit_timestamp_hook = struct_attrs.hooks.iter().any(|p| {
        p.segments
            .last()
            .is_some_and(|s| s.ident == "TimestampHook")
    });
    let auto_timestamp_hook =
        if has_timestamp_field && !explicit_timestamp_hook && !struct_attrs.no_timestamps {
            quote! {
                &::alun_db_core::hook::TimestampHook
                    as &'static dyn ::alun_db_core::traits::Hook,
            }
        } else {
            quote! {}
        };

    let hook_refs: Vec<TokenStream> = struct_attrs
        .hooks
        .iter()
        .map(|path| quote! { & #path as &'static dyn ::alun_db_core::traits::Hook })
        .collect();

    // 异步 Hook（`#[hook_async(Type)]`）：提交后触发，见 `AsyncHook`。
    let async_hook_refs: Vec<TokenStream> = struct_attrs
        .async_hooks
        .iter()
        .map(|path| quote! { & #path as &'static dyn ::alun_db_core::traits::AsyncHook })
        .collect();

    // ==================== 生成 from_row ====================
    // 索引快路径：当 Row 列序与模型字段序一致时，直接按索引取值（O(1)，免字符串查找）。
    let index_from_row_fields: Vec<TokenStream> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let ident = &f.ident;
            let name = &f.name;
            if f.attrs.is_json {
                // #[json] 字段：从原始 Value 取出 JSON 文本（Json 或 Text），再反序列化。
                quote! {
                    #ident: {
                        let raw = ::alun_db_core::row::Row::get_at(&row, #i);
                        let json_str: ::core::option::Option<&str> = match raw {
                            Some(::alun_db_core::stmt::Value::Json(s)) |
                            Some(::alun_db_core::stmt::Value::Text(s)) => Some(s.as_str()),
                            Some(::alun_db_core::stmt::Value::Null) | None => None,
                            other => return Err(::alun_db_core::error::DbError::TypeMismatch {
                                expected: "Json",
                                actual: other.map(|v| v.ty().name()).unwrap_or("Null"),
                            }),
                        };
                        json_str.map(|s| ::serde_json::from_str(s))
                            .transpose()
                            .map_err(|e| ::alun_db_core::error::DbError::InvalidValue(
                                format!("json decode {}: {e}", #name)
                            ))?
                    }
                }
            } else {
                quote! {
                    #ident: row.get_typed_at::<_>(#name, #i)?
                }
            }
        })
        .collect();

    // 兜底路径：列序不一致时，按列名匹配（保留原逻辑）。
    let name_from_row_fields: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let ident = &f.ident;
            let name = &f.name;
            let column = &f.column;
            if f.attrs.is_json {
                // #[json] 字段：从原始 Value 取出 JSON 文本（Json 或 Text），再反序列化。
                quote! {
                    #ident: {
                        let raw = ::alun_db_core::row::Row::get(&row, #column)
                            .or_else(|| ::alun_db_core::row::Row::get(&row, #name));
                        let json_str: ::core::option::Option<&str> = match raw {
                            Some(::alun_db_core::stmt::Value::Json(s)) |
                            Some(::alun_db_core::stmt::Value::Text(s)) => Some(s.as_str()),
                            Some(::alun_db_core::stmt::Value::Null) | None => None,
                            other => return Err(::alun_db_core::error::DbError::TypeMismatch {
                                expected: "Json",
                                actual: other.map(|v| v.ty().name()).unwrap_or("Null"),
                            }),
                        };
                        json_str.map(|s| ::serde_json::from_str(s))
                            .transpose()
                            .map_err(|e| ::alun_db_core::error::DbError::InvalidValue(
                                format!("json decode {}: {e}", #name)
                            ))?
                    }
                }
            } else {
                quote! {
                    #ident: row.get_typed::<_>(#column)
                        .or_else(|_| row.get_typed::<_>(#name))?
                }
            }
        })
        .collect();

    // 期望的列序，用于 `columns_match` 判定是否可走索引快路径。
    // 使用列名：重命名列（#[column(...)]）时数据库行键为列名。
    let expected_names: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let column = &f.column;
            quote! { #column }
        })
        .collect();

    // ==================== 生成 as_row ====================
    let as_row_inserts: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let name = &f.name;
            let ident = &f.ident;
            if f.attrs.is_json {
                // #[json] 字段：序列化为 JSON 字符串写入 Value::Json，失败显式传播
                quote! {
                    if let ::core::option::Option::Some(v) = &self.#ident {
                        r.insert(
                            #name,
                            ::alun_db_core::stmt::Value::Json(
                                ::serde_json::to_string(v).map_err(|e| {
                                    ::alun_db_core::error::DbError::InvalidValue(
                                        format!("json encode {}: {e}", #name)
                                    )
                                })?
                            ),
                        );
                    } else {
                        r.insert(#name, ::alun_db_core::stmt::Value::Null);
                    }
                }
            } else {
                quote! {
                    r.insert(
                        #name,
                        ::alun_db_core::traits::TypeMapper::to_value(&self.#ident),
                    );
                }
            }
        })
        .collect();

    // ==================== 生成 field_values_into ====================
    // 与 `as_row` 同序、同值，但**只写值、不写字段名**：批量写入只关心列顺序，
    // 逐字段的字段名字符串分配纯属浪费。
    let field_values_pushes: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let name = &f.name;
            let ident = &f.ident;
            if f.attrs.is_json {
                quote! {
                    match &self.#ident {
                        ::core::option::Option::Some(v) => out.push(
                            ::alun_db_core::stmt::Value::Json(
                                ::serde_json::to_string(v).map_err(|e| {
                                    ::alun_db_core::error::DbError::InvalidValue(
                                        format!("json encode {}: {e}", #name)
                                    )
                                })?
                            ),
                        ),
                        ::core::option::Option::None => {
                            out.push(::alun_db_core::stmt::Value::Null);
                        }
                    }
                }
            } else {
                quote! {
                    out.push(::alun_db_core::traits::TypeMapper::to_value(&self.#ident));
                }
            }
        })
        .collect();
    let field_count = fields.len();

    // ==================== 生成 pk_value ====================
    let pk_value = quote! {
        ::alun_db_core::traits::TypeMapper::to_value(&self.#pk_ident)
    };

    // ==================== 生成字段表达式访问器 ====================
    let field_accessors: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let ident = &f.ident;
            let column = &f.column;
            let ty = &f.ty;
            quote! {
                /// 返回字段表达式访问器。
                pub fn #ident() -> ::alun_db_core::field::FieldExpr<#ty> {
                    ::alun_db_core::field::FieldExpr::new(#table_name, #column)
                }
            }
        })
        .collect();

    // ==================== 组装输出 ====================
    let expanded = quote! {
        // ---- Model 实现 ----
        impl #impl_generics ::alun_db_core::traits::Model for #name #ty_generics #where_clause {
            const TABLE: &'static str = #table_name;
            const READONLY: bool = #is_readonly;
            const PRIMARY_KEY: &'static str = #pk_name;
            #soft_delete_const
            #tenant_const
            #cross_tenant_const

            fn fields() -> &'static [::alun_db_core::field::FieldMeta] {
                static FIELDS: ::std::sync::OnceLock<Vec<::alun_db_core::field::FieldMeta>> =
                    ::std::sync::OnceLock::new();
                FIELDS.get_or_init(|| vec![ #( #field_meta ),* ])
            }

            fn from_row(row: ::alun_db_core::row::Row) -> ::alun_db_core::error::Result<Self> {
                // 列序与模型字段序一致时走索引快路径，否则回退按名匹配。
                if row.columns_match(&[ #( #expected_names ),* ]) {
                    Ok(Self {
                        #( #index_from_row_fields ),*
                    })
                } else {
                    Ok(Self {
                        #( #name_from_row_fields ),*
                    })
                }
            }

            fn as_row(&self) -> ::alun_db_core::error::Result<::alun_db_core::row::Row> {
                let mut r = ::alun_db_core::row::Row::with_capacity(0);
                #( #as_row_inserts )*
                Ok(r)
            }

            fn field_values_into(
                &self,
                out: &mut ::std::vec::Vec<::alun_db_core::stmt::Value>,
            ) -> ::alun_db_core::error::Result<()> {
                out.clear();
                out.reserve(#field_count);
                #( #field_values_pushes )*
                Ok(())
            }

            fn pk_value(&self) -> ::alun_db_core::stmt::Value {
                #pk_value
            }

            fn hooks() -> &'static [&'static dyn ::alun_db_core::traits::Hook] {
                static HOOKS: &[&dyn ::alun_db_core::traits::Hook] = &[
                    #auto_timestamp_hook
                    #( #hook_refs ),*
                ];
                HOOKS
            }

            /// 异步 Hook：CRUD 提交后触发（事务内排队至 commit 成功后）。
            fn async_hooks() -> &'static [&'static dyn ::alun_db_core::traits::AsyncHook] {
                static ASYNC_HOOKS: &[&dyn ::alun_db_core::traits::AsyncHook] = &[
                    #( #async_hook_refs ),*
                ];
                ASYNC_HOOKS
            }
        }

        // ---- 字段表达式访问器 ----
        impl #impl_generics #name #ty_generics #where_clause {
            #( #field_accessors )*
        }
    };

    Ok(expanded)
}

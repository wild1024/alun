//! `#[derive(Row)]` 代码生成。

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

/// 生成 `Row` 双向转换。
///
/// 与 `#[derive(Model)]` 不同，`Row` 派生不要求主键，也不生成字段表达式。
/// 它只生成从 `Row` 构造和序列化为 `Row` 的能力，用于 JSON 嵌套结构。
pub fn derive(ast: DeriveInput) -> syn::Result<TokenStream> {
    let fields = match &ast.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    &ast,
                    "`#[derive(Row)]` only supports structs with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &ast,
                "`#[derive(Row)]` only supports structs",
            ));
        }
    };

    let name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    let field_count = fields.len();

    // 从 Row 构造
    let from_row_fields: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let ident = f.ident.as_ref().unwrap();
            let name_str = ident.to_string();
            quote! {
                #ident: row.get_typed::<_>(#name_str)?
            }
        })
        .collect();

    // 转为 Row
    let as_row_inserts: Vec<TokenStream> = fields
        .iter()
        .map(|f| {
            let ident = f.ident.as_ref().unwrap();
            let name_str = ident.to_string();
            quote! {
                r.insert(
                    #name_str,
                    ::alun_db_core::traits::TypeMapper::to_value(&self.#ident),
                );
            }
        })
        .collect();

    Ok(quote! {
        impl #impl_generics #name #ty_generics #where_clause {
            /// 从行数据构造。
            pub fn from_row(row: ::alun_db_core::row::Row) -> ::alun_db_core::error::Result<Self> {
                Ok(Self {
                    #( #from_row_fields ),*
                })
            }

            /// 转为行数据。
            pub fn as_row(&self) -> ::alun_db_core::row::Row {
                let mut r = ::alun_db_core::row::Row::with_capacity(#field_count);
                #( #as_row_inserts )*
                r
            }
        }
    })
}

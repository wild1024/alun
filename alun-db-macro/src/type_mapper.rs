//! `#[derive(TypeMapper)]` 代码生成（仅枚举）。

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput};

/// 为枚举生成 `TypeMapper` 实现。
pub fn derive(ast: DeriveInput) -> syn::Result<TokenStream> {
    let variants = match &ast.data {
        Data::Enum(data) => &data.variants,
        _ => {
            return Err(syn::Error::new_spanned(
                &ast,
                "`#[derive(TypeMapper)]` only supports enums",
            ));
        }
    };

    let name = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    // 派生枚举的 `Value` 双向转换：
    // - `From<Enum> for Value`：使派生枚举可直接作为 `Into<Value>` 参数
    //   （find / find_by 等的绑定），免去手写 `to_value()`
    // - `TryFrom<&Value>`：从行值恢复为枚举，兜底 `TypeMapper::from_value`
    // 二者均委托 `TypeMapper`，与具体编码分支（原生枚举 / value_map /
    // 整数编码 / 默认文本）保持一致。
    let conv_tokens = quote! {
        impl #impl_generics From<#name #ty_generics> for ::alun_db_core::stmt::Value #where_clause {
            fn from(value: #name #ty_generics) -> Self {
                ::alun_db_core::traits::TypeMapper::to_value(&value)
            }
        }
        impl #impl_generics ::core::convert::TryFrom<&::alun_db_core::stmt::Value> for #name #ty_generics #where_clause {
            type Error = ::alun_db_core::error::DbError;
            fn try_from(value: &::alun_db_core::stmt::Value) -> ::alun_db_core::error::Result<Self> {
                ::alun_db_core::traits::TypeMapper::from_value(value)
            }
        }
    };

    // `#[int]` derive helper：整数编码模式（不占用语言内建的 `repr` 属性名）
    let is_int_encoded = ast.attrs.iter().any(|a| a.path().is_ident("int"));
    let native_enum: Option<String> = ast
        .attrs
        .iter()
        .find(|a| a.path().is_ident("native_enum"))
        .and_then(|a| a.parse_args::<syn::LitStr>().ok())
        .map(|l| l.value());

    let value_map: Option<(bool, Vec<(String, syn::Ident)>)> = ast
        .attrs
        .iter()
        .find(|a| a.path().is_ident("value_map"))
        .map(parse_value_map)
        .transpose()?;

    if let Some(type_name) = &native_enum {
        let type_name_lit = syn::LitStr::new(type_name, proc_macro2::Span::call_site());
        let to_value_arms: Vec<TokenStream> = variants
            .iter()
            .map(|v| {
                let ident = &v.ident;
                let name_str = ident.to_string();
                quote! { Self::#ident => ::alun_db_core::stmt::Value::Enum(#name_str.to_string()) }
            })
            .collect();
        let from_value_arms: Vec<TokenStream> = variants
            .iter()
            .map(|v| {
                let ident = &v.ident;
                let name_str = ident.to_string();
                quote! { #name_str => Ok(Self::#ident) }
            })
            .collect();

        return Ok(quote! {
            impl #impl_generics ::alun_db_core::traits::TypeMapper for #name #ty_generics #where_clause {
                fn stmt_type() -> ::alun_db_core::stmt::Type {
                    ::alun_db_core::stmt::Type::Enum(#type_name_lit)
                }
                fn to_value(&self) -> ::alun_db_core::stmt::Value {
                    match self { #( #to_value_arms ),* }
                }
                fn from_value(v: &::alun_db_core::stmt::Value) -> ::alun_db_core::error::Result<Self> {
                    // 原生枚举列读回 `Value::Enum`；无原生枚举的方言（SQLite 等
                    // 退化为文本承载）由驱动读回 `Value::Text`，两者按同一组标签解码。
                    match v {
                        ::alun_db_core::stmt::Value::Enum(s)
                        | ::alun_db_core::stmt::Value::Text(s) => match s.as_str() {
                            #( #from_value_arms ,)*
                            _ => Err(::alun_db_core::error::DbError::InvalidValue(
                                format!("unknown enum variant: {s}")
                            )),
                        },
                        _ => Err(::alun_db_core::error::DbError::TypeMismatch {
                            expected: "Enum",
                            actual: v.ty().name(),
                        }),
                    }
                }
            }
            #conv_tokens
        });
    }

    if let Some((with_serde, map)) = &value_map {
        let with_serde = *with_serde;
        let to_value_arms: Vec<TokenStream> = map.iter()
            .map(|(encoded, variant)| {
                let encoded_lit = syn::LitStr::new(encoded, proc_macro2::Span::call_site());
                quote! { Self::#variant => ::alun_db_core::stmt::Value::Text(#encoded_lit.to_string()) }
            })
            .collect();
        let from_value_checks: Vec<TokenStream> = map
            .iter()
            .map(|(encoded, variant)| {
                let encoded_lit = syn::LitStr::new(encoded, proc_macro2::Span::call_site());
                quote! {
                    if s == #encoded_lit { return Ok(Self::#variant); }
                }
            })
            .collect();

        // `serde` 标记：JSON 编码与落库编码共用同一张映射表（D008）
        let serde_tokens = if with_serde {
            let ser_arms: Vec<TokenStream> = map
                .iter()
                .map(|(encoded, variant)| {
                    let lit = syn::LitStr::new(encoded, proc_macro2::Span::call_site());
                    quote! { Self::#variant => #lit }
                })
                .collect();
            let de_arms: Vec<TokenStream> = map
                .iter()
                .map(|(encoded, variant)| {
                    let lit = syn::LitStr::new(encoded, proc_macro2::Span::call_site());
                    // 注意：此处位于 Visitor 的 impl 内，`Self` 是 visitor 而非枚举
                    quote! { #lit => Ok(#name::#variant), }
                })
                .collect();
            let keys: Vec<TokenStream> = map
                .iter()
                .map(|(encoded, _)| {
                    let lit = syn::LitStr::new(encoded, proc_macro2::Span::call_site());
                    quote! { #lit }
                })
                .collect();
            quote! {
                impl #impl_generics ::serde::Serialize for #name #ty_generics #where_clause {
                    fn serialize<S>(&self, serializer: S) -> ::core::result::Result<S::Ok, S::Error>
                    where
                        S: ::serde::Serializer,
                    {
                        serializer.serialize_str(match self { #( #ser_arms ),* })
                    }
                }

                impl<'de> ::serde::Deserialize<'de> for #name {
                    fn deserialize<D>(deserializer: D) -> ::core::result::Result<Self, D::Error>
                    where
                        D: ::serde::Deserializer<'de>,
                    {
                        struct ValueMapVisitor;

                        impl<'de> ::serde::de::Visitor<'de> for ValueMapVisitor {
                            type Value = #name;

                            fn expecting(
                                &self,
                                f: &mut ::core::fmt::Formatter<'_>,
                            ) -> ::core::fmt::Result {
                                f.write_str(concat!("`", stringify!(#name), "` 的 value_map 编码"))
                            }

                            fn visit_str<E>(self, s: &str) -> ::core::result::Result<Self::Value, E>
                            where
                                E: ::serde::de::Error,
                            {
                                match s {
                                    #( #de_arms )*
                                    other => Err(E::unknown_variant(other, &[#( #keys ),*])),
                                }
                            }
                        }

                        deserializer.deserialize_str(ValueMapVisitor)
                    }
                }
            }
        } else {
            quote! {}
        };

        return Ok(quote! {
            impl #impl_generics ::alun_db_core::traits::TypeMapper for #name #ty_generics #where_clause {
                fn stmt_type() -> ::alun_db_core::stmt::Type {
                    ::alun_db_core::stmt::Type::Text
                }
                fn to_value(&self) -> ::alun_db_core::stmt::Value {
                    match self { #( #to_value_arms ),* }
                }
                fn from_value(v: &::alun_db_core::stmt::Value) -> ::alun_db_core::error::Result<Self> {
                    match v {
                        ::alun_db_core::stmt::Value::Text(s) => {
                            #( #from_value_checks )*
                            Err(::alun_db_core::error::DbError::InvalidValue(
                                format!("unknown enum value: {s}")
                            ))
                        }
                        _ => Err(::alun_db_core::error::DbError::TypeMismatch {
                            expected: "Text",
                            actual: v.ty().name(),
                        }),
                    }
                }
            }
            #serde_tokens
            #conv_tokens
        });
    }

    if is_int_encoded {
        let to_value_arms: Vec<TokenStream> = variants
            .iter()
            .map(|v| {
                let ident = &v.ident;
                // 用变体常量而非 `*self`：`to_value(&self)` 下 `*self as i32` 会
                // 对非 `Copy` 枚举报 E0507，而判别值本身是常量。
                quote! { Self::#ident => ::alun_db_core::stmt::Value::I32(Self::#ident as i32) }
            })
            .collect();
        let int_match_arms: Vec<TokenStream> = variants
            .iter()
            .map(|v| {
                let ident = &v.ident;
                quote! {
                    if n == Self::#ident as i32 {
                        return Ok(Self::#ident);
                    }
                }
            })
            .collect();

        return Ok(quote! {
            impl #impl_generics ::alun_db_core::traits::TypeMapper for #name #ty_generics #where_clause {
                fn stmt_type() -> ::alun_db_core::stmt::Type {
                    ::alun_db_core::stmt::Type::I32
                }
                fn to_value(&self) -> ::alun_db_core::stmt::Value {
                    match self { #( #to_value_arms ),* }
                }
                fn from_value(v: &::alun_db_core::stmt::Value) -> ::alun_db_core::error::Result<Self> {
                    // 判别值不绑定列宽：驱动读回整数列时统一归一宽度（如 `I64`），
                    // 由 `int_discriminant` 归一为 `i32` 后再比对判别值。
                    let n = ::alun_db_core::traits::int_discriminant(v)?;
                    #( #int_match_arms )*
                    Err(::alun_db_core::error::DbError::InvalidValue(
                        format!("unknown enum discriminant: {n}")
                    ))
                }
            }
            #conv_tokens
        });
    }

    let to_value_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| {
            let ident = &v.ident;
            let name_str = ident.to_string();
            quote! { Self::#ident => ::alun_db_core::stmt::Value::Text(#name_str.to_string()) }
        })
        .collect();
    let from_value_arms: Vec<TokenStream> = variants
        .iter()
        .map(|v| {
            let ident = &v.ident;
            let name_str = ident.to_string();
            quote! { #name_str => Ok(Self::#ident) }
        })
        .collect();

    Ok(quote! {
        impl #impl_generics ::alun_db_core::traits::TypeMapper for #name #ty_generics #where_clause {
            fn stmt_type() -> ::alun_db_core::stmt::Type {
                ::alun_db_core::stmt::Type::Text
            }
            fn to_value(&self) -> ::alun_db_core::stmt::Value {
                match self { #( #to_value_arms ),* }
            }
            fn from_value(v: &::alun_db_core::stmt::Value) -> ::alun_db_core::error::Result<Self> {
                match v {
                    ::alun_db_core::stmt::Value::Text(s) => match s.as_str() {
                        #( #from_value_arms ,)*
                        _ => Err(::alun_db_core::error::DbError::InvalidValue(
                            format!("unknown enum variant: {s}")
                        )),
                    },
                    _ => Err(::alun_db_core::error::DbError::TypeMismatch {
                        expected: "Text",
                        actual: v.ty().name(),
                    }),
                }
            }
        }
        #conv_tokens
    })
}

fn parse_value_map(attr: &syn::Attribute) -> syn::Result<(bool, Vec<(String, syn::Ident)>)> {
    use syn::parse::ParseStream;
    let mut result = Vec::new();
    // 可选前导标记 `serde`：同时生成 `Serialize`/`Deserialize`，让 JSON 编码
    // 与落库编码一致（缺陷 D008）。未写该标记时行为与旧版完全相同。
    let mut with_serde = false;
    attr.parse_args_with(|input: ParseStream| {
        if input.peek(syn::Ident) {
            let flag: syn::Ident = input.parse()?;
            if flag == "serde" {
                with_serde = true;
                let _ = input.parse::<syn::Token![,]>();
            } else {
                return Err(syn::Error::new(
                    flag.span(),
                    "`#[value_map]` 的首个标识符只支持 `serde`（生成 serde 实现）",
                ));
            }
        }
        while !input.is_empty() {
            let key: syn::LitStr = input.parse()?;
            input.parse::<syn::Token![=>]>()?;
            let value: syn::Ident = input.parse()?;
            result.push((key.value(), value));
            let _ = input.parse::<syn::Token![,]>();
        }
        Ok(())
    })?;
    Ok((with_serde, result))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 展开枚举定义并断言生成 token **可解析**。
    ///
    /// 生成结果不可解析时，下游 crate 只会看到
    /// `proc-macro derive produced unparsable tokens`（D002）。
    fn expand(def: TokenStream) -> String {
        let ast: DeriveInput = syn::parse2(def).expect("测试枚举定义应可解析");
        let generated = derive(ast).expect("派生应成功");
        let text = generated.to_string();
        syn::parse2::<syn::File>(generated)
            .unwrap_or_else(|e| panic!("生成 token 不可解析：{e}\n{text}"));
        text
    }

    /// 四种编码模式的展开结果都必须可解析。
    ///
    /// 文本模式与 `#[native_enum]` 曾把 `from_value` 的匹配臂拼成
    /// `match s.as_str() { "Draft" => … "Issued" => … _ => … }`（缺逗号）→
    /// `error: expected ',' following match arm`。
    #[test]
    fn every_mode_generates_parsable_tokens() {
        expand(quote! { enum E { Draft, Issued } });
        expand(quote! { #[int] enum E { Draft = 1, Issued = 2 } });
        expand(quote! {
            #[value_map("draft" => Draft, "issued" => Issued)] enum E { Draft, Issued }
        });
        expand(quote! { #[native_enum("state")] enum E { Draft, Issued } });
    }

    /// 单变体枚举同样可解析（臂数为 1，末尾仍须接 `_` 兜底臂）。
    #[test]
    fn single_variant_generates_parsable_tokens() {
        expand(quote! { enum E { Only } });
        expand(quote! { #[native_enum("state")] enum E { Only } });
    }

    /// `#[value_map(serde, …)]`：额外生成 serde 实现，使 JSON 与落库编码一致（D008）。
    #[test]
    fn value_map_serde_flag_emits_serde_impls() {
        let without = expand(quote! {
            #[value_map("draft" => Draft)] enum E { Draft }
        });
        assert!(
            !without.contains("serde :: Serialize"),
            "未写 serde 标记时不得生成 serde 实现：{without}"
        );

        let with = expand(quote! {
            #[value_map(serde, "draft" => Draft)] enum E { Draft }
        });
        for needle in ["serde :: Serialize", "serde :: Deserialize", "\"draft\""] {
            assert!(with.contains(needle), "缺少 {needle}：{with}");
        }
    }

    /// `#[int]` 模式的判别值读取经 `int_discriminant` 归一宽度（驱动读回 `I64`）。
    #[test]
    fn int_mode_reads_discriminant_through_helper() {
        let text = expand(quote! { #[int] enum E { Draft = 1, Issued = 2 } });
        assert!(
            text.contains("int_discriminant"),
            "整数模式应经 int_discriminant 归一宽度：{text}"
        );
        assert!(
            !text.contains("* self as i32"),
            "整数模式写侧不应解引用 `self`（要求枚举 `Copy`）：{text}"
        );
    }
}

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemStruct};

/// `#[plugin]`：为结构体自动 impl PluginRegistration 标记 trait
pub fn plugin_impl(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let name = &input.ident;
    let name_str = name.to_string();

    let expanded = quote! {
        #input

        impl #name {
            /// 获取插件名称
            pub fn alun_plugin_name() -> &'static str {
                #name_str
            }
        }
    };

    expanded.into()
}

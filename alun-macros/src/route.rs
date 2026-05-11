use proc_macro::TokenStream;
use quote::{quote, format_ident};
use syn::{parse_macro_input, ItemFn};

/// 展开 #[controller("/path")]：自动为 impl 块中标注了 `#[get]`/`#[post]` 的方法生成路由注册
pub fn controller_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let base_path = if attr.is_empty() {
        "/".to_string()
    } else {
        let s = attr.to_string();
        s.trim_matches('"').to_string()
    };

    let input = parse_macro_input!(item as syn::ItemImpl);
    let _struct_ty = &input.self_ty;

    let mut route_registrations = Vec::new();

    for ii in &input.items {
        if let syn::ImplItem::Fn(method) = ii {
            let fn_name = &method.sig.ident;
            let mut method_path = String::new();
            let mut http_method = String::new();
            let is_async = method.sig.asyncness.is_some();

            for attr in &method.attrs {
                let attr_path = attr.path();
                let attr_name = attr_path.get_ident().map(|i| i.to_string());

                match attr_name.as_deref() {
                    Some("get") | Some("post") | Some("put") | Some("delete") => {
                        http_method = attr_name.unwrap();
                        if let syn::Meta::List(ml) = &attr.meta {
                            let content = ml.tokens.to_string();
                            method_path = content.trim().trim_matches('"').to_string();
                        }
                    }
                    _ => {}
                }
            }

            if !http_method.is_empty() && is_async {
                let full_path = format!("{}{}", base_path.trim_end_matches('/'), method_path);
                let route_name = format_ident!("__ALUN_CTL_{}", fn_name.to_string().to_uppercase());
                let register = gen_route_register(&http_method, &full_path, fn_name, &route_name);
                route_registrations.push(register);
            }
        }
    }

    let expanded = quote! {
        #input
        #(#route_registrations)*
    };

    expanded.into()
}

/// 展开 #[get("/path")]、#[post("/path")] 等独立函数注解
pub fn method_route_impl(http_method: &str, attr: TokenStream, item: TokenStream) -> TokenStream {
    let path = if attr.is_empty() {
        "/".to_string()
    } else {
        let s = attr.to_string();
        s.trim_matches('"').to_string()
    };

    let input = parse_macro_input!(item as ItemFn);
    let fn_name = &input.sig.ident;

    let route_name = format_ident!("__ALUN_FN_{}", fn_name.to_string().to_uppercase());
    let register = gen_route_register(http_method, &path, fn_name, &route_name);

    let expanded = quote! {
        #input
        #register
    };

    expanded.into()
}

fn gen_route_register(
    method: &str,
    path: &str,
    fn_name: &syn::Ident,
    route_name: &syn::Ident,
) -> proc_macro2::TokenStream {
    let add_method = match method {
        "get"    => quote! { add_get },
        "post"   => quote! { add_post },
        "put"    => quote! { add_put },
        "delete" => quote! { add_delete },
        _        => quote! { add_route },
    };

    quote! {
        #[::alun::distributed_slice(::alun::ROUTES)]
        static #route_name: fn(&mut ::alun::AlunRouter) = |router: &mut ::alun::AlunRouter| {
            router.#add_method(#path, #fn_name);
        };
    }
}

/// 展开 `#[permission(path = "/api/admin", method = "GET", permission = "admin:access")]`
pub fn permission_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    let mut path_val = String::new();
    let mut method_val = String::new();
    let mut permission_val = String::new();

    // 解析形如 path = "/api/x", method = "GET", permission = "perm" 的参数
    let attr_str = attr.to_string();
    for part in attr_str.split(',') {
        let kv: Vec<&str> = part.splitn(2, '=').map(|s| s.trim()).collect();
        if kv.len() == 2 {
            let val = kv[1].trim().trim_matches('"');
            match kv[0] {
                "path" => path_val = val.to_string(),
                "method" => method_val = val.to_string(),
                "permission" => permission_val = val.to_string(),
                _ => {}
            }
        }
    }

    let fn_name = &input.sig.ident;
    let route_name = format_ident!("__ALUN_PERM_{}", fn_name.to_string().to_uppercase());

    let expanded = quote! {
        #input

        #[::alun::distributed_slice(::alun::PERMISSION_ROUTES)]
        static #route_name: ::alun::PermissionDef = ::alun::PermissionDef {
            path: #path_val,
            method: #method_val,
            permission: #permission_val,
        };
    };

    expanded.into()
}

/// 展开 `#[no_auth("/api/public")]` 标记无需认证的路径
pub fn no_auth_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    // 解析路径参数
    let path_val = if attr.is_empty() {
        String::new()
    } else {
        attr.to_string().trim_matches('"').to_string()
    };

    let fn_name = &input.sig.ident;
    let route_name = format_ident!("__ALUN_NO_AUTH_{}", fn_name.to_string().to_uppercase());

    let expanded = quote! {
        #input

        #[::alun::distributed_slice(::alun::NO_AUTH_ROUTES)]
        static #route_name: ::alun::NoAuthDef = ::alun::NoAuthDef {
            path: #path_val,
        };
    };

    expanded.into()
}

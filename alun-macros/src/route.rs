use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{ItemFn, parse_macro_input};

/// 展开 #[controller("/path")]：自动为 impl 块中标注了 `#[get]`/`#[post]`/`#[put]`/`#[patch]`/`#[delete]` 的方法生成路由注册
pub fn controller_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let base_path = if attr.is_empty() {
        "/".to_string()
    } else {
        let s = attr.to_string();
        s.trim_matches('"').to_string()
    };

    let mut input = parse_macro_input!(item as syn::ItemImpl);
    let struct_ty = input.self_ty.clone();

    let mut route_registrations = Vec::new();

    for ii in input.items.iter_mut() {
        if let syn::ImplItem::Fn(method) = ii {
            let fn_name = method.sig.ident.clone();
            let is_async = method.sig.asyncness.is_some();
            let mut method_path = String::new();
            let mut http_method = String::new();

            // 摘出 #[get]/#[post]/#[put]/#[patch]/#[delete]（含 #[alun::get] 形式），
            // 使其不再单独展开——注册统一由本宏在 impl 外部生成。
            method.attrs.retain(|attr| {
                let attr_name = attr.path().segments.last().map(|s| s.ident.to_string());
                match attr_name.as_deref() {
                    Some("get") | Some("post") | Some("put") | Some("patch") | Some("delete") => {
                        http_method = attr_name.unwrap();
                        if let syn::Meta::List(ml) = &attr.meta {
                            let content = ml.tokens.to_string();
                            method_path = content.trim().trim_matches('"').to_string();
                        }
                        false
                    }
                    _ => true,
                }
            });

            if !http_method.is_empty() && is_async {
                let full_path = format!("{}{}", base_path.trim_end_matches('/'), method_path);
                let route_name = format_ident!("__ALUN_CTL_{}", fn_name.to_string().to_uppercase());
                let handler = quote! { #struct_ty::#fn_name };
                let register = gen_route_register(&http_method, &full_path, &handler, &route_name);
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

/// 展开 #[get("/path")]、#[post("/path")]、#[put("/path")]、#[patch("/path")]、#[delete("/path")] 等独立函数注解
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
    let handler = quote! { #fn_name };
    let register = gen_route_register(http_method, &path, &handler, &route_name);

    let expanded = quote! {
        #input
        #register
    };

    expanded.into()
}

fn gen_route_register(
    method: &str,
    path: &str,
    handler: &proc_macro2::TokenStream,
    route_name: &syn::Ident,
) -> proc_macro2::TokenStream {
    // 已知动词走 `AlunRouter` 的类型化方法（方法过滤精确到该动词）；
    // 未知动词回退到按方法字符串注册。
    let register_call = match method {
        "get" => quote! { router.add_get(#path, #handler) },
        "post" => quote! { router.add_post(#path, #handler) },
        "put" => quote! { router.add_put(#path, #handler) },
        "patch" => quote! { router.add_patch(#path, #handler) },
        "delete" => quote! { router.add_delete(#path, #handler) },
        other => {
            let upper = other.to_uppercase();
            quote! { router.add_route(#upper, #path, #handler) }
        }
    };

    quote! {
        #[allow(non_snake_case)]
        fn #route_name(router: &mut ::alun::AlunRouter) {
            #register_call;
        }
        ::alun::inventory::submit! { ::alun::RouteEntry(#route_name) }
    }
}

/// 与 [`gen_route_register`] 相同，但把方法级中间件一并挂上（`#[alun::layer]` 用）。
///
/// 层经 `MethodRouter::route_layer` 挂载 ⇒ 只作用于该路由的方法处理器本身，
/// 不走 fallback，也不影响同路径上其它动词。
fn gen_route_register_with_layer(
    method: &str,
    path: &str,
    handler: &proc_macro2::TokenStream,
    route_name: &syn::Ident,
    layer_expr: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let register_call = match method {
        "get" => {
            quote! { router.add_get_with_layer(#path, #handler, |mr| mr.route_layer(#layer_expr)) }
        }
        "post" => {
            quote! { router.add_post_with_layer(#path, #handler, |mr| mr.route_layer(#layer_expr)) }
        }
        "put" => {
            quote! { router.add_put_with_layer(#path, #handler, |mr| mr.route_layer(#layer_expr)) }
        }
        "patch" => {
            quote! { router.add_patch_with_layer(#path, #handler, |mr| mr.route_layer(#layer_expr)) }
        }
        "delete" => {
            quote! { router.add_delete_with_layer(#path, #handler, |mr| mr.route_layer(#layer_expr)) }
        }
        other => {
            let upper = other.to_uppercase();
            quote! { router.add_route_with_layer(#upper, #path, #handler, |mr| mr.route_layer(#layer_expr)) }
        }
    };

    quote! {
        #[allow(non_snake_case)]
        fn #route_name(router: &mut ::alun::AlunRouter) {
            #register_call;
        }
        ::alun::inventory::submit! { ::alun::RouteEntry(#route_name) }
    }
}

/// 展开 `#[alun::layer(<表达式>)]` —— 给同一处理器挂**方法级**中间件（链内、可读身份）。
///
/// 必须与路由注解同用，且写在路由注解**上方**（属性宏自外向内展开，本宏需要看到
/// 尚未展开的 `#[alun::get("/path")]` 才能知道该注册到哪个方法与路径）。
pub fn layer_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemFn);
    let layer_expr: proc_macro2::TokenStream = attr.into();

    // 认领**第一个**路由注解并摘除它（其余路由注解保持原样，各自独立注册）
    let mut claimed = false;
    let mut http_method = String::new();
    let mut route_path = String::new();
    input.attrs.retain(|attr| {
        if claimed {
            return true;
        }
        let Some(name) = attr.path().segments.last().map(|s| s.ident.to_string()) else {
            return true;
        };
        let is_route_method = matches!(name.as_str(), "get" | "post" | "put" | "patch" | "delete");
        if !is_route_method {
            return true;
        }
        // 仅认领单段（`get`，配合 `use alun::get`）或含 `alun` 段（`alun::get`）的写法，
        // 避免误伤同名第三方属性
        let segs: Vec<String> = attr
            .path()
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        if segs.len() > 1 && !segs.iter().any(|s| s == "alun") {
            return true;
        }
        http_method = name;
        if let syn::Meta::List(ml) = &attr.meta {
            route_path = ml.tokens.to_string().trim().trim_matches('"').to_string();
        }
        claimed = true;
        false
    });

    if !claimed {
        return syn::Error::new_spanned(
            &input.sig.ident,
            "`#[alun::layer(...)]` 需要与路由注解同用，且要写在它**上方**：\n\n\
             #[alun::layer(MyLayer::new())]\n\
             #[alun::get(\"/path\")]\n\
             async fn handler() { /* ... */ }\n\n\
             （属性宏自外向内展开，写在下方时路由注解已先展开，本宏将看不到它）",
        )
        .to_compile_error()
        .into();
    }
    if route_path.is_empty() {
        route_path = "/".to_string();
    }

    let fn_name = &input.sig.ident;
    let route_name = format_ident!("__ALUN_FN_{}", fn_name.to_string().to_uppercase());
    let handler = quote! { #fn_name };
    let register = gen_route_register_with_layer(
        &http_method,
        &route_path,
        &handler,
        &route_name,
        &layer_expr,
    );

    let expanded = quote! {
        #input
        #register
    };

    expanded.into()
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
    let _ = fn_name; // 保留解析以复用错误定位；提交项本身不需要名字

    let expanded = quote! {
        #input

        ::alun::inventory::submit! {
            ::alun::PermissionDef {
                path: #path_val,
                method: #method_val,
                permission: #permission_val,
            }
        }
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

    let expanded = quote! {
        #input

        ::alun::inventory::submit! {
            ::alun::NoAuthDef {
                path: #path_val,
            }
        }
    };

    expanded.into()
}

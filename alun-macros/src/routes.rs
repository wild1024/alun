//! `#[alun::routes]` —— 模块级**显式**路由注册（不依赖链接器段收集）。
//!
//! # 为什么需要它
//!
//! 原有的 `#[alun::get]` + `App::scan()` 依赖 `inventory` 在**编译期**
//! 把注册项收集进全局切片。该机制要求链接器保留自定义段，在部分工具链
//! （实测：windows-gnu + rustc 1.101-nightly）上会**静默失效**——
//! 切片长度为 0，所有路由丢失（表现为全站 404），编译与链接都不报错。
//!
//! `#[alun::routes]` 走另一条路：它在**属性宏展开期**扫描模块内的路由注解，
//! 直接生成一个 `fn(App) -> App` 的注册函数。纯 Rust 调用，无任何链接器魔法，
//! 因此任何工具链上都成立。
//!
//! # 用法
//!
//! ```ignore
//! #[alun::routes]
//! mod api {
//!     use alun::prelude::*;
//!
//!     #[alun::get("/users")]
//!     pub async fn list_users() -> Res<String> { Res::ok("[]".into()) }
//!
//!     #[alun::permission(path = "/users", method = "POST", permission = "user:create")]
//!     #[alun::post("/users")]
//!     pub async fn create_user() -> Res<()> { Res::ok_empty() }
//!
//!     #[alun::no_auth("/api/health")]
//!     #[alun::get("/health")]
//!     pub async fn health() -> Res<String> { Res::ok("ok".into()) }
//! }
//!
//! // main.rs
//! App::new()?.module(api::register).start().await?;
//! ```
//!
//! 生成物：
//!
//! ```ignore
//! pub fn register(app: ::alun::App) -> ::alun::App { /* 注册全部路由 + 权限 + 免认证路径 */ }
//! ```
//!
//! 模块内的 `#[alun::get]` / `#[alun::permission]` / `#[alun::no_auth]` 会**被本宏剥离**
//! （不再展开成 distributed_slice），因此这些注解在此上下文中纯属声明性元数据。

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{Attribute, Item, ItemMod, LitStr, parse_macro_input};

/// 一条从模块中抽取出的路由
struct RouteSpec {
    /// HTTP 动词（小写：get / post / put / patch / delete / 自定义）
    method: String,
    /// 路径字面量
    path: String,
    /// 处理函数名
    ident: syn::Ident,
    /// 方法级中间件表达式（`#[alun::layer(<expr>)]`，可省略）
    layer: Option<TokenStream2>,
}

/// 一条权限规则
struct PermSpec {
    path: String,
    method: String,
    permission: String,
}

/// 一条免认证路径
struct NoAuthSpec {
    path: String,
}

/// 已知的 alun 路由类属性名
const ROUTE_METHODS: &[&str] = &["get", "post", "put", "patch", "delete"];

/// 判断属性是否为 alun 的路由 / 权限 / 免认证注解。
///
/// 只比较 `path` 的**末段**，因此 `alun::get`、`::alun::get`、`get`
/// （在 `use alun::get` 之后）三种写法都能识别。
fn alun_attr_kind(attr: &Attribute) -> Option<String> {
    let last = attr.path().segments.last()?.ident.to_string();
    if ROUTE_METHODS.contains(&last.as_str())
        || last == "permission"
        || last == "no_auth"
        || last == "layer"
    {
        // 仅当路径为单段（`get`）或多段且含 `alun` 时接受，避免误伤同名第三方属性
        let segs: Vec<String> = attr
            .path()
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        if segs.len() == 1 || segs.iter().any(|s| s == "alun") {
            return Some(last);
        }
    }
    None
}

/// 从一条路由属性中取路径字面量
fn path_lit(attr: &Attribute) -> syn::Result<String> {
    let lit: LitStr = attr.parse_args()?;
    Ok(lit.value())
}

/// 解析 `#[alun::permission(path = "..", method = "..", permission = "..")]`
fn parse_permission(attr: &Attribute) -> syn::Result<PermSpec> {
    let mut path = String::new();
    let mut method = String::new();
    let mut permission = String::new();

    attr.parse_nested_meta(|meta| {
        let val: LitStr = meta.value()?.parse()?;
        if meta.path.is_ident("path") {
            path = val.value();
        } else if meta.path.is_ident("method") {
            method = val.value();
        } else if meta.path.is_ident("permission") {
            permission = val.value();
        }
        Ok(())
    })?;

    Ok(PermSpec {
        path,
        method,
        permission,
    })
}

/// 解析 `#[alun::no_auth("/path")]`（参数可省略 → 空路径）
fn parse_no_auth(attr: &Attribute) -> syn::Result<NoAuthSpec> {
    let path = if attr.meta.require_list().is_ok() {
        attr.parse_args::<LitStr>()?.value()
    } else {
        String::new()
    };
    Ok(NoAuthSpec { path })
}

/// 取 `#[alun::layer(<expr>)]` 括号内的表达式
fn layer_expr_tokens(attr: &Attribute) -> syn::Result<TokenStream2> {
    Ok(attr.meta.require_list()?.tokens.clone())
}

/// 扫描一个函数，抽取并**移除** alun 注解
fn take_routes_from_fn(
    f: &mut syn::ItemFn,
    routes: &mut Vec<RouteSpec>,
    perms: &mut Vec<PermSpec>,
    no_auths: &mut Vec<NoAuthSpec>,
) -> syn::Result<()> {
    // 先按顺序解析（permission / no_auth / layer 可能出现在 route 之前或之后）
    let mut pending_perm: Vec<Attribute> = Vec::new();
    let mut pending_no_auth: Vec<Attribute> = Vec::new();
    let mut pending_layer: Vec<Attribute> = Vec::new();
    let mut pending_routes: Vec<(String, Attribute)> = Vec::new();

    for attr in f.attrs.iter() {
        if let Some(kind) = alun_attr_kind(attr) {
            if kind == "permission" {
                pending_perm.push(attr.clone());
            } else if kind == "no_auth" {
                pending_no_auth.push(attr.clone());
            } else if kind == "layer" {
                pending_layer.push(attr.clone());
            } else {
                pending_routes.push((kind, attr.clone()));
            }
        }
    }

    // 方法级中间件：同一处理器至多一个（多个会让执行顺序产生歧义）
    if pending_layer.len() > 1 {
        return Err(syn::Error::new_spanned(
            &pending_layer[1],
            "同一处理器上只允许一个 `#[alun::layer(...)]`；需要多层时请先组合成一个 \
             Layer（如 `LayerStack`）再传入",
        ));
    }
    let layer = pending_layer.first().map(layer_expr_tokens).transpose()?;

    if layer.is_some() && pending_routes.is_empty() {
        return Err(syn::Error::new_spanned(
            &f.sig.ident,
            "`#[alun::layer(...)]` 需要与路由注解同用，例如 \
             `#[alun::get(\"/path\")]`",
        ));
    }

    for (kind, attr) in pending_routes {
        routes.push(RouteSpec {
            method: kind,
            path: path_lit(&attr)?,
            ident: f.sig.ident.clone(),
            layer: layer.clone(),
        });
    }
    for attr in pending_perm {
        perms.push(parse_permission(&attr)?);
    }
    for attr in pending_no_auth {
        no_auths.push(parse_no_auth(&attr)?);
    }

    // 剥离注解：它们在本宏上下文中仅作声明，不应再展开成 distributed_slice
    f.attrs.retain(|a| alun_attr_kind(a).is_none());
    Ok(())
}

/// 展开 `#[alun::routes]`
pub fn routes_impl(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut module = parse_macro_input!(item as ItemMod);

    let Some((_brace, items)) = module.content.as_mut() else {
        return syn::Error::new_spanned(
            &module.ident,
            "`#[alun::routes]` 需要内联模块体（`mod x { ... }`）；\
             对 `mod x;` 形式请在 `x.rs` 内使用本宏",
        )
        .to_compile_error()
        .into();
    };

    let mut routes: Vec<RouteSpec> = Vec::new();
    let mut perms: Vec<PermSpec> = Vec::new();
    let mut no_auths: Vec<NoAuthSpec> = Vec::new();

    for it in items.iter_mut() {
        if let Item::Fn(f) = it {
            if let Err(e) = take_routes_from_fn(f, &mut routes, &mut perms, &mut no_auths) {
                return e.to_compile_error().into();
            }
        }
    }

    // 生成路由注册语句
    let mut route_calls: Vec<TokenStream2> = Vec::new();
    for r in &routes {
        let ident = &r.ident;
        let path = &r.path;
        let method = r.method.as_str();
        let call = match &r.layer {
            // 带方法级中间件：改走 `App::routes` + `add_*_with_layer`
            // （`MethodRouter::route_layer` ⇒ 只作用于该方法处理器，位于内置链之内）
            Some(layer) => {
                let add = match method {
                    "get" => quote! { add_get_with_layer },
                    "post" => quote! { add_post_with_layer },
                    "put" => quote! { add_put_with_layer },
                    "patch" => quote! { add_patch_with_layer },
                    "delete" => quote! { add_delete_with_layer },
                    _ => quote! { add_route_with_layer },
                };
                let method_arg = if matches!(method, "get" | "post" | "put" | "patch" | "delete") {
                    quote! {}
                } else {
                    let upper = method.to_uppercase();
                    quote! { #upper, }
                };
                quote! {
                    __alun_app = ::alun::App::routes(__alun_app, |__r: &mut ::alun::AlunRouter| {
                        __r.#add(#method_arg #path, #ident, |mr| mr.route_layer(#layer));
                    });
                }
            }
            None => match method {
                "get" => quote! { __alun_app = ::alun::App::get(__alun_app, #path, #ident); },
                "post" => quote! { __alun_app = ::alun::App::post(__alun_app, #path, #ident); },
                "put" => quote! { __alun_app = ::alun::App::put(__alun_app, #path, #ident); },
                "patch" => quote! { __alun_app = ::alun::App::patch(__alun_app, #path, #ident); },
                "delete" => quote! { __alun_app = ::alun::App::delete(__alun_app, #path, #ident); },
                other => {
                    let upper = other.to_uppercase();
                    quote! { __alun_app = ::alun::App::route(__alun_app, #upper, #path, #ident); }
                }
            },
        };
        route_calls.push(call);
    }

    let perm_items: Vec<TokenStream2> = perms
        .iter()
        .map(|p| {
            let (path, method, perm) = (&p.path, &p.method, &p.permission);
            // 注意：`PermissionDef` 的字段是 `&'static str`（链接期切片要求），
            // 因此这里直接内联字符串字面量，不能 `String::from(...)`。
            quote! {
                ::alun::PermissionDef {
                    path: #path,
                    method: #method,
                    permission: #perm,
                }
            }
        })
        .collect();

    let na_items: Vec<TokenStream2> = no_auths
        .iter()
        .map(|n| {
            let path = &n.path;
            quote! {
                ::alun::NoAuthDef { path: #path }
            }
        })
        .collect();

    let route_count = routes.len();
    let doc = format!(
        "由 `#[alun::routes]` 生成的**显式**注册入口（本模块共 {route_count} 条路由）。\n\n\
         不依赖任何编译期收集机制，因此在任何工具链上都有效。\n\n\
         用法：`App::new()?.module(<模块>::register).start().await?`"
    );

    // 注意：注册函数必须注入到**模块体内**，否则它既看不到模块里的处理函数，
    // 也无法以 `<模块>::register` 的形式被引用。
    let register_fn: syn::Item = match syn::parse2(quote! {
        #[doc = #doc]
        pub fn register(app: ::alun::App) -> ::alun::App {
            let mut __alun_app = app;
            #(#route_calls)*

            let __alun_perms: ::std::vec::Vec<::alun::PermissionDef> =
                ::std::vec![#(#perm_items),*];
            if !__alun_perms.is_empty() {
                __alun_app = ::alun::App::permission_defs(__alun_app, __alun_perms);
            }

            let __alun_na: ::std::vec::Vec<::alun::NoAuthDef> =
                ::std::vec![#(#na_items),*];
            if !__alun_na.is_empty() {
                __alun_app = ::alun::App::no_auth_defs(__alun_app, __alun_na);
            }

            __alun_app
        }
    }) {
        Ok(item) => item,
        Err(e) => return e.to_compile_error().into(),
    };

    if let Some((_, items)) = module.content.as_mut() {
        items.push(register_fn);
    }

    quote! { #module }.into()
}

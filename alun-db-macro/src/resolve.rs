//! 生成代码里的外部 crate 路径解析。
//!
//! 派生宏与 `sql!` 要在生成代码中引用 `alun-db-core`（以及 `#[json]` 字段用到的
//! `serde_json`），但调用方不一定直接声明它们——`alun`（门面）与 `alun-db` 都用
//! `#[doc(hidden)] pub use … as __xxx` 把内核转发出来。
//!
//! 生成代码统一**写死**这些路径，导出前按调用方 `Cargo.toml` 里**实际声明**的
//! crate 重写前缀：
//!
//! | 生成代码 | 调用方声明 `alun-db-core` | 声明 `alun-db` | 只声明 `alun` |
//! |---|---|---|---|
//! | `::alun_db_core::…` | `::alun_db_core` | `::alun_db::__db_core` | `::alun::__db_core` |
//! | `::serde_json::…` | `::serde_json` | `::alun_db::__serde_json` | `::alun::__serde_json` |
//!
//! 于是「只声明 `alun`」的项目也能直接 `#[derive(Model)]` / `sql!`，无需为了宏
//! 展开而在 `Cargo.toml` 里补一个自己并不使用的 crate。

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Group, Ident, Span, TokenStream, TokenTree};
use quote::quote;

/// 生成代码写死引用的外部 crate。
struct Need {
    /// 生成代码里的路径首段。
    placeholder: &'static str,
    /// 候选 `(包名, 包内转发路径)`，按优先级取第一个调用方声明过的。
    candidates: &'static [(&'static str, &'static str)],
}

const NEEDS: &[Need] = &[
    Need {
        placeholder: "alun_db_core",
        candidates: &[
            ("alun-db-core", ""),
            ("alun-db", "::__db_core"),
            ("alun", "::__db_core"),
        ],
    },
    Need {
        placeholder: "serde_json",
        candidates: &[
            ("serde_json", ""),
            ("alun", "::__serde_json"),
            ("alun-db", "::__serde_json"),
        ],
    },
    // `migrate!` 生成 `::alun_db::Migrator::…`：门面层要么直接用 `alun-db`，
    // 要么经 `alun::__db` 转发（`alun` 不一定在根上导出 `Migrator`）。
    Need {
        placeholder: "alun_db",
        candidates: &[("alun-db", ""), ("alun", "::__db")],
    },
];

/// 把生成代码里写死的外部 crate 路径重写为调用方可解析的前缀。
pub fn relocate(tokens: TokenStream) -> TokenStream {
    let resolved: Vec<(&'static str, TokenStream)> = NEEDS
        .iter()
        .map(|need| (need.placeholder, resolve(need)))
        .collect();
    rewrite(tokens, &resolved)
}

/// 按候选顺序解析出一个可解析前缀；都没声明则保留原路径。
fn resolve(need: &Need) -> TokenStream {
    for (package, suffix) in need.candidates {
        if let Some(prefix) = declared_prefix(package, suffix) {
            return prefix;
        }
    }
    // 一个都没声明：原样保留，编译器会直接报"找不到 crate"，线索最明确。
    let ident = Ident::new(need.placeholder, Span::call_site());
    TokenStream::from(TokenTree::Ident(ident))
}

/// 调用方声明了 `package` 时，返回指向其转发路径的前缀。
fn declared_prefix(package: &str, suffix: &str) -> Option<TokenStream> {
    let suffix: TokenStream = suffix.parse().expect("静态后缀总是合法 token");
    let lib_name = package.replace('-', "_");
    match crate_name(package) {
        Ok(FoundCrate::Name(name)) => {
            let ident = ident_of(&name);
            Some(quote!(#ident #suffix))
        }
        // `Itself` 也出现在本包的集成测试里：那里 `crate` 指向测试 crate 而非本包
        // lib，只能按名字引用（集成测试可见本包 lib）。
        Ok(FoundCrate::Itself) => Some(
            if std::env::var("CARGO_CRATE_NAME").is_ok_and(|n| n == lib_name) {
                quote!(crate #suffix)
            } else {
                let ident = Ident::new(&lib_name, Span::call_site());
                quote!(#ident #suffix)
            },
        ),
        Err(_) => None,
    }
}

/// 依 `proc-macro-crate` 给出的名字构造 `Ident`（兼容 `r#type` 形式的依赖重命名）。
fn ident_of(name: &str) -> Ident {
    match name.strip_prefix("r#") {
        Some(raw) => Ident::new_raw(raw, Span::call_site()),
        None => Ident::new(name, Span::call_site()),
    }
}

/// 递归重写 token 流中命中的首段标识符。
///
/// 这些标识符在生成代码里始终是路径首段（`::alun_db_core::…`）；没有前导 `::`
/// 时补上，保证重写结果仍是绝对路径。
fn rewrite(stream: TokenStream, resolved: &[(&'static str, TokenStream)]) -> TokenStream {
    let mut out = TokenStream::new();
    let mut colons = 0usize;
    for tt in stream {
        match tt {
            TokenTree::Ident(ident) => {
                match resolved
                    .iter()
                    .find(|(placeholder, _)| ident == *placeholder)
                {
                    Some((_, prefix)) => {
                        if colons < 2 {
                            out.extend(quote!(::));
                        }
                        out.extend(prefix.clone());
                    }
                    None => out.extend([TokenTree::Ident(ident)]),
                }
                colons = 0;
            }
            TokenTree::Group(group) => {
                let mut rewritten =
                    Group::new(group.delimiter(), rewrite(group.stream(), resolved));
                rewritten.set_span(group.span());
                out.extend([TokenTree::Group(rewritten)]);
                colons = 0;
            }
            leaf => {
                colons = match &leaf {
                    TokenTree::Punct(punct) if punct.as_char() == ':' => colons + 1,
                    _ => 0,
                };
                out.extend([leaf]);
            }
        }
    }
    out
}

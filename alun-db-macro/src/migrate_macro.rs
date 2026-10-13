//! `migrate!` 宏：把迁移目录在**编译期**编入二进制。
//!
//! 生成 `::alun_db::Migrator::from_embedded(&[(文件名, include_str!(…)), …])`，
//! 因此运行时不再依赖 `migrations/` 目录——单文件二进制部署不需要额外拷贝 SQL。
//!
//! ## 只做搬运，不做解析
//!
//! 宏只负责「列目录 + `include_str!`」，**不解析文件名**：分类（版本化 / 可重复 /
//! 种子 / 基线）、前缀与分隔符规则、冲突判定全部由 `alun-db` 的运行时加载器完成。
//! 这样磁盘扫描与编译期嵌入共用同一套语义，不存在「同一条迁移两种行为」的可能，
//! 也避免了在宏里复制一份解析器（那必然随版本漂移）。
//!
//! ## 已知限制
//!
//! - **新增文件不会自动触发重编译**：rustc 记录的依赖是 `include_str!` 到的各个
//!   文件，目录本身不在其中。新增迁移后需 `touch` 源文件或 `cargo clean -p <crate>`。
//!   （`sqlx::migrate!` 有同样的限制。）
//! - 目录为空或不存在时**编译失败**，而不是生成一个空执行器——路径写错必须在
//!   编译期暴露，否则运行时的 `run` 会静默变成 no-op。

use std::fs;
use std::path::{Path, PathBuf};

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::LitStr;

/// 展开 `migrate!("migrations")`。
///
/// # 参数
///
/// - `input`：单个字符串字面量，指向**相对 crate 根目录**的迁移目录
///   （绝对路径也接受）。
///
/// # 返回
///
/// 构造 `Migrator` 的表达式。
///
/// # 错误
///
/// 参数不是字符串字面量、目录不存在 / 不可读、或目录内没有任何 `.sql` 文件时，
/// 在字面量位置报编译错误。
pub fn expand(input: TokenStream) -> syn::Result<TokenStream2> {
    let dir_lit: LitStr = syn::parse(input)?;
    let span = dir_lit.span();
    let raw = dir_lit.value();

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| {
        syn::Error::new(
            span,
            "`migrate!` needs CARGO_MANIFEST_DIR; it can only be expanded by cargo",
        )
    })?;
    let root = PathBuf::from(&manifest_dir);
    let dir = if Path::new(&raw).is_absolute() {
        PathBuf::from(&raw)
    } else {
        root.join(&raw)
    };

    let mut files = Vec::new();
    collect_sql(&dir, &root, span, &mut files)?;
    if files.is_empty() {
        return Err(syn::Error::new(
            span,
            format!(
                "no `.sql` migration files found under '{}'; \
                 fix the path or remove the `migrate!` call — an empty migrator would \
                 silently turn `run()` into a no-op",
                dir.display()
            ),
        ));
    }
    // 排序保证生成代码稳定（同样的输入产出同样的 token 流，构建可复现）
    files.sort();

    let entries = files.iter().map(|file| {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let relative = file
            .strip_prefix(&root)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| file.to_string_lossy().replace('\\', "/"));
        quote! {
            (
                #name,
                ::std::include_str!(::std::concat!(
                    ::std::env!("CARGO_MANIFEST_DIR"),
                    "/",
                    #relative
                )),
            )
        }
    });

    Ok(quote! {
        ::alun_db::Migrator::from_embedded(&[#(#entries),*])
    })
}

/// 递归收集 `dir` 下的全部 `.sql` 文件（符号链接不跟随，避免成环）。
fn collect_sql(dir: &Path, root: &Path, span: Span, out: &mut Vec<PathBuf>) -> syn::Result<()> {
    let read_dir = fs::read_dir(dir).map_err(|e| {
        let hint = if root == dir {
            String::new()
        } else {
            format!(" (resolved to '{}')", dir.display())
        };
        syn::Error::new(span, format!("cannot read migrations dir{hint}: {e}"))
    })?;

    for entry in read_dir {
        let entry =
            entry.map_err(|e| syn::Error::new(span, format!("cannot list migrations dir: {e}")))?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|e| {
            syn::Error::new(span, format!("cannot inspect '{}': {e}", path.display()))
        })?;

        if file_type.is_dir() {
            collect_sql(&path, root, span, out)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let is_sql = path
            .file_name()
            .map(|name| {
                name.to_string_lossy()
                    .to_ascii_lowercase()
                    .ends_with(".sql")
            })
            .unwrap_or(false);
        if is_sql {
            out.push(path);
        }
    }
    Ok(())
}

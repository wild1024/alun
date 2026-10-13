//! 编译期 schema 校验。
//!
//! `#[model(strict)]` 让 derive 在编译期读取一份 DDL schema 文件（显式
//! `schema = "…"` 或 build.rs 快照 `ALUN_DB_SCHEMA_SNAPSHOT`），校验模型
//! 与数据库结构的一致性，包含两层：
//!
//! 1. **列名存在性**：模型字段列必须出现在对应 `CREATE TABLE` 中；
//! 2. **列类型兼容**：Rust 字段类型族与数据库列类型族需匹配（如 `i64`
//!    不应映射 `TEXT` 列）。
//!
//! 类型族是**方言无关的粗粒度归类**：SQLite 以 `INTEGER` 存布尔、MySQL 以
//! `TINYINT(1)` 存布尔、SQLite 以 `TEXT` 存 UUID/JSON 都是合法实践，故兼容
//! 矩阵采用「同族放宽」策略，避免方言差异造成误报。无法归类的类型（自定义
//! 枚举、`#[json]` 嵌套结构等）自动跳过类型比对，仅保留列名校验。

use std::collections::HashMap;
use std::path::PathBuf;

use syn::Type;

/// 单个字段的类型级校验期望。
pub struct ColumnExpectation {
    /// 数据库列名。
    pub column: String,
    /// 字段 Rust 类型映射出的类型族；`None` 表示无法归类（跳过类型校验）。
    pub rust_family: Option<&'static str>,
}

/// 从 schema 文件校验模型各列（列名存在性 + 列类型兼容）。
///
/// `schema_path` 相对 `CARGO_MANIFEST_DIR` 解析。
///
/// # 参数
///
/// - `model_ident`：模型名，用于生成指向性的编译错误。
/// - `table`：模型映射的表/视图名。
/// - `columns`：字段期望（列名 + Rust 类型族）。
/// - `pk`：主键列名。
/// - `schema_path`：DDL 文件路径，相对于 `CARGO_MANIFEST_DIR` 解析。
///
/// # 错误
///
/// schema 文件缺失/解析失败、表未声明、列缺失或列类型族不兼容时返回
/// [`syn::Error`]。
pub fn validate_columns(
    model_ident: &syn::Ident,
    table: &str,
    columns: &[ColumnExpectation],
    pk: &str,
    schema_path: &str,
) -> syn::Result<()> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| {
        syn::Error::new(
            model_ident.span(),
            "CARGO_MANIFEST_DIR is unset; cannot resolve strict schema path",
        )
    })?;
    let path = PathBuf::from(manifest).join(schema_path);
    validate_columns_at(model_ident, table, columns, pk, &path)
}

/// 从给定的 schema 文件路径校验模型各列（列名存在性 + 列类型兼容）。
///
/// `path` 为已解析好的绝对或相对路径（如 build.rs 快照的
/// `ALUN_DB_SCHEMA_SNAPSHOT` 路径）。
///
/// # 参数
///
/// - `model_ident`：模型名，用于生成指向性的编译错误。
/// - `table`：模型映射的表/视图名。
/// - `columns`：字段期望（列名 + Rust 类型族）。
/// - `pk`：主键列名。
/// - `path`：DDL/schema 快照文件路径。
///
/// # 错误
///
/// schema 文件缺失/解析失败、表未声明、列缺失或列类型族不兼容时返回
/// [`syn::Error`]。
pub fn validate_columns_at(
    model_ident: &syn::Ident,
    table: &str,
    columns: &[ColumnExpectation],
    pk: &str,
    path: &PathBuf,
) -> syn::Result<()> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        syn::Error::new(
            model_ident.span(),
            format!(
                "failed to read strict schema file `{}`: {e}",
                path.display()
            ),
        )
    })?;

    let table_cols = parse_create_table_columns(&content, table).ok_or_else(|| {
        syn::Error::new(
            model_ident.span(),
            format!(
                "`CREATE TABLE {table}` not found in schema `{}`",
                path.display()
            ),
        )
    })?;

    for col in columns {
        let col_lower = col.column.to_ascii_lowercase();
        match table_cols.get(&col_lower) {
            None => {
                return Err(syn::Error::new(
                    model_ident.span(),
                    format!(
                        "strict schema mismatch: column `{}` is not declared in `{table}` \
                         (verify `{table}`'s CREATE TABLE in `{}`)",
                        col.column,
                        path.display()
                    ),
                ));
            }
            Some(sql_family) => {
                // 列类型族比对：双方均可归类时才检查，避免未知类型误报
                if let (Some(rust_fam), Some(sql_fam)) = (col.rust_family, sql_family.as_deref())
                    && !is_compatible(rust_fam, sql_fam)
                {
                    return Err(syn::Error::new(
                        model_ident.span(),
                        format!(
                            "strict schema mismatch: column `{}` is `{rust_fam}` in Rust \
                             but `{sql_fam}` in table `{table}` (verify `{table}`'s CREATE TABLE \
                             in `{}`)",
                            col.column,
                            path.display()
                        ),
                    ));
                }
            }
        }
    }
    if !table_cols.contains_key(&pk.to_lowercase()) {
        return Err(syn::Error::new(
            model_ident.span(),
            format!("strict schema mismatch: primary key `{pk}` is not declared in `{table}`",),
        ));
    }
    Ok(())
}

/// 将 Rust 字段类型归类为类型族；无法归类时返回 `None`（跳过类型校验）。
///
/// `Option<T>` 自动解包内层；`#[json]` 字段固定归入 `json` 族。
/// 与 `alun_db_core::stmt::Type` 的映射保持一致（见
/// `alun-db-macro/src/field.rs` 的 `rust_type_to_stmt_type`）。
pub fn rust_type_family(ty: &Type, is_json: bool) -> Option<&'static str> {
    if is_json {
        return Some("json");
    }
    // 解包 Option<T>
    if let Some(inner) = option_inner(ty) {
        return rust_type_family(inner, is_json);
    }
    let name = last_segment(ty)?;
    Some(match name.as_str() {
        "bool" => "bool",
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "isize" | "usize" => "int",
        "f32" | "f64" => "float",
        "String" | "str" => "text",
        "Vec" => "bytes", // Vec<u8> 视作字节列
        "NaiveDate" | "Date" => "date",
        "NaiveTime" | "Time" => "time",
        "NaiveDateTime" | "PrimitiveDateTime" | "Timestamp" => "timestamp",
        "DateTime" | "OffsetDateTime" | "Zoned" => "timestamptz",
        "Uuid" => "uuid",
        "Ulid" => "ulid",
        "Decimal" | "BigDecimal" => "decimal",
        "Value" | "JsonValue" => "json",
        "IpAddr" => "inet",
        "IpNetwork" => "cidr",
        _ => return None,
    })
}

/// 解析 DDL 中 `CREATE TABLE <name> (...)` 的列：列名（小写）→ 类型族。
///
/// 类型族由列定义中的类型短语归一化而来（去括号参数、识别多词类型如
/// `double precision` / `timestamp with time zone`）；无法识别类型的列
/// 值为 `None`（仅参与列名校验）。
fn parse_create_table_columns(
    ddl: &str,
    table: &str,
) -> Option<HashMap<String, Option<&'static str>>> {
    let mut rest = ddl;
    let needle = format!("create table {}", table.to_lowercase());
    while let Some(idx) = rest.to_lowercase().find(&needle) {
        let after_name = &rest[idx + needle.len()..];
        // 跳到 '(' 开始列定义
        let open = after_name.find('(')?;
        let body = &after_name[open + 1..];
        // 找到与当前 '(' 配对的 ')'（考虑嵌套圆括号，如 CHECK/默认值）
        let mut depth = 1;
        for (offset, ch) in body.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        let cols = split_top_level(body, offset);
                        let mut out = HashMap::new();
                        // 每个「列段」的首个 token 即列名（行级约束如 PRIMARY KEY(...) 无列名，跳过）
                        for segment in cols {
                            let seg = segment.trim();
                            if seg.is_empty() {
                                continue;
                            }
                            let name = seg
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .trim_end_matches(',')
                                .to_lowercase();
                            if !name.is_empty() && !is_table_constraint(&name) {
                                out.insert(name, extract_sql_type(seg));
                            }
                        }
                        return Some(out);
                    }
                }
                _ => {}
            }
        }
        // 该表体未正确闭合，继续往后找同名表定义
        rest = after_name;
    }
    None
}

/// 从列段提取类型短语并归一化为类型族。
///
/// 跳过列名 token，收集后续 token 直到遇到约束关键字（`NOT`、`PRIMARY`、
/// `DEFAULT` 等），再去掉括号参数（`VARCHAR(255)` → `varchar`）。
fn extract_sql_type(segment: &str) -> Option<&'static str> {
    let mut tokens = segment.split_whitespace();
    tokens.next()?; // 列名
    let mut parts: Vec<String> = Vec::new();
    for tok in tokens {
        let base = tok.to_ascii_lowercase();
        let base = base.split('(').next().unwrap_or("");
        if is_type_terminator(base) {
            break;
        }
        parts.push(base.to_string());
    }
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join(" ");
    Some(match joined.as_str() {
        // 多词类型先精确匹配
        "double precision" => "float",
        "character varying" => "text",
        "timestamp with time zone" | "time with time zone" => "timestamptz",
        "timestamp without time zone" | "time without time zone" => "timestamp",
        _ => sql_type_family(&joined)?,
    })
}

/// 归一化后的类型族匹配（单 token 类型）。
fn sql_type_family(t: &str) -> Option<&'static str> {
    // 去尾部修饰词（MySQL 的 UNSIGNED / ZEROFILL）
    let base = t
        .trim_end_matches(" unsigned")
        .trim_end_matches(" zerofill");
    Some(match base {
        "boolean" | "bool" => "bool",
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "int2" | "int4"
        | "int8" | "smallserial" | "serial" | "bigserial" => "int",
        "real" | "float" | "float4" | "float8" | "double" => "float",
        "numeric" | "decimal" => "decimal",
        "char" | "character" | "varchar" | "varchar2" | "text" | "clob" | "nchar" | "nvarchar"
        | "ntext" | "name" | "citext" | "string" | "tinytext" | "mediumtext" | "longtext" => "text",
        "blob" | "tinyblob" | "mediumblob" | "longblob" | "bytea" | "binary" | "varbinary"
        | "bytes" | "raw" => "bytes",
        "date" => "date",
        "time" => "time",
        "timestamp" | "datetime" | "smalldatetime" => "timestamp",
        "timestamptz" | "timetz" => "timestamptz",
        "uuid" => "uuid",
        "json" | "jsonb" => "json",
        "inet" => "inet",
        "cidr" => "cidr",
        _ => return None,
    })
}

/// Rust 类型族与数据库列类型族是否兼容。
///
/// 放宽规则针对跨方言合法实践：SQLite 以 `INTEGER` 存布尔、以 `TEXT` 存
/// UUID / JSON / INET；MySQL 以 `TINYINT(1)` 存布尔。
fn is_compatible(rust_family: &str, sql_family: &str) -> bool {
    match rust_family {
        "int" => matches!(sql_family, "int"),
        "float" => matches!(sql_family, "float" | "decimal"),
        "decimal" => matches!(sql_family, "decimal" | "float"),
        "text" => matches!(sql_family, "text"),
        "bool" => matches!(sql_family, "bool" | "int"),
        "bytes" => matches!(sql_family, "bytes"),
        "date" => matches!(sql_family, "date"),
        "time" => matches!(sql_family, "time"),
        // 带时区与否在存储上常混用（如 DATETIME 存 UTC），互相兼容
        "timestamp" => matches!(sql_family, "timestamp" | "timestamptz"),
        "timestamptz" => matches!(sql_family, "timestamptz" | "timestamp"),
        "uuid" => matches!(sql_family, "uuid" | "text"),
        "ulid" => matches!(sql_family, "uuid" | "text"),
        "json" => matches!(sql_family, "json" | "text"),
        "inet" => matches!(sql_family, "inet" | "text"),
        "cidr" => matches!(sql_family, "cidr" | "inet" | "text"),
        // 未知族不做拦截（由列名校验兜底）
        _ => true,
    }
}

/// 类型短语的终止关键字（遇到即认为类型已结束）。
fn is_type_terminator(base: &str) -> bool {
    matches!(
        base,
        "not"
            | "null"
            | "primary"
            | "unique"
            | "check"
            | "default"
            | "references"
            | "collate"
            | "constraint"
            | "comment"
            | "generated"
            | "identity"
            | "stored"
            | "virtual"
            | "as"
            | "on"
            | "using"
    )
}

/// 在体文本 `body` 的 `close` 位置（'(' 之后的内容）内，按顶层逗号切分列段。
fn split_top_level(body: &str, close: usize) -> Vec<String> {
    let chunk = &body[..close];
    let mut segs = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, ch) in chunk.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                segs.push(chunk[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    segs.push(chunk[start..].to_string());
    segs
}

/// 是否为行级约束段（以不允许取为列名的关键字开头）。
fn is_table_constraint(name: &str) -> bool {
    matches!(
        name,
        "primary" | "unique" | "foreign" | "check" | "constraint" | "key" | "index"
    )
}

/// 若 `ty` 是 `Option<T>`，返回内层 `T` 的引用。
fn option_inner(ty: &Type) -> Option<&Type> {
    if let Type::Path(tp) = ty
        && let Some(seg) = tp.path.segments.last()
        && seg.ident == "Option"
        && let syn::PathArguments::AngleBracketed(args) = &seg.arguments
    {
        return args.args.first().and_then(|a| match a {
            syn::GenericArgument::Type(inner) => Some(inner),
            _ => None,
        });
    }
    None
}

/// 提取类型路径的最后一段名（如 `chrono::NaiveDateTime` → `NaiveDateTime`）。
fn last_segment(ty: &Type) -> Option<String> {
    if let Type::Path(tp) = ty {
        tp.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_type_normalizes_params_and_multiword() {
        assert_eq!(extract_sql_type("tag VARCHAR(255) NOT NULL"), Some("text"));
        assert_eq!(extract_sql_type("score DECIMAL(10,2)"), Some("decimal"));
        assert_eq!(
            extract_sql_type("born_at TIMESTAMP WITH TIME ZONE NOT NULL"),
            Some("timestamptz")
        );
        assert_eq!(
            extract_sql_type("t TIMESTAMP WITHOUT TIME ZONE"),
            Some("timestamp")
        );
        assert_eq!(extract_sql_type("d DOUBLE PRECISION"), Some("float"));
        assert_eq!(extract_sql_type("n INT UNSIGNED"), Some("int"));
    }

    #[test]
    fn sql_type_stops_at_constraints() {
        // DEFAULT 值中的 token 不应混入类型
        assert_eq!(
            extract_sql_type("active BOOLEAN NOT NULL DEFAULT TRUE"),
            Some("bool")
        );
        assert_eq!(extract_sql_type("n TEXT"), Some("text"));
        assert_eq!(extract_sql_type("n"), None);
    }

    #[test]
    fn parse_table_captures_name_and_type() {
        let ddl = "CREATE TABLE users (\n  id BIGINT PRIMARY KEY,\n  email VARCHAR(255) NOT NULL UNIQUE,\n  meta JSONB\n);";
        let cols = parse_create_table_columns(ddl, "users").unwrap();
        assert_eq!(cols.get("id"), Some(&Some("int")));
        assert_eq!(cols.get("email"), Some(&Some("text")));
        assert_eq!(cols.get("meta"), Some(&Some("json")));
    }

    #[test]
    fn rust_family_maps_eco_types_and_unwraps_option() {
        let i64_ty: Type = syn::parse_quote!(i64);
        assert_eq!(rust_type_family(&i64_ty, false), Some("int"));
        let opt_ty: Type = syn::parse_quote!(Option<String>);
        assert_eq!(rust_type_family(&opt_ty, false), Some("text"));
        let dt_ty: Type = syn::parse_quote!(chrono::DateTime<chrono::Utc>);
        assert_eq!(rust_type_family(&dt_ty, false), Some("timestamptz"));
        let json_ty: Type = syn::parse_quote!(Profile);
        assert_eq!(rust_type_family(&json_ty, true), Some("json"));
        // 自定义类型无法归类 → None（跳过类型比对）
        let custom_ty: Type = syn::parse_quote!(Status);
        assert_eq!(rust_type_family(&custom_ty, false), None);
    }

    #[test]
    fn compatibility_matrix_allows_cross_dialect_practices() {
        assert!(is_compatible("int", "int"));
        assert!(!is_compatible("text", "int"));
        // SQLite 布尔存 INTEGER / MySQL 布尔存 TINYINT(1)
        assert!(is_compatible("bool", "int"));
        // SQLite 以 TEXT 存 UUID / JSON
        assert!(is_compatible("uuid", "text"));
        assert!(is_compatible("json", "text"));
        // 时区列互容
        assert!(is_compatible("timestamp", "timestamptz"));
        // 未知族不拦截
        assert!(is_compatible("unknown", "int"));
    }
}

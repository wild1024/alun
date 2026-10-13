//! `#[derive(TypeMapper)]` 四种枚举编码模式的展开与运行期语义。
//!
//! 回归范围：
//!
//! - **D002（S0）**：默认文本模式与 `#[native_enum]` 的 `from_value` 匹配臂
//!   缺少尾随逗号（`match s.as_str() { #( #arms ),* _ => … }`）→ 生成
//!   `expected ',' following match arm` / `proc-macro derive produced unparsable tokens`。
//! - **D020（S1）**：`#[int]` 模式只接受 `Value::I32`，而三库驱动均把整数列
//!   归一读回 `Value::I64` → 读回 `TypeMismatch`。
//!
//! 测试直接调用派生生成的方法（`TypeMapper::to_value` / `from_value`、
//! `From<Enum> for Value` / `TryFrom<&Value>`、`Model::from_row` / `as_row` 与字段访问器），
//! 断言的是展开结果**能编译**且映射语义正确。

use alun_db_core::error::DbError;
use alun_db_core::row::Row;
use alun_db_core::stmt::{Type, Value};
use alun_db_core::traits::{Model, TypeMapper};
use alun_db_macro::{Model, TypeMapper};

/// ① 默认文本模式：变体名即编码。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
enum TextState {
    Draft,
    Issued,
}

/// ② `#[int]` 整数模式：判别值即编码。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
#[int]
enum IntState {
    Draft = 1,
    Issued = 2,
}

/// 非 `Copy` 枚举：`#[int]` 的 `to_value(&self)` 不得要求派生 `Copy`（E0507）。
#[derive(TypeMapper, Debug, Clone, PartialEq, Eq)]
#[int]
enum NonCopyIntState {
    Draft = 1,
    Issued = 2,
}

/// ③ `#[value_map]`：显式编码表。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
#[value_map("draft" => Draft, "issued" => Issued)]
enum MappedState {
    Draft,
    Issued,
}

/// ④ `#[native_enum]`：数据库原生 ENUM 类型名。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
#[native_enum("enum_state")]
enum NativeState {
    Draft,
    Issued,
}

/// 枚举列模型：文本 / 整数 / 原生枚举三列（对应 `alun-ledger` 的 `enum_probe`）。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("enum_probe")]
struct EnumProbe {
    #[key]
    id: i64,
    text_state: TextState,
    int_state: IntState,
    native_state: NativeState,
}

/// 按 `SELECT` 列序构造行。
fn row_of(pairs: Vec<(&str, Value)>) -> Row {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

// ==================== ① 默认文本模式 ====================

#[test]
fn text_mode_maps_variant_names() {
    assert_eq!(TextState::stmt_type(), Type::Text);
    assert_eq!(TextState::Draft.to_value(), Value::Text("Draft".into()));

    for state in [TextState::Draft, TextState::Issued] {
        assert_eq!(TextState::from_value(&state.to_value()).unwrap(), state);
    }

    // 未知标签 → InvalidValue；非文本载荷 → TypeMismatch。
    assert!(matches!(
        TextState::from_value(&Value::Text("bogus".into())),
        Err(DbError::InvalidValue(_))
    ));
    assert!(matches!(
        TextState::from_value(&Value::I32(1)),
        Err(DbError::TypeMismatch {
            expected: "Text",
            actual: "I32"
        })
    ));

    // 生成的 `From`/`TryFrom` 转换与 `TypeMapper` 一致。
    let encoded: Value = TextState::Issued.into();
    assert_eq!(encoded, Value::Text("Issued".into()));
    assert_eq!(TextState::try_from(&encoded).unwrap(), TextState::Issued);
}

// ==================== ② `#[int]` 整数模式 ====================

#[test]
fn int_mode_maps_discriminants_across_read_widths() {
    assert_eq!(IntState::stmt_type(), Type::I32);
    assert_eq!(IntState::Issued.to_value(), Value::I32(2));

    for state in [IntState::Draft, IntState::Issued] {
        assert_eq!(IntState::from_value(&state.to_value()).unwrap(), state);
    }

    // D020：驱动把整数列归一为更宽（或更窄）的有符号整数，映射必须仍然成立。
    assert_eq!(
        IntState::from_value(&Value::I64(2)).unwrap(),
        IntState::Issued
    );
    assert_eq!(
        IntState::from_value(&Value::I16(1)).unwrap(),
        IntState::Draft
    );
    assert_eq!(
        IntState::from_value(&Value::I8(2)).unwrap(),
        IntState::Issued
    );

    // 未知判别值 → InvalidValue。
    assert!(matches!(
        IntState::from_value(&Value::I64(99)),
        Err(DbError::InvalidValue(_))
    ));

    // 越界/非整数载荷 → TypeMismatch{expected: "I32"}。
    for v in [
        Value::I64(i64::MAX),
        Value::U8(2),
        Value::U64(2),
        Value::Text("2".into()),
    ] {
        let actual = v.ty().name();
        assert!(
            matches!(
                IntState::from_value(&v),
                Err(DbError::TypeMismatch {
                    expected: "I32",
                    actual: a
                }) if a == actual
            ),
            "载荷 {v:?} 应报 TypeMismatch{{expected: \"I32\", actual: {actual}}}"
        );
    }

    let encoded: Value = IntState::Draft.into();
    assert_eq!(encoded, Value::I32(1));
    assert_eq!(IntState::try_from(&encoded).unwrap(), IntState::Draft);
}

/// `#[int]` 派生不要求枚举实现 `Copy`（原 `to_value` 的 `*self as i32` 会报 E0507）。
#[test]
fn int_mode_without_copy_impl_compiles_and_maps() {
    assert_eq!(NonCopyIntState::Issued.to_value(), Value::I32(2));
    assert_eq!(
        NonCopyIntState::from_value(&Value::I64(1)).unwrap(),
        NonCopyIntState::Draft
    );
    let encoded: Value = NonCopyIntState::Draft.into();
    assert_eq!(encoded, Value::I32(1));
    assert_eq!(
        NonCopyIntState::try_from(&encoded).unwrap(),
        NonCopyIntState::Draft
    );
}

// ==================== ③ `#[value_map]` ====================

#[test]
fn value_map_mode_maps_explicit_keys() {
    assert_eq!(MappedState::stmt_type(), Type::Text);
    assert_eq!(MappedState::Issued.to_value(), Value::Text("issued".into()));

    for state in [MappedState::Draft, MappedState::Issued] {
        assert_eq!(MappedState::from_value(&state.to_value()).unwrap(), state);
    }

    assert!(matches!(
        MappedState::from_value(&Value::Text("Issued".into())),
        Err(DbError::InvalidValue(_))
    ));
    assert!(matches!(
        MappedState::from_value(&Value::I32(2)),
        Err(DbError::TypeMismatch {
            expected: "Text",
            actual: "I32"
        })
    ));

    let encoded: Value = MappedState::Draft.into();
    assert_eq!(encoded, Value::Text("draft".into()));
    assert_eq!(MappedState::try_from(&encoded).unwrap(), MappedState::Draft);
}

// ==================== ④ `#[native_enum]` ====================

#[test]
fn native_enum_mode_maps_labels_and_text_fallback() {
    assert_eq!(NativeState::stmt_type(), Type::Enum("enum_state"));
    assert_eq!(NativeState::Issued.to_value(), Value::Enum("Issued".into()));

    // 原生枚举列（PG ENUM / MySQL ENUM）：`Value::Enum` 往返。
    for state in [NativeState::Draft, NativeState::Issued] {
        assert_eq!(NativeState::from_value(&state.to_value()).unwrap(), state);
    }

    // 无原生枚举的方言（SQLite 等）把列退化为 TEXT：驱动读回 `Value::Text`。
    assert_eq!(
        NativeState::from_value(&Value::Text("Issued".into())).unwrap(),
        NativeState::Issued
    );

    // 未知标签（两种载荷）→ InvalidValue。
    assert!(matches!(
        NativeState::from_value(&Value::Enum("bogus".into())),
        Err(DbError::InvalidValue(_))
    ));
    assert!(matches!(
        NativeState::from_value(&Value::Text("bogus".into())),
        Err(DbError::InvalidValue(_))
    ));

    // 非枚举/文本载荷 → TypeMismatch。
    assert!(matches!(
        NativeState::from_value(&Value::I32(2)),
        Err(DbError::TypeMismatch {
            expected: "Enum",
            actual: "I32"
        })
    ));

    let encoded: Value = NativeState::Draft.into();
    assert_eq!(encoded, Value::Enum("Draft".into()));
    assert_eq!(NativeState::try_from(&encoded).unwrap(), NativeState::Draft);
}

// ==================== `Model` 派生 + 字段访问器 ====================

#[test]
fn model_maps_enum_columns_from_row() {
    assert_eq!(<EnumProbe as Model>::TABLE, "enum_probe");
    assert_eq!(<EnumProbe as Model>::PRIMARY_KEY, "id");

    // 字段访问器（派生生成的 `FieldExpr`）在两种枚举形态下均可构造。
    let _ = EnumProbe::text_state();
    let _ = EnumProbe::int_state();
    let _ = EnumProbe::native_state();

    // 文本列读回 Text、整数列读回 I64（驱动归一）、原生枚举列读回 Enum/Text（按方言）
    // → 三列都能解码。
    let probe = EnumProbe::from_row(row_of(vec![
        ("id", Value::I64(7)),
        ("text_state", Value::Text("Issued".into())),
        ("int_state", Value::I64(1)),
        ("native_state", Value::Enum("Issued".into())),
    ]))
    .expect("from_row 应能解码各枚举列");
    assert_eq!(
        probe,
        EnumProbe {
            id: 7,
            text_state: TextState::Issued,
            int_state: IntState::Draft,
            native_state: NativeState::Issued,
        }
    );

    // 写回：文本列 → Text，整数列 → I32，原生枚举列 → Enum。
    let row = probe.as_row().expect("as_row");
    assert_eq!(row.get("text_state"), Some(&Value::Text("Issued".into())));
    assert_eq!(row.get("int_state"), Some(&Value::I32(1)));
    assert_eq!(row.get("native_state"), Some(&Value::Enum("Issued".into())));
    assert_eq!(probe.pk_value(), Value::I64(7));

    // 列序与字段序不一致时走按名匹配分支；原生枚举列退化为文本承载也能解码。
    let shuffled = EnumProbe::from_row(row_of(vec![
        ("native_state", Value::Text("Draft".into())),
        ("int_state", Value::I64(2)),
        ("id", Value::I64(8)),
        ("text_state", Value::Text("Draft".into())),
    ]))
    .unwrap();
    assert_eq!(
        shuffled,
        EnumProbe {
            id: 8,
            text_state: TextState::Draft,
            int_state: IntState::Issued,
            native_state: NativeState::Draft,
        }
    );
}

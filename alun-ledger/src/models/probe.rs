//! 类型对抗探针模型（不属于业务表；表结构由测试按方言现场创建）。
//!
//! ## 枚举四模式（缺陷 D002 已修复）
//!
//! 四种枚举承载模式现在都能编译并往返：默认文本模式、`#[int]`、`#[value_map]`
//! （见 `transfer.rs` / `invoice.rs`）、`#[native_enum]`（见 `invoice_state.rs`）。
//! 修复前默认文本模式与 `#[int]` 的派生会生成无法解析的 token 流
//! （`error: expected ',' following match arm`）；`--features derive_probe` 是
//! 该派生的**正向回归**（必须编译通过并往返一致）。

use alun_db::prelude::*;
use rust_decimal::Decimal;
use uuid::Uuid;

/// 文本模式枚举（默认文本模式的**真实派生**；D002 已修复）。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextState {
    Draft,
    Issued,
    Paid,
}

/// 整数模式枚举（`#[int]` 的**真实派生**；D002/D020 已修复）。
#[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
#[int]
pub enum IntState {
    Draft = 1,
    Issued = 2,
    Paid = 3,
}

/// bool / Uuid 探针。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("type_probe")]
pub struct TypeProbe {
    #[key]
    pub id: i64,
    pub flag: bool,
    pub uid: Uuid,
    pub label: String,
}

/// 金额列探针（列类型按方言：NUMERIC / DECIMAL(38,10) / TEXT）。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("money_probe")]
pub struct MoneyProbe {
    #[key]
    pub id: i64,
    pub amount: Decimal,
}

/// 枚举模式探针（两列：文本模式 + 整数模式）。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("enum_probe")]
pub struct EnumProbe {
    #[key]
    pub id: i64,
    pub text_state: TextState,
    pub int_state: IntState,
}

/// 洋葱模型顺序探针（挂 3 个 Hook：A / B / C）。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("hook_probe")]
#[hook(crate::hooks::HookA)]
#[hook(crate::hooks::HookB)]
#[hook(crate::hooks::HookC)]
pub struct HookProbe {
    #[key]
    pub id: i64,
    pub label: String,
}

/// 审计痕迹探针（挂 [`crate::hooks::AuditTraceHook`]）。
#[derive(Model, Debug, Clone, PartialEq)]
#[table("audit_probe")]
#[hook(crate::hooks::AuditTraceHook)]
pub struct AuditProbe {
    #[key]
    pub id: i64,
    pub label: String,
}

/// 编译期 schema 校验探针（**反向回归**）：`users.legacy_id` 在第 5 版迁移里被
/// `DROP COLUMN`，快照解析器现在会同步删除该列（D012 已修复）→ 本模型**必须编译失败**
/// （`strict schema mismatch: column legacy_id is not declared`）。
/// 即：`cargo check -p alun-ledger --features strict_probe` 返回非 0 才是期望结果。
#[cfg(feature = "strict_probe")]
#[derive(Model, Debug, Clone, PartialEq)]
#[table("users")]
#[model(strict)]
pub struct StrictStaleProbe {
    #[key]
    pub id: i64,
    pub tenant_id: i64,
    pub email: String,
    pub legacy_id: Option<String>,
}

/// D002 正向回归：默认文本模式 / `#[int]` 的派生必须编译通过且往返正确。
/// `cargo test -p alun-ledger --features derive_probe` 断言两种模式的运行期语义。
#[cfg(feature = "derive_probe")]
pub mod derive_probe {
    use alun_db::prelude::*;

    /// 默认文本模式（曾无法编译）。
    #[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TextModeProbe {
        Draft,
        Issued,
    }

    /// `#[int]` 模式（曾无法编译）。
    #[derive(TypeMapper, Debug, Clone, Copy, PartialEq, Eq)]
    #[int]
    pub enum IntModeProbe {
        Draft = 1,
        Issued = 2,
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use alun_db_core::stmt::Value;

        /// 两种此前无法编译的模式：写入形态正确、任意整数宽度都能读回、未知值报错。
        #[test]
        fn previously_broken_enum_modes_roundtrip() {
            assert_eq!(
                TextModeProbe::Issued.to_value(),
                Value::Text("Issued".into())
            );
            assert_eq!(
                TextModeProbe::from_value(&Value::Text("Issued".into())).unwrap(),
                TextModeProbe::Issued
            );
            assert!(TextModeProbe::from_value(&Value::Text("Nope".into())).is_err());

            assert_eq!(IntModeProbe::Issued.to_value(), Value::I32(2));
            for v in [Value::I8(2), Value::I16(2), Value::I32(2), Value::I64(2)] {
                assert_eq!(
                    IntModeProbe::from_value(&v).unwrap(),
                    IntModeProbe::Issued,
                    "驱动归一后的 {v:?} 必须可读回"
                );
            }
            assert!(IntModeProbe::from_value(&Value::I64(9)).is_err());
            assert!(IntModeProbe::from_value(&Value::Text("2".into())).is_err());
        }
    }
}

//! `Params` —— SQL WHERE 片段的参数元组。

use crate::stmt::Value;

/// SQL WHERE 片段的参数。
///
/// 由 `impl Params for (...)` 提供元组实现，支持 0 到 12 元组。
///
/// 每个元素只要能 `Into<Value>` 即可（如 `i32`、`String`、`&str`、
/// `Option<T>`、chrono 类型等），不要求实现 `TypeMapper`。
///
/// ## 示例
///
/// ```ignore
/// User::find_by(&db, "status = ? and name = ?", (1i32, "Alice")).await?;
/// ```
pub trait Params: Send + Sync {
    /// 参数列表。
    fn into_values(self) -> Vec<Value>;
}

impl Params for () {
    fn into_values(self) -> Vec<Value> {
        vec![]
    }
}

macro_rules! impl_params_tuple {
    ($($idx:tt: $ty:ident),+) => {
        impl<$($ty: Into<Value> + Send + Sync),+> Params for ($($ty,)+) {
            fn into_values(self) -> Vec<Value> {
                vec![$(self.$idx.into(),)+]
            }
        }
    };
}

impl_params_tuple!(0: A);
impl_params_tuple!(0: A, 1: B);
impl_params_tuple!(0: A, 1: B, 2: C);
impl_params_tuple!(0: A, 1: B, 2: C, 3: D);
impl_params_tuple!(0: A, 1: B, 2: C, 3: D, 4: E);
impl_params_tuple!(0: A, 1: B, 2: C, 3: D, 4: E, 5: F);
impl_params_tuple!(0: A, 1: B, 2: C, 3: D, 4: E, 5: F, 6: G);
impl_params_tuple!(0: A, 1: B, 2: C, 3: D, 4: E, 5: F, 6: G, 7: H);

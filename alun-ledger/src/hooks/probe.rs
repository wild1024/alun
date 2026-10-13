//! Hook 顺序探针：验证洋葱模型「`before_*` 正序 / `after_*` 逆序」。

use alun_db::prelude::*;
use alun_db_core::row::Row;
use std::cell::RefCell;

thread_local! {
    static ORDER: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// 取出并清空当前线程记录的执行序列（如 `["A:before_insert", "B:before_insert", ...]`）。
pub fn drain_order() -> Vec<String> {
    ORDER.with(|o| std::mem::take(&mut *o.borrow_mut()))
}

fn record(label: &str) {
    ORDER.with(|o| o.borrow_mut().push(label.to_string()));
}

macro_rules! order_probe {
    ($ty:ty, $label:literal) => {
        impl Hook for $ty {
            fn before_insert(&self, _row: &mut Row) -> alun_db_core::error::Result<()> {
                record(concat!($label, ":before_insert"));
                Ok(())
            }
            fn after_insert(&self, _row: &Row) -> alun_db_core::error::Result<()> {
                record(concat!($label, ":after_insert"));
                Ok(())
            }
            fn before_update(&self, _row: &mut Row) -> alun_db_core::error::Result<()> {
                record(concat!($label, ":before_update"));
                Ok(())
            }
            fn after_update(&self, _row: &Row) -> alun_db_core::error::Result<()> {
                record(concat!($label, ":after_update"));
                Ok(())
            }
            fn before_delete(&self, _pk: &Value) -> alun_db_core::error::Result<()> {
                record(concat!($label, ":before_delete"));
                Ok(())
            }
        }
    };
}

/// 顺序探针 A。
pub struct HookA;
/// 顺序探针 B。
pub struct HookB;
/// 顺序探针 C。
pub struct HookC;

order_probe!(HookA, "A");
order_probe!(HookB, "B");
order_probe!(HookC, "C");

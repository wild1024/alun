//! 故障注入（`FailPoint`）——转发到库内实现，便于跨进程/跨 crate 使用。

pub use alun_ledger::services::faults::{FailPoint, set};

/// 打开注入点。
pub fn arm(fp: FailPoint) {
    set(Some(fp));
}

/// 关闭注入点。
pub fn disarm() {
    set(None);
}

/// 在 guard 作用域内开启注入点，离开作用域自动关闭（panic 安全）。
pub struct Armed;

impl Armed {
    /// 打开注入点并返回 guard。
    pub fn new(fp: FailPoint) -> Self {
        arm(fp);
        Armed
    }
}

impl Drop for Armed {
    fn drop(&mut self) {
        disarm();
    }
}

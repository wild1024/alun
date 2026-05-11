//! 日期工具：格式化、相对时间、时间戳互转

use chrono::{DateTime, Utc, Local};

/// 日期工具 —— 格式化、相对时间描述、时间戳互转、日界计算
///
/// 所有 DateTime 均使用 UTC 时区，`Date` 为零大小结构体（无状态）。
pub struct Date;

impl Date {
    /// 获取当前 UTC 时间
    pub fn now() -> DateTime<Utc> { Utc::now() }

    /// 获取当前本地时间
    pub fn now_local() -> DateTime<Local> { Local::now() }

    /// 日期格式化
    pub fn fmt(dt: &DateTime<Utc>, fmt: &str) -> String {
        dt.format(fmt).to_string()
    }

    /// 从时间戳创建 DateTime
    pub fn from_timestamp(ts: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(ts, 0).unwrap_or_default()
    }

    /// 相对时间描述（如：3分钟前、2小时前）
    pub fn relative(ts: i64) -> String {
        let then = DateTime::from_timestamp(ts, 0).unwrap_or_default();
        let diff = Utc::now() - then;

        if diff.num_seconds() < 60 { format!("{}秒前", diff.num_seconds()) }
        else if diff.num_minutes() < 60 { format!("{}分钟前", diff.num_minutes()) }
        else if diff.num_hours() < 24 { format!("{}小时前", diff.num_hours()) }
        else if diff.num_days() < 30 { format!("{}天前", diff.num_days()) }
        else if diff.num_days() < 365 { format!("{}个月前", diff.num_days() / 30) }
        else { format!("{}年前", diff.num_days() / 365) }
    }

    /// 获取当天的起始时刻（00:00:00 UTC）
    pub fn begin_of_day(dt: &DateTime<Utc>) -> DateTime<Utc> {
        dt.date_naive().and_hms_opt(0, 0, 0).unwrap()
            .and_utc()
    }

    /// 获取当天的结束时刻（23:59:59 UTC）
    pub fn end_of_day(dt: &DateTime<Utc>) -> DateTime<Utc> {
        dt.date_naive().and_hms_opt(23, 59, 59).unwrap()
            .and_utc()
    }
}

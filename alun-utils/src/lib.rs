//! 工具类：字符串、日期、脱敏、短ID、验证、Web解析、加密、XSS净化、UA解析

pub mod crypto;
pub mod date;
pub mod export;
pub mod mask;
pub mod serial;
pub mod sid;
pub mod str;
pub mod ua;
pub mod valid;
pub mod web;

#[cfg(feature = "xss")]
pub mod xss;

pub use crypto::Crypto;
pub use date::Date;
#[cfg(feature = "xlsx")]
pub use export::XlsxSheet;
pub use export::{Export, Import};
pub use mask::Mask;
pub use serial::{
    CyclePeriod, FormatEngine, IncrementStrategy, MemorySerialBackend, SerialError,
    SerialGenerator, SerialRecord, SerialRule,
};
pub use sid::Sid;
pub use str::StrExt;
pub use str::{
    InputCleaner, clean_email, clean_password, clean_string_param, format_file_size,
    generate_invite_code, generate_random_alphanum, generate_random_digits, parse_json_value,
    sanitize_filename,
};
pub use ua::{UaInfo, parse_user_agent};
pub use valid::Valid;
pub use web::WebExt;
pub use web::extract_client_ip;

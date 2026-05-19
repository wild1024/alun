//! 工具类：字符串、日期、脱敏、短ID、验证、Web解析、加密、XSS净化、UA解析

pub mod str;
pub mod date;
pub mod mask;
pub mod sid;
pub mod valid;
pub mod web;
pub mod crypto;
pub mod export;
pub mod ua;

#[cfg(feature = "xss")]
pub mod xss;

pub use str::StrExt;
pub use str::{
    sanitize_filename, parse_json_value, format_file_size,
    clean_string_param, clean_email, clean_password, InputCleaner,
    generate_invite_code, generate_random_digits, generate_random_alphanum,
};
pub use date::Date;
pub use mask::Mask;
pub use sid::Sid;
pub use valid::Valid;
pub use web::WebExt;
pub use web::extract_client_ip;
pub use crypto::Crypto;
pub use export::{Export, Import, ExportFormat};
pub use ua::{parse_user_agent, UaInfo};

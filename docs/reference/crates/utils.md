---
id: reference.crates.utils
title: `alun-utils` — 工具集
summary: `alun-utils` — 工具集——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [utils, reference, str, date, crypto, export]
status: active
owner: alun-core-team
source_of_truth: [alun-utils/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-utils` — 工具集

> 本文件是 [`crates.md`](../crates.md) 的 `utils` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun-utils/src/`

## 子模块

### `str` — `StrExt` Trait

| 方法              | 说明                           |
| --------------- | ---------------------------- |
| `to_snake()`    | `helloWorld` → `hello_world` |
| `to_camel()`    | `hello_world` → `helloWorld` |
| `is_blank()`    | 是否为空字符串                      |
| `truncate(max)` | 按字符数截断，超出末尾补 `...`           |
| `random(len)`   | 生成指定长度的随机字母数字串               |
| `has_text()`    | 非空白字符（`is_blank` 取反）         |

### `str` — 自由函数

| 函数                            | 说明                               | 示例                                           |
| ----------------------------- | -------------------------------- | -------------------------------------------- |
| `sanitize_filename(s)`        | 将非字母/数字/点/横线/下划线的字符替换为 `_`       | `file<name>.txt` → `file_name.txt`           |
| `parse_json_value(s)`         | 解析 JSON 字符串为 `serde_json::Value` | `"{\"k\":1}"` → `Ok(Value)`                  |
| `format_file_size(bytes)`     | 字节数 → 人类可读（B/KB/MB/GB/TB/PB）     | `1500000` → `"1.43 MB"`                      |
| `clean_string_param(s)`       | 去除前后空格                           | `"  hello  "` → `"hello"`                    |
| `clean_email(email)`          | 去除前后空格 + 转小写                     | `" A@B.COM "` → `"a@b.com"`                  |
| `clean_password(pwd)`         | 只去除前后空格，保留中间空格                   | `" pass 123 "` → `"pass 123"`                |
| `generate_invite_code()`      | 生成 12 位随机邀请码                     | `"a1B2c3D4e5F6"`                             |
| `generate_random_digits(n)`   | 生成指定位数的随机数字串（不含 `0`）             | `generate_random_digits(6)` → `"573192"`     |
| `generate_random_alphanum(n)` | 生成无易混淆字符（`0`/`O`/`I`/`l`）的随机串    | `generate_random_alphanum(8)` → `"aB3kM9xQ"` |

### `str` — `InputCleaner` 输入清理器

| 方法                                           | 说明                                                          |
| -------------------------------------------- | ----------------------------------------------------------- |
| `clean_register_input(email, pwd, nickname)` | 清理注册三项：邮箱小写去空格、密码去空格、昵称去空格，返回 `(email, password, nickname)` |
| `clean_login_input(email, pwd)`              | 清理登录两项：邮箱小写去空格、密码去空格，返回 `(email, password)`                 |

### `date` — `Date` 工具

| 方法                   | 说明             |
| -------------------- | -------------- |
| `now()`              | 当前 UTC 时间      |
| `now_local()`        | 当前本地时间         |
| `fmt(dt, fmt)`       | 格式化日期          |
| `from_timestamp(ts)` | 时间戳 → DateTime |
| `relative(ts)`       | 相对时间（"3分钟前"）   |
| `begin_of_day(dt)`   | 当天 00:00:00    |
| `end_of_day(dt)`     | 当天 23:59:59    |

### `mask` — `Mask` 脱敏

| 方法                          | 示例输入                   | 输出                   | 说明               |
| --------------------------- | ---------------------- | -------------------- | ---------------- |
| `mobile(s)`                 | `13812345678`          | `138****5678`        | 手机号：保留前3后4位       |
| `email(s)`                  | `a@b.com`              | `a***@b.com`         | 邮箱：保留首字符和域名       |
| `id_card(s)`                | `320112199001011234`   | `3201****1234`       | 身份证：保留前4后4位       |
| `name(s)`                   | `张三丰`                  | `张**`                | 姓名：保留首字符，其余用 `*`  |
| `bank_card(s)`              | `6222021234567890`     | `6222 **** 7890`     | 银行卡：保留前4后4位       |
| `user_id(s)`                | `user_abc123`          | `us****23`           | 用户ID：保留前2后2字符     |
| `password(s)`               | `secret`               | `******`             | 密码：固定返回 `******`  |
| `address(s)`                | `北京市海淀区中关村`           | `北京市海淀****`          | 地址：保留前6字符         |
| `license_plate(s)`          | `京A12345`              | `京****5`             | 车牌：保留首字符和末位       |
| `mask_by_type(type, value)` | `("mobile", "1381...")` | `"138****5678"`      | 按数据类型自动选择脱敏方式     |

**`mask_json_value`** **函数**：递归对 JSON 对象脱敏，自动检测手机号/身份证/邮箱格式。

| 参数                          | 说明                           |
| --------------------------- | ---------------------------- |
| `value: Value`              | 待脱敏的 JSON                    |
| `sensitive_fields: &[&str]` | 敏感字段名列表（匹配到的字段值替换为 `"****"`） |

```rust,ignore
use alun_utils::mask::mask_json_value;
use serde_json::json;

let input = json!({"password": "secret123", "mobile": "13812345678", "name": "张三"});
let masked = mask_json_value(input, &["password", "mobile"]);

// masked → {"password": "****", "mobile": "****", "name": "张三"}
// 字段名匹配 → 值替换为 "****"；其他字段值若匹配手机号/身份证/邮箱格式也会自动脱敏
```

### `sid` — `Sid` 短 ID

| 方法        | 说明            |
| --------- | ------------- |
| `short()` | 16 位 hex      |
| `tiny()`  | 8 位 hex       |
| `tsid()`  | 时间戳 + 随机数     |
| `uuid()`  | UUID v4（36 字符标准格式） |
| `uuid7()` | UUID v7（36 字符标准格式，时间有序） |

### `ua` — User-Agent 解析

从 User-Agent 字符串中提取设备类型、浏览器类型和操作系统信息。

**`UaInfo`** — 解析结果结构体：

| 字段             | 类型       | 说明                               |
| -------------- | -------- | -------------------------------- |
| `device_type`  | `String` | 设备类型：`"PC"` / `"MOBILE"` / `"TABLET"` / `"UNKNOWN"` |
| `browser_type` | `String` | 浏览器：`"Chrome"` / `"Firefox"` / `"Safari"` / `"Edge"` / `"Unknown"` |
| `os_type`      | `String` | 操作系统：`"Windows"` / `"macOS"` / `"Linux"` / `"iOS"` / `"Android"` / `"Unknown"` |

**`parse_user_agent(ua)`** — 解析 User-Agent 字符串，返回 `UaInfo`。

```rust,ignore
use alun_utils::parse_user_agent;

let info = parse_user_agent(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0.0.0 Safari/537.36"
);
assert_eq!(info.device_type, "PC");
assert_eq!(info.browser_type, "Chrome");
assert_eq!(info.os_type, "Windows");
```

### `valid` — `Valid` 验证

| 方法                             | 说明                          |
| ------------------------------ | --------------------------- |
| `is_email(s)`                  | 邮箱格式                        |
| `is_mobile(s)`                 | 手机号（中国大陆）                   |
| `is_phone(s)`                  | 电话号码（E.164 格式）              |
| `is_url(s)`                    | URL 格式                      |
| `is_ipv4(s)`                   | IPv4 格式                     |
| `is_strong_password(s)`        | 强密码（≥8位，含大小写+数字+特殊字符）       |
| `is_username(s)`               | 用户名（3\~50位字母/数字/下划线/点/横线）   |
| `is_color(s)`                  | 十六进制颜色（#RRGGBB）             |
| `is_uuid(s)`                   | UUID（v1\~v7 均支持）            |
| `is_id_card(s)`                | 中国居民身份证号（含校验位）              |
| `is_date(s)`                   | 日期格式（YYYY-MM-DD）            |
| `is_datetime(s)` | 日期时间（ISO 8601 / RFC 3339 或 YYYY-MM-DD） |
| `is_file_extension(filename, allowed)` | 验证文件扩展名是否在允许列表中（`allowed: &[&str]`） |
| `is_json(s)`                   | 合法 JSON 字符串                 |
| `is_base64(s)`                 | Base64 编码                   |
| `is_digits(s)`                 | 纯数字                         |
| `is_alphanumeric(s)`           | 字母+数字组合                     |
| `len_between(s, min, max)`     | 字符串长度范围                     |
| `has_html(s)`                  | 检测是否包含 HTML 标签              |
| `is_html_free(s)`              | 检测是否不包含 HTML 标签             |
| `format_validation_errors(e)`  | 将 `ValidationErrors` 转为可读消息 |

> **`format_validation_errors`** 需要启用 `validator-integration` feature，用于将 `validator::ValidationErrors` 转换为 `"字段名: 错误描述"` 格式的可读字符串。

### `crypto` — `Crypto` 加密

| 方法                                           | 签名 / 说明                                                        |
| -------------------------------------------- | -------------------------------------------------------------- |
| `sha256(data)`                               | `fn sha256(data: &str) -> String`（hex）                         |
| `hmac_sha256(key, data)`                     | `fn hmac_sha256(key: &[u8], data: &str) -> String`             |
| `aes_encrypt(key, plaintext)`                | `fn aes_encrypt(key: &[u8], plaintext: &str) -> Option<(String, String)>`，返回 `(密文 Base64, nonce Base64)`，key 必须 32 字节 |
| `aes_decrypt(key, cipher, nonce)`            | `fn aes_decrypt(key: &[u8], ciphertext_b64: &str, nonce_b64: &str) -> Option<String>` |
| `hash_password(pwd)`                         | `fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error>`（Argon2，盐自动生成） |
| `verify_password(pwd, hash)`                 | `fn verify_password(password: &str, hash: &str) -> Result<bool, String>`（自动识别 `$argon2`/`$2` 前缀，即 Argon2 或 BCrypt） |
| `base64_url_encode(data)`                    | `fn base64_url_encode(data: &[u8]) -> String`                  |
| `base64_url_decode(s)`                       | `fn base64_url_decode(s: &str) -> Option<Vec<u8>>`             |
| `random_key()`                               | `fn random_key() -> Vec<u8>`（32 字节）                            |
| `random_token(len)`                          | `fn random_token(len: usize) -> String`（`2 * len` 个 hex 字符）    |

### `serial` — 单号生成体系

工厂/规则/记录/后端四件套，crate 根即导出（`alun_utils::{SerialRule, SerialGenerator, …}`）：

| 类型                          | 说明                                                                                  |
| --------------------------- | ----------------------------------------------------------------------------------- |
| `SerialRule`                | 规则：`key`、`format`（如 `"ORD{YYYY}{MM}{DD}{SEQ:8}"`）、`cycle`、`initial_value`、`step`、`is_enabled`；builder：`new(key, format)`、`with_cycle()`、`with_initial_value()`、`with_step()` |
| `CyclePeriod`               | `NoCycle`（默认）/ `Daily` / `Monthly` / `Yearly`，`current_value()`            |
| `IncrementStrategy`         | `Sequential`（默认）/ `Random { max }`                                    |
| `SerialRecord`              | 生成记录：`rule_key`、`serial_no`、`counter`、`cycle_value`、`created_at`                       |
| `FormatEngine`              | `compile(format) -> Result<Self, SerialError>`、`render(seq, ts: Option<i64>) -> String` |
| `SerialError`               | `RuleNotFound` / `RuleDisabled` / `FormatError` / `CounterOverflow` / `StorageError` |
| `SerialGenerator`（trait）    | `generate` / `batch_generate` / `peek` / `register_rule` / `remove_rule` / `enable_rule` / `disable_rule` / `query_records` / `list_rules`（全异步，返回 `Result<_, SerialError>`） |
| `MemorySerialBackend`       | 内存实现（`new()`，默认最多 10000 条；`with_max_records(n)`），实现 `SerialGenerator`              |

> `alun_utils::serial` 里**没有**全局生成器访问函数；应用侧通过 `alun_plugin::serial_plugin::SerialPlugin` 的 `generator()` 拿到 `Arc<dyn SerialGenerator>`。

### `web` — `WebExt` Web 工具

| 方法                                                     | 说明                            |
| ------------------------------------------------------ | ----------------------------- |
| `WebExt::domain(url_str) -> Option<String>`            | 提取域名                          |
| `WebExt::path(url_str) -> Option<String>`              | 提取路径                          |
| `WebExt::real_ip(headers: &[(String, String)], remote_addr: &str) -> String` | 依据转发头推断真实 IP |
| `WebExt::is_private_ip(ip: &str) -> bool`              | 内网地址判断                        |
| `WebExt::build_query(params: &[(&str, &str)]) -> String` | 拼接 Query String             |
| `extract_client_ip(headers: &HeaderMap, connect_info: &SocketAddr) -> String` | 自由函数：从请求头 + 连接信息取客户端 IP |

### `export` — `Export` / `Import` 导出导入

| 方法                                                                    | 说明                                     |
| --------------------------------------------------------------------- | -------------------------------------- |
| `Export::to_csv(columns, rows)`                                       | `(&[&str], &[HashMap<String, String>]) -> Result<String>` |
| `Export::to_csv_bytes(columns, rows)`                                 | `-> Result<Vec<u8>>`，**带 UTF-8 BOM 的 CSV**（Excel 可直接打开；仍不是 XLSX） |
| `Export::to_json(items)`                                              | `(&[T]) -> Result<String>`（pretty JSON） |
| `Import::from_csv(csv_str)`                                           | `-> Result<Vec<HashMap<String, String>>>` |
| `Import::from_json::<T>(json_str)`                                    | `-> Result<Vec<T>>`                     |

**真 XLSX（需 `xlsx` feature）** —— 写用 `rust_xlsxwriter`、读用 `calamine`：

| 方法                                                                    | 说明                                     |
| --------------------------------------------------------------------- | -------------------------------------- |
| `Export::to_xlsx_bytes(columns, rows)`                                 | `-> Result<Vec<u8>>`，**真正的 OOXML 工作簿**（表头加粗 + 冻结 + 列宽自适应） |
| `Export::to_xlsx_typed(columns, items)`                                | `(&[&str], &[T: Serialize]) -> Result<Vec<u8>>` |
| `Export::to_xlsx_workbook(&[XlsxSheet])`                               | 多 sheet 工作簿；`XlsxSheet::new(name, columns, rows)` |
| `Import::xlsx_sheet_names(bytes)`                                      | `-> Result<Vec<String>>`                |
| `Import::from_xlsx(bytes)` / `from_xlsx_sheet(bytes, sheet)`           | `-> Result<Vec<HashMap<String, String>>>`，首行作表头 |
| `Import::from_xlsx_typed::<T>(bytes)`                                  | `-> Result<Vec<T>>`                     |

> 产物与读写侧的**类型推断规则一致**：只有能"原样往返"的文本（如 `42`）才落成数值
> 单元格，`"00123"` 这类保持文本，避免前导零被 Excel 吃掉。
> 旧 `Export::to_xlsx`（BOM + CSV）与空壳枚举 `ExportFormat` 已**删除**。

### `xss` — HTML/XSS 净化（需启用 `xss` feature）

基于 `ammonia` crate 的 HTML 净化工具。**按需启用**，仅当后端接收/返回用户富文本时需要。

| 函数                           | 说明                             |
| ---------------------------- | ------------------------------ |
| `sanitize_html(html)`        | 使用默认规则净化 HTML（保留安全标签如 p/a/img） |
| `sanitize_html_strict(html)` | 严格净化（strip 所有标签，仅保留纯文本）        |
| `has_potential_xss(html)`    | 检测 HTML 是否包含潜在 XSS 载荷          |

```toml
# Cargo.toml（经 alun 的 xss feature 转发；直接依赖 alun-utils 等价）
alun = { version = "0.3", features = ["xss"] }
```

## 使用示例

```rust,ignore
use alun_utils::*;

// ── 字符串转换 ──
"helloWorld".to_snake();           // → "hello_world"
"hello_world".to_camel();           // → "helloWorld"
"".is_blank();                      // → true
"  ".is_blank();                    // → true

// ── 字符串清理 ──
sanitize_filename("file<name>.txt");       // → "file_name.txt"
clean_email("  User@Mail.COM  ");          // → "user@mail.com"
clean_string_param("  hello  ");           // → "hello"
clean_password("  pass 123  ");            // → "pass 123"

// ── 输入清理器 ──
let (email, pwd, nick) = InputCleaner::clean_register_input(" A@B.com ", " 123 ", " Tom ");
// → ("a@b.com", "123", "Tom")
let (email, pwd) = InputCleaner::clean_login_input(" A@B.com ", " 123 ");

// ── 格式化 ──
format_file_size(0);                       // → "0 B"
format_file_size(1_500_000);               // → "1.43 MB"
parse_json_value(r#"{"key": 1}"#);         // → Ok(Value({"key": Number(1)}))

// ── 随机生成 ──
generate_invite_code();                    // → 12位随机邀请码（字母数字）
generate_random_digits(6);                 // → 6位数字（如 "573192"，不含0）
generate_random_alphanum(8);               // → 8位无混淆字符（如 "aB3kM9xQ"）

// ── 日期操作 ──
let now = Date::now();                                           // UTC
let local = Date::now_local();                                   // 本地时间
Date::fmt(&now, "%Y-%m-%d %H:%M:%S");                           // → "2026-05-06 08:16:22"
Date::relative(now.timestamp());                                 // → "3分钟前"
Date::begin_of_day(&now);                                        // → 当天 00:00:00
Date::from_timestamp(1700000000);                                // 时间戳 → DateTime

// ── 脱敏 ──
Mask::mobile("13812345678");                                     // → "138****5678"
Mask::email("alice@company.com");                                // → "a***@company.com"
Mask::id_card("320112199001011234");                             // → "3201****1234"
Mask::name("张三丰");                                              // → "张**"
Mask::bank_card("6222021234567890");                             // → "6222 **** 7890"
Mask::user_id("user_abc123");                                    // → "us****23"
Mask::password("secret");                                        // → "******"
Mask::address("北京市海淀区中关村");                                   // → "北京市海淀****"
Mask::license_plate("京A12345");                                  // → "京****5"
Mask::mask_by_type("mobile", "13812345678");                    // → "138****5678"（按类型自动选择）

// ── JSON 脱敏 ──
use alun_utils::mask::mask_json_value;
use serde_json::json;
let input = json!({"password": "secret", "mobile": "13812345678"});
let masked = mask_json_value(input, &["password"]);

// ── ID 生成 ──
Sid::short();                     // → "a1b2c3d4e5f6a7b8"（16位hex）
Sid::tiny();                      // → "a1b2c3d4"（8位hex）
Sid::tsid();                      // 时间戳+随机数混合
Sid::uuid();                      // → UUID v4
Sid::uuid7();                     // → UUID v7（时间有序，适合主键）

// ── User-Agent 解析 ──
let info = parse_user_agent(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36"
);
info.device_type;                             // → "PC"
info.browser_type;                            // → "Chrome"
info.os_type;                                 // → "Windows"

// ── 验证 ──
Valid::is_email("a@b.com");                          // → true
Valid::is_mobile("13812345678");                     // → true
Valid::is_phone("+8613812345678");                   // → true
Valid::is_url("https://example.com");                // → true
Valid::is_ipv4("192.168.1.1");                       // → true
Valid::is_strong_password("Abc@12345");              // → true
Valid::is_username("john_doe");                      // → true
Valid::is_color("#FF00AA");                          // → true
Valid::is_uuid("550e8400-e29b-41d4-a716-446655440000"); // → true
Valid::is_id_card("110101199003077758");             // → true（含校验位）
Valid::is_date("2024-01-01");                        // → true
Valid::is_datetime("2024-01-01T00:00:00Z");          // → true
Valid::is_json(r#"{"key": 1}"#);                     // → true
Valid::is_base64("SGVsbG8=");                        // → true
Valid::is_digits("123456");                          // → true
Valid::is_alphanumeric("abc123");                    // → true
Valid::len_between("hello", 2, 10);                  // → true
Valid::has_html("<div>hello</div>");                 // → true
Valid::is_html_free("plain text");                   // → true
Valid::is_file_extension("photo.jpg", &["jpg", "png"]); // → true

// ── 加密 ──
Crypto::sha256("alun");                              // → SHA256 hex
Crypto::random_key();                                // → 32 字节随机密钥 Vec<u8>
Crypto::random_token(32);                            // → 64 位 hex token（2 * len）
Crypto::hash_password("pass123");                    // → Result<String, argon2::password_hash::Error>
Crypto::verify_password("pass123", &hash);           // → Result<bool, String>（自动检测 Argon2/BCrypt）

let key = Crypto::random_key();                      // 32 字节
if let Some((cipher_b64, nonce_b64)) = Crypto::aes_encrypt(&key, "secret data") {
    let plain = Crypto::aes_decrypt(&key, &cipher_b64, &nonce_b64);   // → Option<String>
}

// ── Web 工具 ──
WebExt::domain("https://a.com/path");                // → Some("a.com")
WebExt::path("https://a.com/path");                  // → Some("/path")
WebExt::build_query(&[("a", "1"), ("b", "2")]);      // → "?a=1&b=2"

// ── CSV/JSON 导出 ──
use std::collections::HashMap;
let records = vec![
    HashMap::from([("name","张三"), ("age","28")]),
    HashMap::from([("name","李四"), ("age","32")]),
];
let csv = Export::to_csv(&["name", "age"], &records)?;
let json = Export::to_json(&records)?;

// ── XSS 净化（需 xss feature） ──
use alun_utils::xss;
let safe = xss::sanitize_html("<script>alert(1)</script><p>Hello</p>");
assert_eq!(safe, "<p>Hello</p>");

let strict = xss::sanitize_html_strict("<p>Hello</p>");
assert_eq!(strict, "Hello");                   // 仅保留纯文本

let malicious = xss::has_potential_xss("<script>alert(1)</script>");
assert!(malicious);

// ── 在 Handler 中使用 ──
#[alun::get("/api/demo/utils")]
async fn utils_demo(Query(params): Query<HashMap<String, String>>) -> Res<serde_json::Value> {
    let input = params.get("input").cloned().unwrap_or_default();
    Res::ok(json!({
        "to_snake": input.to_snake(),
        "to_camel": input.to_camel(),
        "is_email": Valid::is_email(&input),
        "masked": Mask::mobile(&input),
        "timestamp": Sid::tsid(),
        "size_formatted": format_file_size(1_500_000),
        "invite_code": generate_invite_code(),
    }))
}
```

***

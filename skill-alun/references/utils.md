# Utilities (`alun-utils`)

200+ utility functions across these modules.

| 模块 | 内容 |
|------|------|
| `str` | `StrExt`（驼峰/蛇形互转、截断、判空、随机串）、文件名与输入清理、随机数字/字母数字、`format_file_size`、`parse_json_value` |
| `date` | `Date`：格式化、相对时间、时间戳互转、日界计算 |
| `mask` | `Mask`：手机/邮箱/身份证/银行卡/姓名/用户ID/地址/车牌脱敏；`mask_json_value` 递归脱敏 JSON |
| `sid` | `Sid`：UUID v4/v7、16 位短 ID、8 位微 ID、20 位 TSID |
| `valid` | `Valid`：邮箱/手机/固话/URL/用户名/颜色/纯数字/UUID/身份证/日期时间/JSON/HTML/文件扩展名等布尔校验 |
| `crypto` | `Crypto`：SHA-256、HMAC-SHA256、AES-256-GCM、Argon2 密码哈希（可验 BCrypt）、Base64URL、随机密钥/Token |
| `export` | `Export` / `Import`：CSV / JSON；**真 XLSX 需 `xlsx` feature**（`Export::to_xlsx_*` / `Import::from_xlsx*` / `XlsxSheet`） |
| `web` | `WebExt`、`extract_client_ip`、`is_private_ip` |
| `ua` | `parse_user_agent` → `UaInfo` |
| `serial` | 单号引擎：`SerialRule` / `FormatEngine` / `CyclePeriod` / `IncrementStrategy` / `MemorySerialBackend` |
| `xss` | HTML 净化（需 `xss` feature，见文末） |

> **从哪导入**：`alun` 门面已再导出 `Crypto` / `Mask` / `Sid` / `Valid`（在 `alun::prelude`）与单号类型
> （`alun::SerialRule`、`CyclePeriod`、`IncrementStrategy`、`MemorySerialBackend`、`SerialGenerator`、
> `SerialRecord`、`SerialError`、`FormatEngine`、`SerialPlugin`），用这些**不必**声明 `alun-utils`。
>
> 其余（`StrExt` / `Date` / `Export` / `Import` / `WebExt` / `UaInfo` / 顶层清理与随机函数、
> `web::is_private_ip`、`mask::mask_json_value`）门面未再导出，需在 `Cargo.toml` 直接声明
> `alun-utils`（没有对应的 `alun` feature）；直接依赖底层 crate 依然可用，只是这些名字没有门面路径。
> XSS 是例外——见文末，经 `alun` 的 `xss` feature 转发。

## String Manipulation

```rust,no_run
use alun_utils::StrExt;
use alun_utils::{clean_email, clean_password, clean_string_param, sanitize_filename};

# fn demo() {
let _ = "helloWorld".to_snake();     // → hello_world
let _ = "hello_world".to_camel();    // → helloWorld
let _ = "".is_blank();               // → true
let _ = "  x  ".has_text();          // → true（is_blank 的取反）

let _ = sanitize_filename("file<name>.txt");  // → file_name.txt
let _ = clean_email("  User@Mail.COM  ");     // → user@mail.com
let _ = clean_string_param("  hello  ");      // → hello
let _ = clean_password("  pass  ");           // → pass
let _ = "abcdef".truncate(3);                 // → abc...（按**字节**截断，超长补 "..."）
let _ = <str as StrExt>::random(8);           // 8 位小写字母+数字（关联函数，非方法）
# }
```

> `StrExt::truncate` 用 `&self[..max]` 做字节切片：截断点多字节字符时会 panic，中文场景请自行按
> `chars()` 截断。`StrExt::random(len)` 是关联函数，调用形式 `<str>::random(8)`。

## Input Cleaners

```rust,no_run
use alun_utils::InputCleaner;

# fn demo() {
let (_email, _pwd, _nick) = InputCleaner::clean_register_input(" A@B.com ", " 123 ", " Tom ");
let (_email, _pwd) = InputCleaner::clean_login_input(" A@B.com ", " 123 ");
# }
```

## Date Utilities

```rust,no_run
use alun_utils::Date;

# fn demo() {
let now = Date::now();                                // DateTime<Utc>
let _ = Date::now_local();                            // DateTime<Local>
let _ = Date::fmt(&now, "%Y-%m-%d %H:%M:%S");
let _ = Date::relative(now.timestamp());              // → "3分钟前"
let _ = Date::begin_of_day(&now);                     // 当天 00:00:00 UTC
let _ = Date::end_of_day(&now);                       // 当天 23:59:59 UTC
let _ = Date::from_timestamp(1700000000);
# }
```

## Data Masking

```rust,no_run
use alun_utils::Mask;

# fn demo() {
let _ = Mask::mobile("13812345678");          // → 138****5678
let _ = Mask::email("alice@mail.com");        // → a***@mail.com（本地部分 ≤2 字符时输出 *@域名）
let _ = Mask::id_card("320112199001011234");  // → 3201****1234
let _ = Mask::name("张三丰");                   // → 张**
let _ = Mask::bank_card("6222021234567890123");  // → 6222 **** 0123
let _ = Mask::user_id("user_123456");            // → us****56（保留前2后2）
let _ = Mask::password("secret");                // → ******（固定）
let _ = Mask::address("北京市朝阳区某路1号");      // 保留前 6 **字节** + ****
let _ = Mask::license_plate("浙A12345");          // → 浙****5
let _ = Mask::mask_by_type("mobile", "13812345678");  // 按类型名分派（见下）
# }
```

`Mask::mask_by_type(data_type, value)` 支持的类型名：`user_id` / `mobile` / `password` / `address` /
`bank_card` / `license_plate` / `email` / `id_card` / `name`，其他值走默认遮盖。

对整份 JSON 递归脱敏（模块函数，门面未再导出）：

```rust,no_run
use alun_utils::mask::mask_json_value;

# fn demo() {
let v = serde_json::json!({"mobile": "13812345678", "note": "contact a@b.com"});
let masked = mask_json_value(v, &["mobile"]);
let _ = masked;
# }
```

## ID Generation

```rust,no_run
use alun_utils::Sid;

# fn demo() {
let _ = Sid::short();   // 16 hex chars
let _ = Sid::tiny();    // 8 hex chars
let _ = Sid::tsid();    // 20 位：13 位毫秒时间戳 + 7 位随机
let _ = Sid::uuid();    // UUID v4（36 字符标准格式，含连字符）
let _ = Sid::uuid7();   // UUID v7（time-ordered，推荐作数据库主键）
# }
```

## Validation

```rust,no_run
use alun_utils::Valid;

# fn demo() {
let _ = Valid::is_email("a@b.com");
let _ = Valid::is_mobile("13812345678");     // 也接受固话 "010-12345678"
let _ = Valid::is_phone("+8613812345678");   // E.164
let _ = Valid::is_url("https://example.com");
let _ = Valid::is_username("tom_01");
let _ = Valid::is_ipv4("192.168.1.1");
let _ = Valid::is_strong_password("Abc@12345");
let _ = Valid::is_digits("123456");
let _ = Valid::len_between("hello", 2, 10);
let _ = Valid::is_uuid("550e8400-e29b-41d4-a716-446655440000");
let _ = Valid::is_id_card("11010519491231002X");  // 含校验位
let _ = Valid::is_date("2026-01-01");
let _ = Valid::is_json(r#"{"k":1}"#);
let _ = Valid::is_file_extension("a.png", &["png", "jpg"]);
# }
```

| 分组 | 方法 |
|------|------|
| 基础 | `is_email`、`is_mobile`、`is_phone`、`is_url`、`is_username`、`is_color`（`#RRGGBB`）、`is_digits`、`is_alphanumeric`、`len_between` |
| 密码 | `is_strong_password`（≥8 位且含大小写+数字+特殊字符） |
| 网络与编码 | `is_ipv4`、`is_base64` |
| 标识 | `is_uuid`（v1~v7）、`is_id_card`（含校验位） |
| 日期 | `is_date`（`YYYY-MM-DD`）、`is_datetime`（RFC 3339 或日期） |
| 文本 | `is_json`、`has_html`、`is_html_free` |
| 文件 | `is_file_extension(filename, allowed)` |
| validator 集成 | `format_validation_errors(&ValidationErrors)`（需 `alun-utils` 的 `validator-integration` feature） |

## Cryptography

```rust,no_run
use alun_utils::Crypto;

# fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
let _ = Crypto::sha256("data");
let _ = Crypto::hmac_sha256(b"key", "data");              // HMAC-SHA256（hex）
let hash = Crypto::hash_password("pass123")?;            // Argon2（内部自动加盐）
let _ok = Crypto::verify_password("pass123", &hash)?;    // 自动识别 Argon2/BCrypt
let key = Crypto::random_key();                          // 32 字节随机密钥
let _ = Crypto::random_token(32);                        // 随机 hex token
let _ = Crypto::base64_url_encode(b"data");              // URL 安全、无填充
let _ = Crypto::base64_url_decode("ZGF0YQ");

// AES-256-GCM：aes_encrypt(key, plaintext) -> Option<(ciphertext_b64, nonce_b64)>
let (ciphertext, nonce) = Crypto::aes_encrypt(&key, "secret").ok_or("加密失败")?;
let _plain = Crypto::aes_decrypt(&key, &ciphertext, &nonce).ok_or("解密失败")?;
# Ok(())
# }
```

## Data Export

```rust,no_run
use alun_utils::{Export, Import};

# fn demo() -> std::result::Result<(), Box<dyn std::error::Error>> {
# let records: Vec<std::collections::HashMap<String, String>> = vec![];
let csv = Export::to_csv(&["name", "age"], &records)?;
let json = Export::to_json(&records)?;                       // 任意 &[T: Serialize]
let csvb = Export::to_csv_bytes(&["name", "age"], &records)?; // Vec<u8>：BOM + CSV（仍不是 XLSX）
let back: Vec<std::collections::HashMap<String, String>> = Import::from_csv(&csv)?;
let items: Vec<serde_json::Value> = Import::from_json(&json)?;
# let _ = (csv, json, csvb, back, items); Ok(())
# }
```

### 真 XLSX（`alun-utils` 的 `xlsx` feature）

```rust,ignore
use alun_utils::{Export, Import, XlsxSheet};

let bytes = Export::to_xlsx_bytes(&["name", "age"], &records)?;   // 真正的 OOXML 工作簿
let bytes = Export::to_xlsx_typed(&["name", "age"], &items)?;     // 任意 &[T: Serialize]
let sheets = [XlsxSheet::new("用户", &["name"], &users), XlsxSheet::new("订单", &["no"], &orders)];
let bytes = Export::to_xlsx_workbook(&sheets)?;                   // 多 sheet

let names = Import::xlsx_sheet_names(&bytes)?;
let rows  = Import::from_xlsx(&bytes)?;                           // 第一个 sheet，首行作表头
let rows  = Import::from_xlsx_sheet(&bytes, "订单")?;
let items: Vec<Item> = Import::from_xlsx_typed(&bytes)?;
```

- 写侧 `rust_xlsxwriter`、读侧 `calamine`；**不开 `xlsx` feature 时这些 API 不存在**（不会退化成 CSV）。
- 数值单元格只在文本能"原样往返"时生成：`42` → 数值，`"00123"` → 文本（保住前导零）。
- 旧 `Export::to_xlsx`（BOM+CSV）与空壳枚举 `ExportFormat` 已删除，勿再使用。

## Serial Number Generation (`alun-utils::serial`)

基于规则配置的分布式单号引擎，将"格式模板 + 循环周期 + 计数策略"抽象为可配置的 `SerialRule`。

```rust,no_run
use alun_utils::{CyclePeriod, IncrementStrategy, MemorySerialBackend, SerialGenerator, SerialRule};

# async fn demo() -> std::result::Result<(), alun_utils::SerialError> {
// 创建内存后端
let backend = MemorySerialBackend::new();
// 记录上限（默认 10000 条）
let backend = backend.with_max_records(1000);

// 注册规则（结构体字面量）
backend.register_rule(SerialRule {
    key: "order".into(),
    format: "ORD{YYYY}{MM}{DD}{SEQ:8}".into(),
    cycle: CyclePeriod::Daily,
    initial_value: 1,
    step: IncrementStrategy::Sequential,
    is_enabled: true,
}).await?;

// 或 builder 写法：SerialRule::new(key, format).with_cycle(..).with_initial_value(..).with_step(..)
backend.register_rule(
    SerialRule::new("contract", "CT{YY}{MM}{SEQ:6}").with_cycle(CyclePeriod::Monthly),
).await?;

// 生成单号 → "ORD2026061000000001"
let no = backend.generate("order").await?;

// 批量生成
let nos = backend.batch_generate("order", 5).await?;

// 预览（不消耗计数器）
let next = backend.peek("order").await?;

// 运行时管理规则

backend.remove_rule("order").await?;     // 删除规则
backend.enable_rule("order").await?;     // 启用规则
backend.disable_rule("order").await?;    // 禁用规则（generate() 返回 RuleDisabled 错误）
backend.list_rules().await?;             // 列出所有规则
backend.query_records("order", 1, 20).await?;  // 查询生成记录（分页）→ (Vec<SerialRecord>, u64)
# let _ = (no, nos, next);
# Ok(())
# }
```

> 时间基准是**本地时间**：`CyclePeriod::current_value()` 与 `FormatEngine::render(..)` 都取
> `Local::now()`（`render(seq, Some(ts))` 可显式传入 Unix 秒时间戳）。

### 核心类型

| 类型 | 说明 |
|------|------|
| `SerialRule` | 单号规则定义：`key`（唯一标识）、`format`（格式模板）、`cycle`（循环周期）、`initial_value`（初始值）、`step`（增量策略）、`is_enabled`（启用状态）；另有 `new` / `with_cycle` / `with_initial_value` / `with_step` builder |
| `CyclePeriod` | 循环周期：`NoCycle`（永不重置，默认）、`Daily`（按天 YYYYMMDD）、`Monthly`（按月 YYYYMM）、`Yearly`（按年 YYYY）；`current_value()` 返回当前周期串 |
| `IncrementStrategy` | 增量策略：`Sequential`（顺序递增，默认）、`Random { max }`（在 [1, max] 范围内随机跳动） |
| `FormatEngine` | 预编译格式串：`compile(format)` → `Result<Self, SerialError>`、`render(seq, ts: Option<i64>) -> String`；未知占位符 / 位数非正 → `FormatError` |
| `MemorySerialBackend` | 内存后端（进程内，默认最多 10000 条记录，`with_max_records` 可调） |
| `SerialRecord` | 生成记录：`rule_key`、`serial_no`、`counter`、`cycle_value`、`created_at`（RFC3339 字符串） |
| `SerialError` | 错误类型：`RuleNotFound`、`RuleDisabled`、`FormatError`、`CounterOverflow`、`StorageError`（均带 `String` 载荷） |

内置只有 `MemorySerialBackend`；Redis / PostgreSQL 需要业务方自行实现 `SerialGenerator` 后注入
`SerialPlugin::new(cfg.plugins.serial.clone(), backend)`（`[plugins.serial] backend` 配置项本身不参与后端选择）。

### SerialGenerator Trait（自定义后端接口）

遵循 `TaskStorage` 设计模式——业务方实现 trait，通过 `Arc<dyn SerialGenerator>` 注入 `SerialPlugin`：

```rust,ignore
#[async_trait]
pub trait SerialGenerator: Send + Sync {
    async fn generate(&self, rule_key: &str) -> Result<String, SerialError>;
    async fn batch_generate(&self, rule_key: &str, count: u32) -> Result<Vec<String>, SerialError>;
    async fn peek(&self, rule_key: &str) -> Result<String, SerialError>;
    async fn register_rule(&self, rule: SerialRule) -> Result<(), SerialError>;
    async fn remove_rule(&self, rule_key: &str) -> Result<(), SerialError>;
    async fn enable_rule(&self, rule_key: &str) -> Result<(), SerialError>;
    async fn disable_rule(&self, rule_key: &str) -> Result<(), SerialError>;
    async fn query_records(&self, rule_key: &str, page: u64, page_size: u64) -> Result<(Vec<SerialRecord>, u64), SerialError>;
    async fn list_rules(&self) -> Result<Vec<SerialRule>, SerialError>;
}
```

### 格式模板占位符

| 占位符 | 说明 | 示例 |
|--------|------|------|
| `{YYYY}` | 4 位年份 | 2026 |
| `{YY}` | 2 位年份 | 26 |
| `{MM}` | 2 位月份 | 06 |
| `{DD}` | 2 位日期 | 11 |
| `{SEQ:n}` | n 位补零序号 | `{SEQ:8}` → 00000001 |
| `{RAND:n}` | n 位随机数 | `{RAND:4}` → 5821 |
| `{TS}` | Unix 时间戳（秒） | 1716537600 |
| `{TSMS}` | Unix 时间戳（毫秒） | 1716537600123 |

## Web & UA Helpers

```rust,no_run
use alun_utils::web::is_private_ip;
use alun_utils::{extract_client_ip, parse_user_agent, UaInfo, WebExt};
use axum::http::HeaderMap;
use std::net::SocketAddr;

# fn demo(headers: &HeaderMap, peer: &SocketAddr) {
// 可信客户端 IP：X-Real-IP → X-Forwarded-For **末段** → 直连对端 IP → "0.0.0.0"
let _ip = extract_client_ip(headers, peer);
let _ = is_private_ip("192.168.1.1");          // → true（10/8、172.16/12、192.168/16、环回、链路本地、ULA…）

let info: UaInfo = parse_user_agent(
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 \
     (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
);
let _ = (info.device_type, info.browser_type, info.os_type);  // → MOBILE / Safari / iOS

let _ = WebExt::domain("https://a.example.com/x/y");        // → Some("a.example.com")
let _ = WebExt::path("https://a.example.com/x/y");          // → Some("/x/y")
let _ = WebExt::build_query(&[("q", "a b")]);                // → "?q=a%20b"
let _ = WebExt::is_private_ip("10.0.0.1");                   // → true（委托给模块函数）
# }
```

> 限流 / 风控取 IP 请用 `extract_client_ip`（只信任 `X-Real-IP` 与 `X-Forwarded-For` 末段）。
> `WebExt::real_ip(..)` 取的是 `X-Forwarded-For` **首段**，该值客户端可伪造，仅遗留场景使用。

## XSS Sanitization（经 `alun` 的 `xss` feature 转发）

> 能力实现在 **`alun-utils::xss`**，但已由 `alun` 的 `xss` feature 转发（`xss = ["alun-utils/xss"]`），
> 模块经 `alun::xss` / `alun::prelude::xss` 暴露：
> `alun = { version = "0.3", features = ["xss"] }`（默认的 `full` **不含** `xss`，必须显式开）。
> 直接依赖 `alun-utils = { version = "0.3", features = ["xss"] }` 等价——同一个 feature，会合并。

```rust,no_run
use alun::prelude::*;   // `xss` feature 下 prelude 里就有 `xss` 模块

# fn demo() {
let _safe = xss::sanitize_html("<script>alert(1)</script><p>Hello</p>");  // → <p>Hello</p>
let _strict = xss::sanitize_html_strict("<p>Hello</p>");                  // → Hello
let _malicious = xss::has_potential_xss("<script>alert(1)</script>");     // → true
# }
```

也可写全路径：`alun::xss::sanitize_html(..)`，或直接依赖底层 crate 时
`alun_utils::xss::sanitize_html(..)`（三者是同一个函数）。

## Formatting Helpers

```rust,no_run
use alun::prelude::*;
use alun_utils::{format_file_size, generate_invite_code, generate_random_alphanum, generate_random_digits, parse_json_value};

# fn demo() {
let _ = format_file_size(1_500_000);        // → "1.43 MB"
let _ = parse_json_value(r#"{"k":1}"#);
let _ = generate_invite_code();             // 12-char invite code
let _ = generate_random_digits(6);          // 6 digits (no 0)
let _ = generate_random_alphanum(8);        // 8 chars (charset 去掉 0/O/I/l 等易混淆字符)
# }
```

## Global Resource Access

```rust,no_run
use alun::prelude::*;
use alun::{download_path, try_download_path, try_upload_path, upload_path, try_template};

# fn demo() {
// Primary accessors (panics if not initialized)
let _ = db();                       // &Db
let _ = cache();                    // &SharedCache
let _ = cfg();                      // &AppConfig (static config)
let _ = config();                   // &Arc<ConfigManager> (dynamic config)

// Upload/download paths
let _ = upload_path();              // &str，默认 "uploads"
let _ = download_path();            // &str，默认 "downloads"

// Safe accessors (return Option)
let _ = try_db();                   // Option<&Db>
let _ = try_cache();                // Option<&SharedCache>
let _ = try_config();               // Option<&Arc<ConfigManager>>
let _ = try_template();             // Option<&TemplateEngine>
let _ = try_upload_path();          // Option<&str>
let _ = try_download_path();        // Option<&str>
# }
```

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [导出导入（CSV / JSON / XLSX）](export-and-import.md) | [返回总目录](../SKILL.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

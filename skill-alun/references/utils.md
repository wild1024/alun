# Utilities (`alun-utils`)

200+ utility functions across these modules.

## String Manipulation

```rust
use alun_utils::StrExt;
"helloWorld".to_snake();   // → hello_world
"hello_world".to_camel();  // → helloWorld
"".is_blank();              // → true

sanitize_filename("file<name>.txt");  // → file_name.txt
clean_email("  User@Mail.COM  ");    // → user@mail.com
clean_string_param("  hello  ");     // → hello
clean_password("  pass  ");          // → pass
```

## Input Cleaners

```rust
let (email, pwd, nick) = InputCleaner::clean_register_input(" A@B.com ", " 123 ", " Tom ");
let (email, pwd) = InputCleaner::clean_login_input(" A@B.com ", " 123 ");
```

## Date Utilities

```rust
let now = Date::now();
Date::fmt(&now, "%Y-%m-%d %H:%M:%S");
Date::relative(now.timestamp());  // → "3分钟前"
Date::begin_of_day(&now);
Date::from_timestamp(1700000000);
```

## Data Masking

```rust
Mask::mobile("13812345678");          // → 138****5678
Mask::email("a@b.com");              // → a***@b.com
Mask::id_card("320112199001011234"); // → 3201****1234
Mask::name("张三丰");                  // → 张**
```

## ID Generation

```rust
Sid::short();   // 16 hex chars
Sid::tiny();    // 8 hex chars
Sid::tsid();    // Timestamp + random
Sid::uuid();    // UUID v4（36 字符标准格式，含连字符）
Sid::uuid7();   // UUID v7（36 字符标准格式，含连字符，time-ordered, recommended for DB primary keys）
```

## Validation

```rust
Valid::is_email("a@b.com");
Valid::is_mobile("13812345678");
Valid::is_url("https://example.com");
Valid::is_ipv4("192.168.1.1");
Valid::is_strong_password("Abc@12345");
Valid::len_between("hello", 2, 10);
Valid::is_digits("123456");
```

## Cryptography

```rust
Crypto::sha256("data");
Crypto::hash_password("pass123");           // Argon2
Crypto::verify_password("pass123", &hash)?;  // Auto-detect Argon2/BCrypt
Crypto::random_key();                       // 32 random bytes
Crypto::random_token(32);                   // Random hex token
let encrypted = Crypto::aes_encrypt("secret", &key_hex)?;
let decrypted = Crypto::aes_decrypt(&encrypted, &key_hex)?;
```

## Data Export

```rust
let csv = Export::to_csv(&["name", "age"], &records)?;
let json = Export::to_json(&records)?;
```

## Serial Number Generation (`alun-utils::serial`)

基于规则配置的分布式单号引擎，将"格式模板 + 循环周期 + 计数策略"抽象为可配置的 `SerialRule`。

```rust
use alun_utils::{SerialRule, SerialGenerator, MemorySerialBackend, CyclePeriod, IncrementStrategy};

// 创建内存后端
let gen = MemorySerialBackend::new();

// 注册规则
gen.register_rule(SerialRule {
    key: "order".into(),
    format: "ORD{YYYY}{MM}{DD}{SEQ:8}".into(),
    cycle: CyclePeriod::Daily,
    initial_value: 1,
    step: IncrementStrategy::Sequential,
    is_enabled: true,
}).await?;

// 生成单号 → "ORD2026061000000001"
let no = gen.generate("order").await?;

// 批量生成
let nos = gen.batch_generate("order", 5).await?;

// 预览（不消耗计数器）
let next = gen.peek("order").await?;

// 运行时管理规则
gen.register_rule(rule).await?;      // 注册/更新规则
gen.remove_rule("order").await?;     // 删除规则
gen.enable_rule("order").await?;     // 启用规则
gen.disable_rule("order").await?;    // 禁用规则（generate() 返回 RuleDisabled 错误）
gen.list_rules().await?;             // 列出所有规则
gen.query_records("order", 1, 20).await?;  // 查询生成记录（分页）
```

### 核心类型

| 类型 | 说明 |
|------|------|
| `SerialRule` | 单号规则定义：`key`（唯一标识）、`format`（格式模板）、`cycle`（循环周期）、`initial_value`（初始值）、`step`（增量策略）、`is_enabled`（启用状态） |
| `CyclePeriod` | 循环周期：`NoCycle`（永不重置）、`Daily`（按天 YYYYMMDD）、`Monthly`（按月 YYYYMM）、`Yearly`（按年 YYYY） |
| `IncrementStrategy` | 增量策略：`Sequential`（顺序递增）、`Random { max }`（在 [1, max] 范围内随机跳动） |
| `MemorySerialBackend` | 内存后端（进程内，适合单实例），可通过 `SerialPlugin` 或独立使用 |
| `SerialRecord` | 生成记录：`rule_key`、`serial_no`、`counter`、`cycle_value`、`created_at` |
| `SerialError` | 错误类型：`RuleNotFound`、`RuleDisabled`、`FormatError`、`CounterOverflow`、`StorageError` |

### SerialGenerator Trait（自定义后端接口）

遵循 `TaskStorage` 设计模式——业务方实现 trait，通过 `Arc<dyn SerialGenerator>` 注入 `SerialPlugin`：

```rust
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

## XSS Sanitization (requires `features = ["xss"]`)

```rust
let safe = xss::sanitize_html("<script>alert(1)</script><p>Hello</p>");  // → <p>Hello</p>
let strict = xss::sanitize_html_strict("<p>Hello</p>");                  // → Hello
let malicious = xss::has_potential_xss("<script>alert(1)</script>");    // → true
```

## Formatting Helpers

```rust
format_file_size(1_500_000);  // → "1.43 MB"
parse_json_value(r#"{"k":1}"#);
generate_invite_code();       // 12-char invite code
generate_random_digits(6);    // 6 digits (no 0)
generate_random_alphanum(8);  // 8 chars (no confusing chars like 0/O/I/l)
```

## Global Resource Access

```rust
// Primary accessors (panics if not initialized)
db()              // &Db
cache()           // &SharedCache
cfg()             // &AppConfig (reference to static config)
config()          // &ConfigManager (dynamic config)

// Upload/download paths
upload_path()     // Returns path string (default: "uploads")
download_path()   // Returns path string (default: "downloads")

// Safe accessors (return Option)
try_db()          // Option<&Db>
try_cache()       // Option<&SharedCache>
try_config()      // Option<&Arc<ConfigManager>>
try_template()    // Option<&TemplateEngine>
try_upload_path()    // Option<&str>
try_download_path()  // Option<&str>
```
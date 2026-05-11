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
Sid::uuid();    // UUID v4
Sid::uuid7();   // UUID v7 (time-ordered, recommended for DB primary keys)
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
Crypto::verify_password("pass123", &hash)?;
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
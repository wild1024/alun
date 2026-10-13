# alun-utils

Alun 工具库：字符串处理、日期、脱敏、ID 生成、格式校验、加密、导出导入、HTML 净化、单号。

> 门面 `alun` 会把常用部分（`Crypto` / `Mask` / `Sid` / `Valid` / `Export` / `Import` / `xss`）
> 放进 `alun::prelude::*`，业务项目通常**不需要**直接依赖本 crate。

## 快速上手

```rust,no_run
use alun_utils::{Crypto, Mask, Sid, Valid};

assert_eq!(Sid::uuid().len(), 36);
assert!(Valid::is_email("a@b.com"));
assert_eq!(Mask::mobile("13800138000"), "138****8000");
let hash = Crypto::hash_password("s3cret")?;
assert!(Crypto::verify_password("s3cret", &hash)?);
```

下面所有 API 都在 crate 根再导出（`alun_utils::Sid`），也可以按模块访问（`alun_utils::sid::Sid`）。

## 主要 API

### `Sid` —— 各类 ID

`Sid::uuid()`（UUID v4）、`Sid::uuid7()`（时间有序）、`Sid::short()`（16 位十六进制）、
`Sid::tiny()`（8 位）、`Sid::tsid()`（20 位毫秒序 ID）。

### `Mask` —— 敏感数据脱敏

`Mask::mobile()` / `email()` / `id_card()` / `bank_card()` / `name()` / `user_id()` /
`password()` / `address()` / `license_plate()`，以及 `Mask::mask_by_type(data_type, value)`
按类型批量脱敏。另有自由函数 `mask_json_value(value, sensitive_fields)` 处理整个 JSON 对象。

> 手机脱敏是 `Mask::mobile()`（没有 `Mask::phone()`）；**手机号校验**是 `Valid::is_mobile()`。

### `Crypto` —— 哈希与对称加密

`sha256`、`hmac_sha256`、`aes_encrypt` / `aes_decrypt`（AES-256-GCM，32 字节密钥，
返回 `Option<(密文 b64, nonce b64)>`）、`hash_password` / `verify_password`（Argon2，盐内部生成，
`verify_password` 也认 BCrypt 哈希）、`base64_url_encode` / `base64_url_decode`、
`random_key()`、`random_token(len)`。

### `Valid` —— 格式校验

`is_email` / `is_mobile` / `is_phone` / `is_url` / `is_username` / `is_color` / `is_digits` /
`is_alphanumeric` / `is_strong_password` / `is_ipv4` / `is_base64` / `is_uuid` / `is_id_card` /
`is_date` / `is_datetime` / `is_json` / `has_html` / `is_html_free` / `len_between` /
`is_file_extension`；开 `validator-integration` feature 另有
`Valid::format_validation_errors(errors)`。

### `Date` —— 日期

`now()` / `now_local()` / `fmt(dt, fmt)` / `from_timestamp(ts)` / `relative(ts)` /
`begin_of_day(dt)` / `end_of_day(dt)`。

### `StrExt` —— 字符串扩展

`is_blank()` / `has_text()` / `to_camel()` / `to_snake()` / `truncate(max)` / `random(len)`。
自由函数：`sanitize_filename`、`parse_json_value`、`format_file_size`、`clean_string_param`、
`clean_email`、`clean_password`、`InputCleaner`、`generate_invite_code`、`generate_random_digits`、
`generate_random_alphanum`。

### Web / UA 辅助

`WebExt` 与 `extract_client_ip(..)`（取客户端 IP）、`parse_user_agent(..) -> UaInfo`。

### 单号（`serial`，根再导出）

- `SerialRule` —— `new(key, format)`、`with_cycle(CyclePeriod)`、`with_initial_value(n)`、`with_step(IncrementStrategy)`
- `CyclePeriod` —— `NoCycle`（默认）/ `Daily` / `Monthly` / `Yearly`
- `IncrementStrategy` —— `Sequential`（默认）或 `Random { max }`
- `trait SerialGenerator` —— `generate` / `batch_generate` / `peek` / `register_rule` / `remove_rule` /
  `enable_rule` / `disable_rule` / `query_records` / `list_rules`
- `MemorySerialBackend` —— 进程内后端，`new()` / `with_max_records(n)`
- `FormatEngine` —— `compile(format)` / `render(seq, ts)`；另有 `SerialRecord`、`SerialError`

格式令牌：`{YYYY}` `{YY}` `{MM}` `{DD}` `{SEQ:n}` `{RAND:n}` `{TS}` `{TSMS}`。

```rust,no_run
use alun_utils::{CyclePeriod, serial::{MemorySerialBackend, SerialGenerator, SerialRule}};

let backend = MemorySerialBackend::new();
backend.register_rule(
    SerialRule::new("order", "ORD{YYYY}{MM}{DD}{SEQ:8}").with_cycle(CyclePeriod::Daily),
).await?;
let order_no = backend.generate("order").await?;    // 形如 ORD2026100500000001
```

## 导出与导入（CSV / JSON / 真 XLSX）

| 能力 | API | feature |
| --- | --- | --- |
| CSV（带 BOM） | `Export::to_csv(columns, rows)` / `to_csv_bytes(..)` | 默认 |
| JSON | `Export::to_json(&[T])` | 默认 |
| 读 CSV / JSON | `Import::from_csv(str)` / `Import::from_json::<T>(str)` | 默认 |
| **写 XLSX** | `Export::to_xlsx_bytes(..)` / `to_xlsx_typed(..)` / `to_xlsx_workbook(&[XlsxSheet])`（多 sheet） | `xlsx` |
| **读 XLSX** | `Import::xlsx_sheet_names(bytes)` / `from_xlsx(bytes)` / `from_xlsx_sheet(bytes, sheet)` / `from_xlsx_typed::<T>(bytes)` | `xlsx` |

```rust,no_run
use std::collections::HashMap;
use alun_utils::export::{Export, Import, XlsxSheet};

let rows: Vec<HashMap<&str, String>> = vec![HashMap::from([("code", "A-001")])];
let bytes = Export::to_xlsx_bytes(&["code"], &rows)?;      // 真 OOXML（PK\x03\x04 开头）
let sheets = vec![XlsxSheet::new("明细", &["code"], &rows)];
let workbook = Export::to_xlsx_workbook(&sheets)?;         // 多 sheet 工作簿
let names = Import::xlsx_sheet_names(&workbook)?;
```

> ⚠️ 两条硬约束：
> 1. **不开 `xlsx` feature 时这些方法根本不存在** —— 宁可编译失败，也不会给你
>    「扩展名是 xlsx、内容其实是 CSV」的错产物；
> 2. **类型原样往返**：单元格类型由「字符串是不是纯数字」推断。单号 `00123` 想保持文本就存字符串；
>    要参与求和的 `19999` 存数字串即可，Excel 里会自动落成数字。

## HTML 净化（feature `xss`）

```rust,ignore
let safe = alun_utils::xss::sanitize_html("<script>alert(1)</script><b>hi</b>");
```

门面侧对应 `alun` 的 `xss` feature，`use alun::prelude::*;` 后可直接 `xss::sanitize_html(..)`。

## Feature

| Feature | 启用内容 |
|---------|---------|
| `validator-integration` | `Valid::format_validation_errors` 与 `validator` 互操作 |
| `xss` | `xss` 模块：`sanitize_html` / `sanitize_html_strict` / `has_potential_xss`（基于 ammonia） |
| `xlsx` | 真 XLSX 读写（`rust_xlsxwriter` 写 / `calamine` 读） |

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)
- 速查参考：`skill-alun/references/utils.md`、`skill-alun/references/export-and-import.md`

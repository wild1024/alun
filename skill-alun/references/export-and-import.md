# 导出与导入：CSV / JSON / 真 XLSX

> 能力来自 `alun-utils`，门面 `alun` 已把它放进 prelude：`alun::Export` / `alun::Import` / `alun::XlsxSheet`。
> 门面 feature：`xlsx`（真 XLSX，`full` 预设已含）。

## 先分清三种产物

| 你要的产物 | 用哪个 API | feature |
|---|---|---|
| CSV（带 BOM） | `Export::to_csv(cols, &rows)` / `to_csv_bytes(..)` | 默认 |
| JSON | `Export::to_json(&items)` | 默认 |
| **真 .xlsx**（Excel 直接打开） | `Export::to_xlsx_bytes(..)` / `to_xlsx_typed(..)` / `to_xlsx_workbook(..)` | `xlsx` |

导入方向：`Import::from_csv(str)`、`Import::from_json::<T>(str)`，以及 `xlsx` 下的
`xlsx_sheet_names(bytes)` / `from_xlsx(bytes)` / `from_xlsx_sheet(bytes, sheet)` / `from_xlsx_typed::<T>(bytes)`。

> ⚠️ **不开 `xlsx` feature 时这些 API 根本不存在** —— 框架选择「编译失败」而不是「悄悄返回一个
> CSV 却叫 xlsx」。如果你线上出现 Excel 提示「文件格式与扩展名不一致」，先查 feature。

## 写：单个 sheet

```rust,no_run
use alun::prelude::*;
use std::collections::HashMap;

let rows: Vec<HashMap<&str, String>> = vec![
    HashMap::from([("code", "A-001"), ("qty", "3")]),
    HashMap::from([("code", "A-002"), ("qty", "10")]),
];
let bytes: Vec<u8> = Export::to_xlsx_bytes(&["code", "qty"], &rows)?;
```

## 写：多 sheet 工作簿

```rust,no_run
use alun::prelude::*;
use std::collections::HashMap;

let detail = vec![HashMap::from([("code", "A-001")])];
let summary = vec![HashMap::from([("total", "13")])];

let sheets = vec![
    XlsxSheet::new("明细", &["code"], &detail),
    XlsxSheet::new("汇总", &["total"], &summary),
];
let workbook = Export::to_xlsx_workbook(&sheets)?;   // Vec<u8>，合法 OOXML（PK\x03\x04）
```

`XlsxSheet::new(name, headers, &rows)`：自动列宽自适应 + 冻结首行，`name` 建议 ≤31 字符。

## 直接作为 HTTP 响应

```rust,no_run
use alun::prelude::*;

#[alun::get("/api/export/orders")]
async fn export() -> impl axum::response::IntoResponse {
    let rows: Vec<std::collections::HashMap<&str, String>> = vec![];
    (
        [("content-type", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")],
        Export::to_xlsx_bytes(&["code"], &rows),
    )
}
```

> 注意：`Res<T>` 的 JSON 包装不适合二进制导出 —— 二进制响应直接返回 `impl IntoResponse`
> （或 `axum::response::File`），不要塞进 `Res::ok(bytes)`。

## 读

```rust,ignore
use alun::prelude::*;

let names = Import::xlsx_sheet_names(&bytes)?;         // ["明细", "汇总"]
let rows  = Import::from_xlsx_sheet(&bytes, &names[0])?;   // Vec<HashMap<String, String>>
let typed = Import::from_xlsx_typed::<MyRow>(&bytes)?;     // 需要 Dao 匹配/反序列化
```

## 类型规则（最重要的一条）

单元格类型由**字符串内容**推断：`"100"` 会落成数字、`"00123"` 保留为文本。规则：

- 想让 `00123` 在 Excel 里显示成文本（单号、邮编、区号）→ 值就存字符串 `"00123"`；
- 想让 `19999` 参与求和 → 存 `"19999"`（纯数字串）即可，会自动落成数字；
- 布尔用 `Cell::Boolean`，不要塞 `"true"` 字符串。

一句话：**「看得出是编号的」就用字符串存，看得出是数值的就用数字串**。这是「原样往返」策略，
避免 Excel 把单号改成 1.23E+4。

## CSV 的配套约束

- `Export::to_csv` / `to_csv_bytes` 带 **UTF-8 BOM**，Excel 直接双击打开不乱码；
- 走 HTTP 时响应头配 `text/csv; charset=utf-8`，文件名后缀 `.csv`；
- 前端 `blob = new Blob([...], { type: 'text/csv;charset=utf-8' })`，别用 `.xlsx` 后缀。

## 常见坑

| 现象 | 原因 / 处理 |
|---|---|
| `Export::to_xlsx_bytes` 编译不过 | 没开 `xlsx` feature（门面加 `features = ["xlsx"]`） |
| Excel 提示「格式与扩展名不一致」 | 拿到的是 CSV（多半是 feature 没开走了旧路径），或 `file x.xlsx` 不是 `PK\x03\x04` 开头 |
| 单号变科学计数法 | 值被当成数字了 —— 确认存的是字符串且带前导零 |
| `XlsxSheet::new(name, headers, &rows)` 借用报错 | `&rows` 是临时值：先 `let rows = vec![..];` 再传 `&rows` |
| 空 sheet 读出来 0 行 | calamine 会裁剪全空列区间；确保该列至少有一个值 |

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [可观测 / Prometheus / 安全指标](observability.md) | [工具函数（ID / 脱敏 / 校验 / 加密 / 单号）](utils.md) → |

<sub>本分册属于「按能力速查（12 份 modules）」，共 12 篇。首篇 / 末篇可点「返回总目录」。</sub>

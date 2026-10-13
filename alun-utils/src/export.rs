//! 导出导入工具：CSV / JSON / XLSX
//!
//! 三件事先说清楚，避免历史上的「名实不符」重演：
//!
//! 1. `to_csv*` 产出 **CSV**（`to_csv_bytes` 额外加 UTF-8 BOM，便于 Excel 双击打开）。
//!    它不是 XLSX，下载响应请把 `Content-Type` 设为 `text/csv; charset=utf-8`、
//!    扩展名用 `.csv`。
//! 2. 真正的 `.xlsx`（OOXML 工作簿）由 **`xlsx` feature** 提供：
//!    [`Export::to_xlsx_bytes`] / [`Export::to_xlsx_typed`] / [`Export::to_xlsx_workbook`]
//!    写入，[`Import::from_xlsx`] 等解析。**不开 feature 时这些 API 不存在**
//!    ——而不是「存在但返回 CSV」。旧的 `to_xlsx`（实为 BOM+CSV）已**删除**。
//! 3. 因此本模块**没有** `ExportFormat` 枚举：它从未被任何方法消费，只是个
//!    空壳类型，属于诱导误用的死 API。

use alun_core::{Error, Result};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;

/// 导出工具
pub struct Export;

impl Export {
    /// 将 KV 数据集导出为 CSV 字符串
    ///
    /// `columns` 指定列顺序，`rows` 为 key→value 映射的集合。
    ///
    /// ```ignore
    /// let csv = Export::to_csv(&["id", "name"], &records)?;
    /// ```
    pub fn to_csv(columns: &[&str], rows: &[HashMap<String, String>]) -> Result<String> {
        let mut w = csv::Writer::from_writer(Vec::new());
        w.write_record(columns)
            .map_err(|e| Error::Msg(format!("CSV写入失败: {}", e)))?;
        for row in rows {
            let vals: Vec<&str> = columns
                .iter()
                .map(|c| row.get(*c).map(|s| s.as_str()).unwrap_or(""))
                .collect();
            w.write_record(&vals)
                .map_err(|e| Error::Msg(format!("CSV写入失败: {}", e)))?;
        }
        let data = w
            .into_inner()
            .map_err(|e| Error::Msg(format!("CSV输出失败: {}", e)))?;
        String::from_utf8(data).map_err(|e| Error::Msg(format!("UTF8转换失败: {}", e)))
    }

    /// 导出为 **带 UTF-8 BOM 的 CSV 字节流**（Excel 双击可直接打开）。
    ///
    /// 语义明确：产物是 CSV，不是 XLSX。需要真正的工作簿请用
    /// [`Export::to_xlsx_bytes`]（需 `xlsx` feature）。
    ///
    /// ```ignore
    /// let bytes = Export::to_csv_bytes(&["id", "name"], &records)?;
    /// std::fs::write("out.csv", bytes)?;
    /// ```
    pub fn to_csv_bytes(columns: &[&str], rows: &[HashMap<String, String>]) -> Result<Vec<u8>> {
        let csv_content = Self::to_csv(columns, rows)?;
        let mut data = vec![0xEFu8, 0xBB, 0xBF];
        data.extend(csv_content.as_bytes());
        Ok(data)
    }

    /// 将结构化数据导出为 JSON
    pub fn to_json<T: Serialize>(items: &[T]) -> Result<String> {
        serde_json::to_string_pretty(items)
            .map_err(|e| Error::Msg(format!("JSON序列化失败: {}", e)))
    }
}

/// 导入工具
pub struct Import;

impl Import {
    /// 从 CSV 字符串解析为 `Vec<HashMap<String, String>>`
    pub fn from_csv(csv_str: &str) -> Result<Vec<HashMap<String, String>>> {
        let mut reader = csv::Reader::from_reader(csv_str.as_bytes());
        let headers = reader
            .headers()
            .map_err(|e| Error::Msg(format!("CSV头解析失败: {}", e)))?
            .clone();
        let mut rows = Vec::new();
        for result in reader.records() {
            let record = result.map_err(|e| Error::Msg(format!("CSV行解析失败: {}", e)))?;
            let mut map = HashMap::new();
            for (i, val) in record.iter().enumerate() {
                if let Some(h) = headers.get(i) {
                    map.insert(h.to_string(), val.to_string());
                }
            }
            rows.push(map);
        }
        Ok(rows)
    }

    /// 从 JSON 反序列化
    pub fn from_json<T: DeserializeOwned>(json_str: &str) -> Result<Vec<T>> {
        serde_json::from_str(json_str).map_err(|e| Error::Msg(format!("JSON解析失败: {}", e)))
    }
}

// ══════════════════════════════════════════════════════════════════
// 真 XLSX（OOXML）：feature `xlsx`
//
// 写：`rust_xlsxwriter`（纯 Rust，无系统依赖）
// 读：`calamine`（xlsx / xlsb / xls / ods）
// ══════════════════════════════════════════════════════════════════

#[cfg(feature = "xlsx")]
mod xlsx_impl {
    use super::{Error, Export, HashMap, Import, Result, Serialize};
    use calamine::{Data, Reader, Xlsx, open_workbook_from_rs};
    use rust_xlsxwriter::{Color, Format, Workbook, Worksheet};
    use serde::de::DeserializeOwned;
    use std::io::Cursor;

    // ── 单元格类型推断 ────────────────────────────────────────
    //
    // 只把**能原样往返**的文本转成数字：`"00123"` 格式化回字符串是 `"123"`，
    // 与原文不等 → 保持文本。这样订单号 / 手机号 / 身份证之类的零填充或
    // 超长数字不会被 Excel 悄悄改写成科学计数法或丢前导零。

    /// 文本 → 单元格类型。返回 `Ok(Cell::*)` 表示可安全数值化。
    fn classify(v: &str) -> Cell<'_> {
        if let Ok(i) = v.parse::<i64>() {
            // 排除前导零与 `+1` 这类非规范写法
            if i.to_string() == v {
                return Cell::Int(i);
            }
        }
        if let Ok(f) = v.parse::<f64>() {
            if f.is_finite() && f.to_string() == v {
                return Cell::Float(f);
            }
        }
        match v {
            "true" => Cell::Bool(true),
            "false" => Cell::Bool(false),
            _ => Cell::Text(v),
        }
    }

    enum Cell<'a> {
        Int(i64),
        Float(f64),
        Bool(bool),
        Text(&'a str),
    }

    fn write_cell(ws: &mut Worksheet, row: u32, col: u16, v: &str) -> Result<()> {
        if v.is_empty() {
            // 显式留空，让列宽/边框仍然作用于该单元格
            ws.write_blank(row, col, &Format::new())
                .map_err(map_err)?;
            return Ok(());
        }
        match classify(v) {
            Cell::Int(i) => ws.write_number(row, col, i as f64).map_err(map_err)?,
            Cell::Float(f) => ws.write_number(row, col, f).map_err(map_err)?,
            Cell::Bool(b) => ws.write_boolean(row, col, b).map_err(map_err)?,
            Cell::Text(s) => ws.write_string(row, col, s).map_err(map_err)?,
        };
        Ok(())
    }

    // ── 写 ───────────────────────────────────────────────────

    /// 一个工作表的内容：`name` 为 sheet 名，`columns` 定列顺序，
    /// `rows` 为 key→value 映射。
    ///
    /// ```ignore
    /// let sheets = [XlsxSheet::new("用户", &["id", "name"], &users)];
    /// let bytes = Export::to_xlsx_workbook(&sheets)?;
    /// ```
    pub struct XlsxSheet<'a> {
        /// 工作表名（≤31 字符，且不含 `\ / ? * [ ] :`）
        pub name: &'a str,
        /// 列顺序；不在其中的字段不会导出
        pub columns: &'a [&'a str],
        /// 数据行
        pub rows: &'a [HashMap<String, String>],
    }

    impl<'a> XlsxSheet<'a> {
        pub fn new(name: &'a str, columns: &'a [&'a str], rows: &'a [HashMap<String, String>]) -> Self {
            Self {
                name,
                columns,
                rows,
            }
        }
    }

    impl Export {
        /// 导出为**真正的 `.xlsx`（OOXML）工作簿**字节流。
        ///
        /// 首行表头加粗 + 冻结窗格，列宽按内容自适应。
        /// 产物可被 Excel / LibreOffice / `calamine` / `openpyxl` 正常解析。
        ///
        /// ```ignore
        /// let bytes = Export::to_xlsx_bytes(&["id", "name"], &records)?;
        /// // Content-Type: application/vnd.openxmlformats-officedocument
        /// //              .spreadsheetml.sheet
        /// ```
        pub fn to_xlsx_bytes(columns: &[&str], rows: &[HashMap<String, String>]) -> Result<Vec<u8>> {
            Self::to_xlsx_workbook(&[XlsxSheet::new("Sheet1", columns, rows)])
        }

        /// 从**序列化类型**导出（字段取值走 `serde_json`，与 [`Export::to_json`] 同源）。
        ///
        /// `columns` 为字段名顺序；对象 / 数组等非标量按紧凑 JSON 文本写入。
        pub fn to_xlsx_typed<T: Serialize>(columns: &[&str], items: &[T]) -> Result<Vec<u8>> {
            let rows = typed_to_rows(columns, items)?;
            Self::to_xlsx_bytes(columns, &rows)
        }

        /// 导出**多 sheet** 工作簿。
        pub fn to_xlsx_workbook(sheets: &[XlsxSheet<'_>]) -> Result<Vec<u8>> {
            if sheets.is_empty() {
                return Err(Error::Msg("XLSX导出失败: 至少需要一个工作表".into()));
            }
            let mut wb = Workbook::new();
            let header = Format::new()
                .set_bold()
                .set_background_color(Color::RGB(0xD9E1F2));
            for sheet in sheets {
                let ws = wb.add_worksheet();
                ws.set_name(sheet.name).map_err(map_err)?;
                for (c, name) in sheet.columns.iter().enumerate() {
                    ws.write_string_with_format(0, c as u16, *name, &header)
                        .map_err(map_err)?;
                }
                for (r, row) in sheet.rows.iter().enumerate() {
                    let r = (r + 1) as u32;
                    for (c, col) in sheet.columns.iter().enumerate() {
                        let v = row.get(*col).map(|s| s.as_str()).unwrap_or("");
                        write_cell(ws, r, c as u16, v)?;
                    }
                }
                // 冻结表头 + 按内容自适应列宽
                ws.set_freeze_panes(1, 0).map_err(map_err)?;
                ws.autofit();
            }
            wb.save_to_buffer().map_err(map_err)
        }
    }

    fn typed_to_rows<T: Serialize>(
        columns: &[&str],
        items: &[T],
    ) -> Result<Vec<HashMap<String, String>>> {
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let value = serde_json::to_value(item)
                .map_err(|e| Error::Msg(format!("XLSX导出失败: 序列化失败 {}", e)))?;
            let mut row = HashMap::new();
            for col in columns {
                let text = match value.get(*col) {
                    None | Some(serde_json::Value::Null) => String::new(),
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                };
                row.insert((*col).to_string(), text);
            }
            rows.push(row);
        }
        Ok(rows)
    }

    // ── 读 ───────────────────────────────────────────────────

    fn open(data: &[u8]) -> Result<Xlsx<Cursor<&[u8]>>> {
        open_workbook_from_rs(Cursor::new(data)).map_err(|e| Error::Msg(format!("XLSX解析失败: {}", e)))
    }

    fn cell_to_string(d: &Data) -> String {
        match d {
            Data::Int(i) => i.to_string(),
            Data::Float(f) => f.to_string(),
            Data::String(s) => s.clone(),
            Data::Bool(b) => b.to_string(),
            Data::DateTime(dt) => dt.to_string(),
            Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
            Data::Error(e) => e.to_string(),
            Data::Empty => String::new(),
        }
    }

    fn range_to_rows(range: &calamine::Range<Data>) -> Vec<HashMap<String, String>> {
        let mut out = Vec::new();
        let mut headers: Vec<String> = Vec::new();
        for (r, row) in range.rows().enumerate() {
            if r == 0 {
                headers = row.iter().map(cell_to_string).collect();
                continue;
            }
            let mut map = HashMap::new();
            for (i, cell) in row.iter().enumerate() {
                if let Some(h) = headers.get(i) {
                    if !h.is_empty() {
                        map.insert(h.clone(), cell_to_string(cell));
                    }
                }
            }
            out.push(map);
        }
        out
    }

    impl Import {
        /// 列出工作簿内的所有 sheet 名（按文件内顺序）。
        pub fn xlsx_sheet_names(data: &[u8]) -> Result<Vec<String>> {
            let wb = open(data)?;
            Ok(wb.sheet_names().to_vec())
        }

        /// 解析**第一个** sheet，首行作表头。
        pub fn from_xlsx(data: &[u8]) -> Result<Vec<HashMap<String, String>>> {
            let mut wb = open(data)?;
            let first = wb
                .sheet_names()
                .first()
                .cloned()
                .ok_or_else(|| Error::Msg("XLSX解析失败: 工作簿内没有工作表".into()))?;
            let range = wb
                .worksheet_range(&first)
                .map_err(|e| Error::Msg(format!("XLSX解析失败: {}", e)))?;
            Ok(range_to_rows(&range))
        }

        /// 解析**指定** sheet，首行作表头。
        pub fn from_xlsx_sheet(data: &[u8], sheet: &str) -> Result<Vec<HashMap<String, String>>> {
            let mut wb = open(data)?;
            let range = wb
                .worksheet_range(sheet)
                .map_err(|e| Error::Msg(format!("XLSX解析失败: 工作表`{}`: {}", sheet, e)))?;
            Ok(range_to_rows(&range))
        }

        /// 解析第一个 sheet 并反序列化为具体类型（表头 = 字段名）。
        ///
        /// 单元格值**先按推断类型**解析（数字 / 布尔落成对应标量，空单元格为 `""`）；
        /// 若目标字段是 `String` 而单元格恰好是纯数字，则退回"全部按字符串"再试一次。
        /// 与写侧一致，`"00123"` 这类**不能原样往返**的文本不会被当成数字。
        pub fn from_xlsx_typed<T: DeserializeOwned>(data: &[u8]) -> Result<Vec<T>> {
            let rows = Self::from_xlsx(data)?;
            rows_to_typed(rows)
        }
    }

    fn rows_to_typed<T: DeserializeOwned>(rows: Vec<HashMap<String, String>>) -> Result<Vec<T>> {
        // 先用**推断类型**解析（数字/布尔落成对应的 JSON 标量）；失败则退回
        // "全部按字符串"再试一次——这样 `name: String` 里恰好写着 `42` 也能过。
        // 两次都失败时报第一次的错误（更贴近单元格的真实类型）。
        match deserialize_rows::<T>(&rows, true) {
            Ok(v) => Ok(v),
            Err(first) => deserialize_rows::<T>(&rows, false).map_err(|_| first),
        }
    }

    fn deserialize_rows<T: DeserializeOwned>(
        rows: &[HashMap<String, String>],
        infer: bool,
    ) -> Result<Vec<T>> {
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let item = serde_json::from_value(json_value(row, infer))
                .map_err(|e| Error::Msg(format!("XLSX解析失败: 反序列化失败 {}", e)))?;
            out.push(item);
        }
        Ok(out)
    }

    fn json_value(row: &HashMap<String, String>, infer: bool) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in row {
            map.insert(k.clone(), cell_json(v, infer));
        }
        serde_json::Value::Object(map)
    }

    fn cell_json(v: &str, infer: bool) -> serde_json::Value {
        if !infer {
            return serde_json::Value::String(v.to_string());
        }
        // 与写侧同一条"能原样往返才数值化"的规则，保证 `"00123"` 仍是文本
        match classify(v) {
            Cell::Int(i) => serde_json::Value::Number(i.into()),
            Cell::Float(f) => serde_json::Number::from_f64(f)
                .map(serde_json::Value::Number)
                .unwrap_or_else(|| serde_json::Value::String(v.to_string())),
            Cell::Bool(b) => serde_json::Value::Bool(b),
            Cell::Text(s) => serde_json::Value::String(s.to_string()),
        }
    }

    fn map_err<E: std::fmt::Display>(e: E) -> Error {
        Error::Msg(format!("XLSX写入失败: {}", e))
    }
}

#[cfg(feature = "xlsx")]
pub use xlsx_impl::XlsxSheet;

#[cfg(test)]
#[cfg(feature = "xlsx")]
mod tests {
    use super::*;

    fn row(id: &str, name: &str) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("id".into(), id.into());
        m.insert("name".into(), name.into());
        m
    }

    /// .xlsx 本质是一个 zip 包，必须以 `PK\x03\x04` 开始。
    #[test]
    fn output_is_a_real_zip_container() {
        let bytes = Export::to_xlsx_bytes(&["id", "name"], &[row("1", "Alice")]).unwrap();
        assert_eq!(&bytes[..4], &[0x50, 0x4B, 0x03, 0x04]);
    }

    #[test]
    fn roundtrip_preserves_header_and_values() {
        let bytes = Export::to_xlsx_bytes(&["id", "name"], &[row("1", "Alice"), row("2", "Bob")])
            .unwrap();
        let rows = Import::from_xlsx(&bytes).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("id").unwrap(), "1");
        assert_eq!(rows[0].get("name").unwrap(), "Alice");
        assert_eq!(rows[1].get("name").unwrap(), "Bob");
    }

    /// 零填充编号不能被 Excel 悄悄变成数字（会丢前导零）。
    #[test]
    fn leading_zero_text_is_not_coerced_to_number() {
        let bytes = Export::to_xlsx_bytes(&["id"], &[row("00123", "x")]).unwrap();
        let rows = Import::from_xlsx(&bytes).unwrap();
        assert_eq!(rows[0].get("id").unwrap(), "00123");
    }

    /// 规范写法的数字**应当**落成数值单元格（Excel 右对齐、可直接求和）。
    #[test]
    fn canonical_numbers_become_numeric_cells() {
        use calamine::{Data, Reader};
        let bytes = Export::to_xlsx_bytes(&["id"], &[row("42", "x")]).unwrap();
        let mut wb: calamine::Xlsx<_> =
            calamine::open_workbook_from_rs(std::io::Cursor::new(bytes.as_slice())).unwrap();
        let range = wb.worksheet_range("Sheet1").unwrap();
        assert!(matches!(range.get((1, 0)), Some(Data::Float(_)) | Some(Data::Int(_))));
    }

    #[test]
    fn multi_sheet_workbook() {
        let a = [row("1", "Alice")];
        let b = [row("9", "Zoe")];
        let sheets = [
            XlsxSheet::new("用户", &["id", "name"], &a),
            XlsxSheet::new("订单", &["id", "name"], &b),
        ];
        let bytes = Export::to_xlsx_workbook(&sheets).unwrap();

        let names = Import::xlsx_sheet_names(&bytes).unwrap();
        assert_eq!(names, vec!["用户".to_string(), "订单".to_string()]);
        assert_eq!(Import::from_xlsx(&bytes).unwrap()[0].get("name").unwrap(), "Alice");
        assert_eq!(
            Import::from_xlsx_sheet(&bytes, "订单").unwrap()[0].get("name").unwrap(),
            "Zoe"
        );
    }

    #[test]
    fn typed_export_and_import() {
        #[derive(Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Item {
            id: i64,
            name: String,
        }
        let items = [Item { id: 1, name: "A".into() }, Item { id: 2, name: "B".into() }];
        let bytes = Export::to_xlsx_typed(&["id", "name"], &items).unwrap();
        let back: Vec<Item> = Import::from_xlsx_typed(&bytes).unwrap();
        assert_eq!(back, items);
    }

    #[test]
    fn empty_workbook_is_rejected() {
        assert!(Export::to_xlsx_workbook(&[]).is_err());
    }
}

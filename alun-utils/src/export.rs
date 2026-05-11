//! 导出导入工具：CSV / Excel / JSON
//!
//! 使用方式简洁，一行代码完成导入导出。

use alun_core::{Result, Error};
use serde::{Serialize, de::DeserializeOwned};

/// 导出格式
pub enum ExportFormat {
    Csv,
    Json,
    Xlsx,
}

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
    pub fn to_csv(
        columns: &[&str],
        rows: &[std::collections::HashMap<String, String>],
    ) -> Result<String> {
        let mut w = csv::Writer::from_writer(Vec::new());
        w.write_record(columns).map_err(|e| Error::Msg(format!("CSV写入失败: {}", e)))?;
        for row in rows {
            let vals: Vec<&str> = columns.iter()
                .map(|c| row.get(*c).map(|s| s.as_str()).unwrap_or(""))
                .collect();
            w.write_record(&vals).map_err(|e| Error::Msg(format!("CSV写入失败: {}", e)))?;
        }
        let data = w.into_inner().map_err(|e| Error::Msg(format!("CSV输出失败: {}", e)))?;
        String::from_utf8(data).map_err(|e| Error::Msg(format!("UTF8转换失败: {}", e)))
    }

    /// 将结构化数据导出为 JSON
    pub fn to_json<T: Serialize>(items: &[T]) -> Result<String> {
        serde_json::to_string_pretty(items).map_err(|e| Error::Msg(format!("JSON序列化失败: {}", e)))
    }

    /// 导出为 XLSX 格式（BOM + CSV），兼容 Excel 打开
    pub fn to_xlsx(
        columns: &[&str],
        rows: &[std::collections::HashMap<String, String>],
    ) -> Result<Vec<u8>> {
        let csv_content = Self::to_csv(columns, rows)?;
        let bom = vec![0xEFu8, 0xBB, 0xBF];
        let mut data = bom;
        data.extend(csv_content.as_bytes());
        Ok(data)
    }
}

/// 导入工具
pub struct Import;

impl Import {
    /// 从 CSV 字符串解析为 Vec<HashMap<String, String>>
    pub fn from_csv(csv_str: &str) -> Result<Vec<std::collections::HashMap<String, String>>> {
        let mut reader = csv::Reader::from_reader(csv_str.as_bytes());
        let headers = reader.headers()
            .map_err(|e| Error::Msg(format!("CSV头解析失败: {}", e)))?
            .clone();
        let mut rows = Vec::new();
        for result in reader.records() {
            let record = result.map_err(|e| Error::Msg(format!("CSV行解析失败: {}", e)))?;
            let mut map = std::collections::HashMap::new();
            for (i, val) in record.iter().enumerate() {
                if let Some(h) = headers.get(i) { map.insert(h.to_string(), val.to_string()); }
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

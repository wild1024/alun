//! 示例 03：数据库 CRUD + 导出
//!
//! 演示 alun-db 的 Row 模式 CRUD 操作和 CSV/JSON 导出。

use alun::{App, Res, ApiError, ValidatedJson, Row};
use alun::prelude::*;
use std::collections::HashMap;

#[alun::post("/api/user")]
async fn create(ValidatedJson(req): ValidatedJson<HashMap<String, serde_json::Value>>) -> std::result::Result<Res<Row>, ApiError> {
    let mut row = Row::table("sys_user");
    for (k, v) in &req { row.set(k, v.clone()); }
    let row = db().insert(&row).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(row))
}

#[alun::get("/api/users")]
async fn list() -> std::result::Result<Res<Vec<Row>>, ApiError> {
    let rows = db().query("SELECT * FROM sys_user LIMIT 50", &[]).await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(rows))
}

#[alun::get("/api/users/export")]
async fn export() -> std::result::Result<Res<String>, ApiError> {
    let rows = db().query("SELECT id, username, real_name, status FROM sys_user LIMIT 100", &[]).await.map_err(|e| ApiError::internal(e.to_string()))?;
    let records: Vec<HashMap<String, String>> = rows.iter().map(|r| {
        let mut m = HashMap::new();
        for (k, v) in &r.data {
            m.insert(k.clone(), if let serde_json::Value::String(s) = v { s.clone() } else { v.to_string() });
        }
        m
    }).collect();
    let csv = alun_utils::Export::to_csv(&["id", "username", "real_name", "status"], &records)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(csv))
}

#[tokio::main]
async fn main() {
    App::new().expect("初始化失败").scan().start().await.unwrap();
}

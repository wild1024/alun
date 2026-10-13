//! 示例 03：数据库 CRUD + 导出
//!
//! 演示 alun-db 的 Model CRUD 操作和 CSV 导出。

use alun::prelude::*;
use axum::extract::Json;
use serde::Deserialize;
use std::collections::HashMap;

use std::result::Result;

/// `sys_user` 表对应的强类型模型。
///
/// 说明（2026-09 重构后的宏默认约定）：`#[table("sys_user")]`、`#[key]`、`#[auto]` 此处都是**显式覆盖**，
/// 其结果与默认推导一致——表名由结构体名蛇形推得（`SysUser` → `sys_user`）、名为 `id` 的字段即主键、
/// 整型 `id` 默认自增。
#[derive(Model, Debug, Clone, serde::Serialize, Deserialize)]
#[table("sys_user")]
pub struct SysUser {
    #[key]
    #[auto]
    pub id: i64,
    pub username: String,
    pub real_name: String,
    pub status: i32,
}

/// 创建用户的请求体。
#[derive(Deserialize)]
pub struct CreateUserReq {
    pub username: String,
    pub real_name: String,
    pub status: Option<i32>,
}

#[alun::post("/api/user")]
async fn create(req: Json<CreateUserReq>) -> Result<Res<SysUser>, ApiError> {
    let mut user = SysUser {
        id: 0,
        username: req.username.clone(),
        real_name: req.real_name.clone(),
        status: req.status.unwrap_or(1),
    };
    user.insert_or_update(db())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(user))
}

#[alun::get("/api/users")]
async fn list() -> Result<Res<Vec<SysUser>>, ApiError> {
    let users = SysUser::query()
        .find(db())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(users))
}

#[alun::get("/api/users/export")]
async fn export() -> Result<Res<String>, ApiError> {
    let users = SysUser::query()
        .find(db())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let records: Vec<HashMap<String, String>> = users
        .iter()
        .map(|u| {
            let mut m = HashMap::new();
            m.insert("id".into(), u.id.to_string());
            m.insert("username".into(), u.username.clone());
            m.insert("real_name".into(), u.real_name.clone());
            m.insert("status".into(), u.status.to_string());
            m
        })
        .collect();
    let csv = alun_utils::Export::to_csv(&["id", "username", "real_name", "status"], &records)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(csv))
}

#[tokio::main]
async fn main() {
    App::new()
        .expect("初始化失败")
        .scan()
        .start()
        .await
        .unwrap();
}

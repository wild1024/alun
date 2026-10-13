//! MySQL 端到端回归：标识类型承载（D010）与布尔承载（D021）。
//!
//! 依赖本地 MySQL（`ALUN_TEST_MYSQL_URL`，默认
//! `mysql://alun:alun@localhost:3306/alun_test`）；连接失败时打印原因并跳过，
//! 无 MySQL 环境不红测试。
//!
//! 覆盖的列型与方言 DDL 契约一致（`MysqlDialect::render_type`）：
//! `Uuid → CHAR(36)`、`bool → TINYINT(1)`；`BINARY(16)` 为手写列型的读回路径。

use std::future::Future;

use alun_db_core::statement::{RawStmt, Statement};
use alun_db_core::stmt::Value;
use alun_db_core::stmt::compact::CompactId;
use alun_db_driver_mysql::MysqlConnection;

/// 本地 MySQL 默认连接串（与 `alun-db/tests/mysql.rs` 一致）。
const DEFAULT_URL: &str = "mysql://alun:alun@localhost:3306/alun_test";

/// 规范 36 位 UUID 文本。
const UUID_TEXT: &str = "018f4b2c-1111-7222-8333-444455556666";

/// 连接探测上限（服务器接受 TCP 却不应答时不能无限阻塞）。
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// 探测表名（与验收项目的探针表区分）。
const TABLE: &str = "mysql_driver_probe";

/// 在单线程 runtime 上跑完异步体。
fn block_on<F: Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(f)
}

/// 构造 Raw 语句。
fn raw(sql: &str, params: Vec<Value>) -> Statement {
    Statement::Raw(RawStmt {
        sql: sql.to_string(),
        params,
    })
}

/// 解析 [`UUID_TEXT`]。
fn uuid() -> CompactId {
    let hex = UUID_TEXT.replace('-', "");
    let mut out = [0u8; 16];
    for (i, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(pair).expect("hex ascii");
        out[i] = u8::from_str_radix(text, 16).expect("hex byte");
    }
    CompactId(out)
}

#[test]
fn uuid_and_bool_roundtrip() {
    block_on(async {
        let url = std::env::var("ALUN_TEST_MYSQL_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
        // 有界探测：服务器接受 TCP 却不应答时不能让测试无限挂住。
        let mut conn =
            match tokio::time::timeout(CONNECT_TIMEOUT, MysqlConnection::connect(&url)).await {
                Ok(Ok(conn)) => conn,
                Ok(Err(e)) => {
                    eprintln!("跳过：无法连接 MySQL（{e}）");
                    return;
                }
                Err(_) => {
                    eprintln!("跳过：连接 MySQL 超时（>{}s）", CONNECT_TIMEOUT.as_secs());
                    return;
                }
            };

        for sql in [
            format!("DROP TABLE IF EXISTS {TABLE}"),
            format!(
                "CREATE TABLE {TABLE} (\
                 id BIGINT NOT NULL PRIMARY KEY, \
                 uid CHAR(36) NOT NULL, \
                 bin BINARY(16) NOT NULL, \
                 flag TINYINT(1) NOT NULL, \
                 n BIGINT NOT NULL)"
            ),
        ] {
            conn.execute_stmt(&raw(&sql, vec![]))
                .await
                .unwrap_or_else(|e| panic!("`{sql}` 失败: {e}"));
        }

        // `Uuid` 绑 36 位文本进 `CHAR(36)`（旧实现绑 16 字节原始载荷，
        // 服务端报 `Incorrect string value`）；`BINARY(16)` 列显式绑 16 字节
        for (id, flag) in [(1_i64, true), (2, false)] {
            conn.execute_stmt(&raw(
                &format!("INSERT INTO {TABLE} (id, uid, bin, flag, n) VALUES (?, ?, ?, ?, ?)"),
                vec![
                    Value::I64(id),
                    Value::Uuid(uuid()),
                    Value::Bytes(uuid().0.to_vec()),
                    Value::Bool(flag),
                    Value::I64(0),
                ],
            ))
            .await
            .expect("Uuid/Bool 写入");
        }

        let rows = conn
            .query_stmt(&raw(
                &format!("SELECT uid, bin, flag, n FROM {TABLE} ORDER BY id"),
                vec![],
            ))
            .await
            .expect("读取探针行");
        assert_eq!(rows.len(), 2, "探针表应有 2 行");

        let first = &rows.0[0];
        assert_eq!(
            first.get("uid"),
            Some(&Value::Text(UUID_TEXT.to_string())),
            "CHAR(36) 承载的文本原样读回（类型化读边界解析标识类型）"
        );
        assert_eq!(
            first.get("bin"),
            Some(&Value::Uuid(uuid())),
            "BINARY(16) 载荷必须还原为 Value::Uuid"
        );
        assert_eq!(
            first.get("flag"),
            Some(&Value::Bool(true)),
            "TINYINT(1) 的 1 必须读回 Value::Bool(true)"
        );
        assert_eq!(
            first.get("n"),
            Some(&Value::I64(0)),
            "BIGINT 列保持 Value::I64"
        );

        let second = &rows.0[1];
        assert_eq!(
            second.get("flag"),
            Some(&Value::Bool(false)),
            "TINYINT(1) 的 0 必须读回 Value::Bool(false)"
        );
        assert_ne!(
            second.get("flag"),
            second.get("n"),
            "false 与 0 必须可区分（D021）"
        );

        conn.execute_stmt(&raw(&format!("DROP TABLE IF EXISTS {TABLE}"), vec![]))
            .await
            .expect("清理探针表");
    });
}

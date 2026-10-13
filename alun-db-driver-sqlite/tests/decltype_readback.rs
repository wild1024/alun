//! 声明类型（`sqlite3_column_decltype`）驱动的读回——端到端覆盖 `query_sync`
//! 的列元数据传递。
//!
//! SQLite 是动态类型数据库：`BOOLEAN`/`DECIMAL`/`UUID` 列与
//! `INTEGER`/`TEXT` 列的**存储类别**完全相同，只有拿到列的声明类型才能
//! 还原语义。本文件用真实连接验证该链路，并锁定「声明类型无信号时不做
//! 猜测」的边界。

use alun_db_core::error::DbError;
use alun_db_core::statement::{RawStmt, Statement};
use alun_db_core::stmt::Value;
use alun_db_core::stmt::compact::{CompactDecimal, CompactId, CompactTimestamp};
use alun_db_driver_sqlite::SqliteConnection;

fn raw(sql: &str, params: Vec<Value>) -> Statement {
    Statement::Raw(RawStmt {
        sql: sql.to_string(),
        params,
    })
}

/// 构造探测表：显式声明 `BOOLEAN` / `DECIMAL` / `UUID` / `TEXT` / `INTEGER`，
/// 与 ledger 探针表的声明方式一致（SQLite 接受任意类型名）。
fn probe_connection() -> SqliteConnection {
    let conn = SqliteConnection::open("sqlite::memory:").expect("open sqlite");
    conn.execute_script_sync(
        "CREATE TABLE probe (
             id     INTEGER PRIMARY KEY,
             flag   BOOLEAN NOT NULL,
             n      INTEGER NOT NULL,
             amount DECIMAL NOT NULL,
             uid    UUID NOT NULL,
             label  TEXT NOT NULL,
             ts     TEXT NOT NULL
         )",
    )
    .expect("create probe");
    conn
}

#[test]
fn decltype_drives_readback_of_declared_types() {
    let conn = probe_connection();
    let uid = CompactId([
        0x01, 0x8f, 0x4b, 0x2c, 0x11, 0x11, 0x72, 0x22, 0x83, 0x33, 0x44, 0x44, 0x55, 0x55, 0x66,
        0x66,
    ]);
    conn.execute_sync(&raw(
        "INSERT INTO probe (id, flag, n, amount, uid, label, ts) VALUES (?, ?, ?, ?, ?, ?, ?)",
        vec![
            Value::I64(1),
            Value::Bool(true),
            Value::I64(0),
            Value::Decimal(CompactDecimal {
                mantissa: 100,
                scale: 0,
            }),
            Value::Uuid(uid),
            // 普通文本列内容恰为 "0"：不得被猜成整数/布尔
            Value::Text("0".into()),
            Value::Timestamp(CompactTimestamp(-1)),
        ],
    ))
    .expect("insert probe");

    let rows = conn
        .query_sync(&raw(
            "SELECT flag, n, amount, uid, label, ts FROM probe",
            vec![],
        ))
        .expect("select probe");
    let row = &rows.0[0];

    assert_eq!(
        row.get("flag"),
        Some(&Value::Bool(true)),
        "BOOLEAN 声明 → Bool（与同值的 0/1 整数列可区分）"
    );
    assert_eq!(row.get("n"), Some(&Value::I64(0)), "INTEGER 列仍为 I64");
    assert_eq!(
        row.get("amount"),
        Some(&Value::Decimal(CompactDecimal {
            mantissa: 100,
            scale: 0
        })),
        "DECIMAL 声明 → Decimal"
    );
    assert_eq!(row.get("uid"), Some(&Value::Uuid(uid)), "UUID 声明 → Uuid");
    assert_eq!(
        row.get("label"),
        Some(&Value::Text("0".into())),
        "TEXT 列保持原文"
    );
    assert_eq!(
        row.get("ts"),
        Some(&Value::Text("1969-12-31 23:59:59.999999".into())),
        "1970 前的时间戳格式化为合法定宽文本"
    );

    // 反向：文本承载的时间戳读回原文，且负值文本仍小于 epoch 文本（字典序 == 时间序）
    conn.execute_sync(&raw(
        "INSERT INTO probe (id, flag, n, amount, uid, label, ts) VALUES (?, ?, ?, ?, ?, ?, ?)",
        vec![
            Value::I64(2),
            Value::Bool(false),
            Value::I64(1),
            Value::Decimal(CompactDecimal {
                mantissa: 100,
                scale: 0,
            }),
            Value::Uuid(uid),
            Value::Text("0".into()),
            Value::Timestamp(CompactTimestamp(0)),
        ],
    ))
    .expect("insert probe 2");
    let ordered = conn
        .query_sync(&raw("SELECT id FROM probe ORDER BY ts ASC", vec![]))
        .expect("order by ts");
    let ids: Vec<i64> = ordered
        .0
        .iter()
        .map(|r| match r.get("id") {
            Some(Value::I64(v)) => *v,
            other => panic!("id 应为 I64，实际 {other:?}"),
        })
        .collect();
    assert_eq!(ids, vec![1, 2], "负时间戳排在 epoch 之前");
}

#[test]
fn expression_columns_have_no_decltype_and_are_not_guessed() {
    let conn = probe_connection();
    conn.execute_sync(&raw(
        "INSERT INTO probe (id, flag, n, amount, uid, label, ts) VALUES (?, ?, ?, ?, ?, ?, ?)",
        vec![
            Value::I64(1),
            Value::Bool(true),
            Value::I64(7),
            Value::Decimal(CompactDecimal {
                mantissa: 1,
                scale: 0,
            }),
            Value::Uuid(CompactId([0u8; 16])),
            Value::Text("7".into()),
            Value::Timestamp(CompactTimestamp(0)),
        ],
    ))
    .expect("insert probe");

    let rows = conn
        .query_sync(&raw("SELECT flag, label, n + 0 AS expr FROM probe", vec![]))
        .expect("select with expression");
    let row = &rows.0[0];
    assert_eq!(row.get("expr"), Some(&Value::I64(7)), "表达式列按存储类别");
    // 直接列引用仍带声明类型；`label TEXT` 内容为 "7" 时不得被算式/声明猜成数字
    assert_eq!(row.get("label"), Some(&Value::Text("7".into())));
    assert_eq!(row.get("flag"), Some(&Value::Bool(true)));
}

#[test]
fn array_binding_fails_loudly_end_to_end() {
    let conn = probe_connection();
    match conn.execute_sync(&raw(
        "INSERT INTO probe (id, flag, n, amount, uid, label, ts) VALUES (?, ?, ?, ?, ?, ?, ?)",
        vec![
            Value::I64(9),
            Value::Bool(true),
            Value::I64(0),
            Value::Array(vec![Value::I64(1), Value::I64(2)]),
            Value::Uuid(CompactId([0u8; 16])),
            Value::Text("x".into()),
            Value::Timestamp(CompactTimestamp(0)),
        ],
    )) {
        Err(DbError::UnsupportedFeature { feature, dialect }) => {
            assert_eq!(feature, "array binding");
            assert_eq!(dialect, "sqlite");
        }
        other => panic!("数组绑定应显式报 UnsupportedFeature，实际 {other:?}"),
    }

    // 失败前不得写入任何行（不得静默落 NULL）
    let rows = conn
        .query_sync(&raw("SELECT id FROM probe WHERE id = 9", vec![]))
        .expect("select");
    assert!(rows.is_empty(), "数组绑定失败后不应有行落库");
}

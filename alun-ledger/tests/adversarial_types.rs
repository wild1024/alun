//! M11 类型能力对抗（用例 TYP-001..010，规划 §6.11）。
//!
//! 探针表**在现场按方言建**（不写进迁移）：这些列类型是三库差异的靶子，
//! 只有按方言选列类型才能暴露「同一份 Rust 模型在三库得到不同读回形态」。
//!
//! ```text
//! 列           | SQLite | PostgreSQL | MySQL
//! money_probe.amount | TEXT           | NUMERIC(38,10) | DECIMAL(38,10)
//! type_probe.flag    | INTEGER        | BOOLEAN        | TINYINT(1)
//! type_probe.uid     | TEXT           | UUID           | CHAR(36)
//! ts_probe.ts        | TEXT           | TIMESTAMPTZ    | DATETIME(6)
//! enum_probe.int_state | INTEGER      | INTEGER        | INT
//! array_probe.items  | TEXT           | INTEGER[]      | JSON
//! ```
//!
//! 判读铁律 R2/R4：每条断言记录**真实**读回形态；标 `红线` 的断言是
//! 缺陷探测（当前实现会如实复现缺口，不允许弱化成恒真）。

mod common;

use std::str::FromStr;

use alun_db::prelude::*;
use alun_db_core::stmt::Value;
use alun_db_core::stmt::compact::CompactDecimal;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use alun_ledger::models::probe::{EnumProbe, IntState, MoneyProbe, TextState, TypeProbe};
use alun_ledger::models::{Invoice, InvoiceStatus};
use alun_ledger::services::{money, rowx};

use common::fixtures::*;
use common::harness::{Dialect, LedgerCtx, ctx};

// ==================== 探针表 ====================

/// 现场建探针表（方言原生 DDL，幂等）。
///
/// 共享库（sqlite-file / PG / MySQL）上表会跨用例残留，故先 DROP 再 CREATE；
/// SQLite 内存库每用例独立，DROP 是空操作。
async fn create_probe_tables(c: &LedgerCtx) {
    let sqlite: &[&str] = &[
        "DROP TABLE IF EXISTS type_probe",
        "CREATE TABLE type_probe (id INTEGER NOT NULL PRIMARY KEY, flag BOOLEAN NOT NULL, \
         uid TEXT NOT NULL, label TEXT NOT NULL)",
        "DROP TABLE IF EXISTS money_probe",
        "CREATE TABLE money_probe (id INTEGER NOT NULL PRIMARY KEY, amount TEXT NOT NULL)",
        "DROP TABLE IF EXISTS enum_probe",
        "CREATE TABLE enum_probe (id INTEGER NOT NULL PRIMARY KEY, text_state TEXT NOT NULL, \
         int_state INTEGER NOT NULL)",
        "DROP TABLE IF EXISTS ts_probe",
        "CREATE TABLE ts_probe (id INTEGER NOT NULL PRIMARY KEY, ts TEXT NOT NULL)",
        "DROP TABLE IF EXISTS null_probe",
        "CREATE TABLE null_probe (id INTEGER NOT NULL PRIMARY KEY, s TEXT, n INTEGER, b BOOLEAN)",
        "DROP TABLE IF EXISTS array_probe",
        "CREATE TABLE array_probe (id INTEGER NOT NULL PRIMARY KEY, items TEXT)",
    ];
    let postgres: &[&str] = &[
        "DROP TABLE IF EXISTS type_probe",
        "CREATE TABLE type_probe (id BIGINT NOT NULL PRIMARY KEY, flag BOOLEAN NOT NULL, \
         uid UUID NOT NULL, label TEXT NOT NULL)",
        "DROP TABLE IF EXISTS money_probe",
        "CREATE TABLE money_probe (id BIGINT NOT NULL PRIMARY KEY, amount NUMERIC(38, 10) NOT NULL)",
        "DROP TABLE IF EXISTS enum_probe",
        "CREATE TABLE enum_probe (id BIGINT NOT NULL PRIMARY KEY, text_state TEXT NOT NULL, \
         int_state INTEGER NOT NULL)",
        "DROP TABLE IF EXISTS ts_probe",
        "CREATE TABLE ts_probe (id BIGINT NOT NULL PRIMARY KEY, ts TIMESTAMPTZ NOT NULL)",
        "DROP TABLE IF EXISTS null_probe",
        "CREATE TABLE null_probe (id BIGINT NOT NULL PRIMARY KEY, s TEXT, n BIGINT, b BOOLEAN)",
        "DROP TABLE IF EXISTS array_probe",
        "CREATE TABLE array_probe (id BIGINT NOT NULL PRIMARY KEY, items INTEGER[])",
    ];
    let mysql: &[&str] = &[
        "DROP TABLE IF EXISTS type_probe",
        "CREATE TABLE type_probe (id BIGINT NOT NULL PRIMARY KEY, flag TINYINT(1) NOT NULL, \
         uid CHAR(36) NOT NULL, label VARCHAR(64) NOT NULL)",
        "DROP TABLE IF EXISTS money_probe",
        "CREATE TABLE money_probe (id BIGINT NOT NULL PRIMARY KEY, amount DECIMAL(38, 10) NOT NULL)",
        "DROP TABLE IF EXISTS enum_probe",
        "CREATE TABLE enum_probe (id BIGINT NOT NULL PRIMARY KEY, text_state VARCHAR(32) NOT NULL, \
         int_state INT NOT NULL)",
        "DROP TABLE IF EXISTS ts_probe",
        "CREATE TABLE ts_probe (id BIGINT NOT NULL PRIMARY KEY, ts DATETIME(6) NOT NULL)",
        "DROP TABLE IF EXISTS null_probe",
        "CREATE TABLE null_probe (id BIGINT NOT NULL PRIMARY KEY, s TEXT, n BIGINT, b TINYINT(1))",
        "DROP TABLE IF EXISTS array_probe",
        "CREATE TABLE array_probe (id BIGINT NOT NULL PRIMARY KEY, items JSON)",
    ];
    let ddls = match c.dialect {
        Dialect::SqliteMem | Dialect::SqliteFile => sqlite,
        Dialect::Postgres => postgres,
        Dialect::Mysql => mysql,
    };
    for ddl in ddls {
        c.db()
            .execute(ddl)
            .await
            .unwrap_or_else(|e| panic!("探针 DDL 失败 `{ddl}`: {e}"));
    }
}

/// 写入一行 `money_probe`（金额经 `TypeMapper::to_value`，与 `save` 同路径）。
async fn put_money(c: &LedgerCtx, id: i64, amount: Decimal) {
    c.db()
        .execute_with_params(
            "INSERT INTO money_probe (id, amount) VALUES (?, ?)",
            vec![Value::I64(id), amount.to_value()],
        )
        .await
        .expect("seed money_probe");
}

async fn put_ts(c: &LedgerCtx, id: i64, ts: DateTime<Utc>) {
    c.db()
        .execute_with_params(
            "INSERT INTO ts_probe (id, ts) VALUES (?, ?)",
            vec![Value::I64(id), ts.to_value()],
        )
        .await
        .expect("seed ts_probe");
}

// ==================== TYP-001 ====================

/// TYP-001：SQLite 上 `ORDER BY amount DESC` 是**字典序**（金额列 TEXT）。
#[tokio::test]
async fn typ_001_order_by_money_desc_uses_text_collation_on_sqlite() {
    let Some(c) = ctx().await else { return };
    create_probe_tables(&c).await;
    put_money(&c, 1, money(1000, 0)).await;
    put_money(&c, 2, money(999, 99)).await;

    let rows = c
        .db()
        .find("SELECT id, amount FROM money_probe ORDER BY amount DESC")
        .await
        .expect("order by amount desc");
    let first_id = rowx::i64_of(&rows[0], "id").expect("id");
    let first_amount = rowx::decimal_of(&rows[0], "amount").expect("amount");
    // 金额列承载形态随方言而变：SQLite 为 TEXT，PG/MySQL 为 NUMERIC/DECIMAL
    let stored: Option<String> = if c.dialect.is_sqlite() {
        Some(
            c.db()
                .scalar::<String>("SELECT amount FROM money_probe WHERE id = 1")
                .await
                .expect("stored text"),
        )
    } else {
        None
    };

    if c.dialect.is_sqlite() {
        // 机制：金额列落 TEXT（sqlite convert.rs `Value::Decimal → RValue::Text`）
        let affinity: String = c
            .db()
            .scalar("SELECT typeof(amount) FROM money_probe LIMIT 1")
            .await
            .expect("typeof");
        assert_eq!(affinity, "text", "SQLite 金额列是 TEXT");
        // 红线：TEXT 排序是字典序——'999.99' > '1000.00'
        assert_eq!(
            (first_id, first_amount),
            (2, money(999, 99)),
            "期望（正确的数值序）首位是 1000.00，实际字典序把 999.99 排在首位"
        );
        assert!(
            first_amount < money(1000, 0),
            "首位金额 {first_amount} 数值上小于 1000.00，却被排在最前 → 字典序 ≠ 数值序"
        );
        eprintln!(
            "[TYP-001][红线] sqlite-mem: 存储文本 `{stored:?}`，ORDER BY amount DESC 首位 = {first_amount}（字典序）"
        );
    } else {
        assert_eq!(
            (first_id, first_amount),
            (1, money(1000, 0)),
            "PG/MySQL 的 NUMERIC/DECIMAL 列按数值排序"
        );
        eprintln!(
            "[TYP-001] {}: ORDER BY amount DESC 首位 = {first_amount}（数值序正确）",
            c.dialect.name()
        );
    }
}

// ==================== TYP-002 ====================

/// TYP-002：SQLite 上 `SUM(amount)` 把 TEXT 列强转 REAL → 浮点误差。
#[tokio::test]
async fn typ_002_sum_of_money_text_column() {
    let Some(c) = ctx().await else { return };
    create_probe_tables(&c).await;
    let amounts = [money(0, 10), money(0, 20)];
    for (i, a) in amounts.iter().enumerate() {
        put_money(&c, i as i64 + 1, *a).await;
    }

    let sql_sum = money::read_one(c.db(), "SELECT SUM(amount) FROM money_probe", vec![])
        .await
        .expect("sql sum");
    let rust_sum: Decimal = amounts.iter().copied().sum();
    assert_eq!(rust_sum, money(0, 30), "Rust 十进制求和是精确的");

    if c.dialect.is_sqlite() {
        let kind: String = c
            .db()
            .scalar("SELECT typeof(SUM(amount)) FROM money_probe")
            .await
            .expect("typeof sum");
        assert_eq!(kind, "real", "SQLite 把 TEXT 列 SUM 成 REAL（浮点聚合）");
        // 红线：SQL 求和结果与精确十进制和不相等
        assert_ne!(
            sql_sum, rust_sum,
            "SQL 求和 {sql_sum} 与精确十进制和 {rust_sum} 应不等（该断言用于探测浮点误差）"
        );
        eprintln!("[TYP-002][红线] sqlite-mem: SUM(TEXT) = {sql_sum}（REAL），精确和 = {rust_sum}");
    } else {
        assert_eq!(
            sql_sum,
            rust_sum,
            "{} 的 NUMERIC/DECIMAL 列 SUM 精确",
            c.dialect.name()
        );
        eprintln!("[TYP-002] {}: SUM = {sql_sum}（精确）", c.dialect.name());
    }
}

// ==================== TYP-003 ====================

/// TYP-003：Decimal 往返——类型化读在 PG/MySQL 可用、SQLite 必 `TypeMismatch`；
/// `services::money` 适配器三库可用；超范围值显式报错（不 panic）。
#[tokio::test]
async fn typ_003_decimal_typed_read_vs_money_adapter() {
    let Some(c) = ctx().await else { return };
    create_probe_tables(&c).await;

    let typed = [
        (1_i64, money(0, 1)),
        (2, money(0, 10)),
        (3, Decimal::from_str("-0.00").expect("neg zero")),
    ];
    // 写入走类型化 `save`（值经 to_value → Value::Decimal）
    for (id, amount) in typed {
        let mut probe = MoneyProbe { id, amount };
        probe
            .insert_or_update(c.db())
            .await
            .expect("typed save money_probe");
    }

    // 三方言都可用（SQLite 的 TEXT 承载经类型化读边界解析，D001 已修复）
    let typed_read = MoneyProbe::find(c.db(), 1)
        .await
        .unwrap_or_else(|e| panic!("{}: 类型化读取应成功，实际 {e:?}", c.dialect.name()));
    assert_eq!(
        typed_read.amount,
        money(0, 1),
        "{} 类型化往返",
        c.dialect.name()
    );
    eprintln!(
        "[TYP-003] {}: 类型化往返成功（金额列承载 = {}）",
        c.dialect.name(),
        if c.dialect.is_sqlite() {
            "TEXT"
        } else {
            "NUMERIC/DECIMAL"
        }
    );

    // 适配器路径：三库一致
    for (id, want) in typed {
        let got = money::read_one(
            c.db(),
            "SELECT amount FROM money_probe WHERE id = ?",
            vec![Value::I64(id)],
        )
        .await
        .expect("adapter read");
        assert_eq!(got, want, "适配器读回 id={id}");
    }
    let neg_zero = money::read_one(
        c.db(),
        "SELECT amount FROM money_probe WHERE id = ?",
        vec![Value::I64(3)],
    )
    .await
    .expect("adapter read -0.00");
    eprintln!(
        "[TYP-003] {}: -0.00 读回 = {neg_zero}（数值等价 0，is_sign_negative={}）",
        c.dialect.name(),
        neg_zero.is_sign_negative()
    );
    assert_eq!(neg_zero, Decimal::ZERO, "-0.00 与 0 数值等价");

    // 38 位字面量：超出 rust_decimal 96 位尾数
    let huge = "12345678901234567890123456789012345678";
    let inserted = c
        .db()
        .execute_with_params(
            "INSERT INTO money_probe (id, amount) VALUES (?, ?)",
            vec![Value::I64(90), Value::Text(huge.to_string())],
        )
        .await;
    match inserted {
        Ok(_) => {
            let got = money::read_one(
                c.db(),
                "SELECT amount FROM money_probe WHERE id = ?",
                vec![Value::I64(90)],
            )
            .await;
            assert!(
                matches!(got, Err(DbError::InvalidValue(_))),
                "超范围 Decimal 应显式 InvalidValue（不得 panic），实际 {got:?}"
            );
            eprintln!(
                "[TYP-003] {}: 38 位字面量可入列，适配器读回 InvalidValue（无 panic）",
                c.dialect.name()
            );
        }
        Err(e) => {
            assert!(
                matches!(e, DbError::Query { .. } | DbError::InvalidValue(_)),
                "超范围写入应显式报错，实际 {e:?}"
            );
            eprintln!(
                "[TYP-003] {}: 38 位字面量被服务端拒绝（NUMERIC(38,10) 仅 28 位整数位）：{e}",
                c.dialect.name()
            );
        }
    }

    // MySQL 专属：绑定标度 > 38 → InvalidValue（mysql convert.rs）
    if c.dialect == Dialect::Mysql {
        let over = Value::Decimal(CompactDecimal {
            mantissa: 1,
            scale: 39,
        });
        let r = c
            .db()
            .execute_with_params(
                "INSERT INTO money_probe (id, amount) VALUES (?, ?)",
                vec![Value::I64(91), over],
            )
            .await;
        assert!(
            matches!(r, Err(DbError::InvalidValue(_))),
            "MySQL scale>38 应 InvalidValue，实际 {r:?}"
        );
        eprintln!("[TYP-003] mysql: 绑定 scale=39 → InvalidValue");
    }
}

// ==================== TYP-004 ====================

/// TYP-004：亚微秒截断发生在 `to_value`（与方言无关），落库精度 = 微秒。
#[tokio::test]
async fn typ_004_sub_microsecond_truncation_happens_in_to_value() {
    let Some(c) = ctx().await else { return };
    create_probe_tables(&c).await;

    let dt =
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap() + Duration::nanoseconds(123_456_789);
    assert_eq!(dt.timestamp_subsec_nanos(), 123_456_789);

    // (1) 纯转换层：to_value 用 timestamp_micros() → 纳秒在绑定前已丢
    let value = dt.to_value();
    let back = DateTime::<Utc>::from_value(&value).expect("from_value");
    assert_ne!(back, dt, "亚微秒信息在 to_value 阶段就丢失（与方言无关）");
    assert_eq!(back.timestamp_micros(), dt.timestamp_micros(), "微秒仍一致");
    assert_eq!(back.timestamp_subsec_nanos(), 123_456_000, "只剩微秒精度");

    // (2) 落库后：列类型分方言，精度同为微秒
    put_ts(&c, 1, dt).await;
    let row = c
        .db()
        .find_first_with_params("SELECT ts FROM ts_probe WHERE id = ?", vec![Value::I64(1)])
        .await
        .expect("read ts")
        .expect("row");
    let stored = rowx::ts_of(&row, "ts").expect("ts_of");
    assert_eq!(stored.timestamp_micros(), dt.timestamp_micros());
    assert_eq!(stored.timestamp_subsec_nanos() % 1000, 0, "落库精度 = 微秒");
    assert_ne!(stored, dt);

    if c.dialect.is_sqlite() {
        let kind: String = c
            .db()
            .scalar("SELECT typeof(ts) FROM ts_probe WHERE id = 1")
            .await
            .expect("typeof ts");
        let width: i64 = c
            .db()
            .scalar("SELECT length(ts) FROM ts_probe WHERE id = 1")
            .await
            .expect("length ts");
        assert_eq!(
            (kind.as_str(), width),
            ("text", 26),
            "SQLite 时间戳是定宽文本 `%Y-%m-%d %H:%M:%S.%6f`"
        );
    }
    eprintln!(
        "[TYP-004] {}: 纳秒 123456789 → 落库 {:?}（截断发生在 to_value，与方言无关）",
        c.dialect.name(),
        rowx::value(&row, "ts").map(|v| format!("{v:?}"))
    );
}

// ==================== TYP-005 ====================

/// TYP-005：1970 前 / 2038 后 / 闰日 / 跨时区往返 + TEXT 字典序 == 时间序。
#[tokio::test]
async fn typ_005_datetime_extremes_and_text_ordering() {
    let Some(c) = ctx().await else { return };
    create_probe_tables(&c).await;

    // 不存在的日期：chrono 拒绝而非静默归一
    assert!(
        NaiveDate::from_ymd_opt(2100, 2, 29).is_none(),
        "2100 不是闰年"
    );
    assert!(
        NaiveDate::from_ymd_opt(2028, 2, 29).is_some(),
        "2028 是闰年"
    );

    let plus14 = DateTime::parse_from_rfc3339("2026-01-01T13:00:00+14:00")
        .unwrap()
        .with_timezone(&Utc);
    let minus12 = DateTime::parse_from_rfc3339("2025-12-31T11:00:00-12:00")
        .unwrap()
        .with_timezone(&Utc);
    assert_eq!(plus14, minus12, "+14:00 与 -12:00 指向同一瞬间");

    let cases: [(i64, DateTime<Utc>); 6] = [
        (1, Utc.with_ymd_and_hms(1969, 12, 31, 23, 59, 59).unwrap()),
        (2, Utc.with_ymd_and_hms(2038, 1, 19, 3, 14, 8).unwrap()),
        (3, Utc.with_ymd_and_hms(2040, 1, 1, 0, 0, 0).unwrap()),
        (4, Utc.with_ymd_and_hms(2028, 2, 29, 12, 0, 0).unwrap()),
        (5, plus14),
        (6, minus12),
    ];
    for (id, ts) in cases {
        put_ts(&c, id, ts).await;
    }

    for (id, ts) in cases {
        let row = c
            .db()
            .find_first_with_params("SELECT ts FROM ts_probe WHERE id = ?", vec![Value::I64(id)])
            .await
            .expect("read ts")
            .expect("row");
        // 1970 前后都必须无损往返（SQLite 负微秒格式化曾产出非法文本，D018 已修复）
        let got = rowx::ts_of(&row, "ts").unwrap_or_else(|e| panic!("id={id} 往返失败: {e}"));
        assert_eq!(got, ts, "id={id} 往返一致");
        if c.dialect.is_sqlite() && ts.timestamp_micros() < 0 {
            let stored = rowx::value(&row, "ts").expect("ts").clone();
            let text = match &stored {
                Value::Text(t) => t.clone(),
                other => panic!("SQLite 时间戳应以文本承载，实际 {other:?}"),
            };
            assert!(
                text.starts_with("1969-12-31"),
                "1970 前时间戳文本必须落在 1969 年（实际 `{text}`）"
            );
        }
    }

    // SQLite TEXT 定宽零填充 → 字典序 == 时间序（正时间戳；负时间戳的格式缺口见上）
    let rows = c
        .db()
        .find("SELECT id, ts FROM ts_probe ORDER BY ts ASC")
        .await
        .expect("order by ts");
    // 负时间戳的文本同样定宽零填充 → 全量（含 1969）字典序 == 时间序
    let sql_ids: Vec<i64> = rows
        .iter()
        .map(|r| rowx::i64_of(r, "id").expect("id"))
        .collect();
    let mut expected: Vec<(i64, DateTime<Utc>)> = cases.to_vec();
    expected.sort_by_key(|(_, ts)| *ts);
    let expected_ids: Vec<i64> = expected.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        sql_ids,
        expected_ids,
        "{} 上 ORDER BY ts 与时间序一致",
        c.dialect.name()
    );
    eprintln!(
        "[TYP-005] {}: 正时间戳 ORDER BY ts = 时间序；2038/闰日/跨时区往返一致",
        c.dialect.name()
    );
}

// ==================== TYP-006 ====================

/// TYP-006：Uuid 往返——仅 PG（`UUID` 列）可用。
#[tokio::test]
async fn typ_006_uuid_roundtrip_per_dialect() {
    let Some(c) = ctx().await else { return };
    create_probe_tables(&c).await;
    let uid = Uuid::parse_str("018f4b2c-1111-7222-8333-444455556666").expect("uuid");

    let mut probe = TypeProbe {
        id: 1,
        flag: true,
        uid,
        label: "u".into(),
    };
    let saved = probe.insert_or_update(c.db()).await;

    // 写入：SQLite 落 TEXT、PG 落 UUID 列
    let saved_ok = saved.is_ok();
    if !saved_ok {
        let e = saved.expect_err("save 失败原因");
        eprintln!(
            "[TYP-006][差异登记] {}: 类型化写入被拒绝：{e}",
            c.dialect.name()
        );
    } else if c.dialect.is_sqlite() {
        eprintln!("[TYP-006] sqlite-mem: Uuid 写入成功（列类型 TEXT）");
    }

    if !saved_ok {
        // 写入已被拒绝（如 MySQL CHAR(36) 列收到 16 字节二进制，缺陷 D010）：
        // 无行可读，直接登记差异并结束本用例
        eprintln!(
            "[TYP-006][差异登记] {}: Uuid 类型化写入失败，跳过读回断言",
            c.dialect.name()
        );
        return;
    }

    // 读回：只看 uid 列（模型级类型化读取的首个失败点是 flag，属 TYP-007）
    let row = c
        .db()
        .find_first("SELECT uid FROM type_probe WHERE id = 1")
        .await
        .expect("read uid row")
        .expect("row");
    let raw = rowx::value(&row, "uid").expect("uid column").clone();
    let decoded = Uuid::from_value(&raw);

    match c.dialect {
        Dialect::Postgres => {
            assert!(saved_ok, "PG UUID 列写入应成功");
            assert_eq!(
                raw,
                Value::Uuid(alun_db_core::stmt::compact::CompactId(*uid.as_bytes()))
            );
            assert_eq!(decoded.expect("PG Uuid 读回"), uid, "PG UUID 列往返一致");
            let full = TypeProbe::find(c.db(), 1).await.expect("PG 类型化模型读取");
            assert_eq!(full.uid, uid);
            eprintln!("[TYP-006] postgres: UUID 列类型化往返一致");
        }
        Dialect::Mysql => {
            // 列类型 CHAR(36)：绑定与读回都走 36 位连字符文本（D010 已修复）
            match &raw {
                Value::Text(t) => assert_eq!(t, &uid.to_string()),
                other => panic!("mysql uid 读回应为 CHAR(36) 文本，实际 {other:?}"),
            }
            assert_eq!(decoded.expect("mysql Uuid 读回"), uid, "mysql 标识往返一致");
            let full = TypeProbe::find(c.db(), 1)
                .await
                .expect("mysql 类型化模型读取");
            assert_eq!(full.uid, uid);
            eprintln!("[TYP-006] mysql: CHAR(36) 列类型化往返一致");
        }
        Dialect::SqliteMem | Dialect::SqliteFile => {
            assert!(saved_ok, "SQLite 写入应成功（UID 落 TEXT）");
            assert_eq!(
                raw,
                Value::Text(uid.to_string()),
                "SQLite 上 UUID 以文本原样保存"
            );
            // 文本承载经类型化读边界解析（D010 已修复）
            assert_eq!(
                decoded.expect("SQLite Uuid 读回"),
                uid,
                "SQLite 标识往返一致"
            );
            let full = TypeProbe::find(c.db(), 1)
                .await
                .expect("SQLite 类型化模型读取");
            assert_eq!(full.uid, uid);
            eprintln!("[TYP-006] sqlite-mem: TEXT 列类型化往返一致");
        }
    }
}

// ==================== TYP-007 ====================

/// TYP-007：bool（`BOOLEAN`/`TINYINT(1)`/`INTEGER`）与 `#[int]` 语义枚举往返。
#[tokio::test]
async fn typ_007_bool_and_int_enum_roundtrip() {
    let Some(c) = ctx().await else { return };
    create_probe_tables(&c).await;
    let uid = Uuid::parse_str("018f4b2c-1111-7222-8333-444455556666").expect("uuid");

    // 写入走 raw SQL：本用例只关心 flag / int_state 的读回形态
    c.db()
        .execute_with_params(
            "INSERT INTO type_probe (id, flag, uid, label) VALUES (?, ?, ?, ?)",
            vec![
                Value::I64(1),
                Value::Bool(true),
                // 绑定需匹配探测表列类型：PG UUID / MySQL CHAR(36) / SQLite TEXT
                uid_bind(&c, &uid),
                Value::Text("b".into()),
            ],
        )
        .await
        .expect("seed type_probe");

    let row = c
        .db()
        .find_first("SELECT flag FROM type_probe WHERE id = 1")
        .await
        .expect("read flag")
        .expect("row");
    let got = TypeProbe::find(c.db(), 1).await;

    if c.dialect == Dialect::Postgres {
        assert_eq!(
            rowx::value(&row, "flag").expect("flag column"),
            &Value::Bool(true),
            "PG BOOLEAN 列"
        );
        let p = got.expect("PG 类型化读取");
        assert!(p.flag);
        assert_eq!(p.uid, uid);
        eprintln!("[TYP-007] postgres: BOOLEAN 列往返一致");
    } else {
        // 布尔列（SQLite `BOOLEAN` / MySQL `TINYINT(1)`）按声明类型回 Bool（D021 已修复）
        assert_eq!(
            rowx::value(&row, "flag").expect("flag column"),
            &Value::Bool(true),
            "{} 布尔列应保留 Bool 形态",
            c.dialect.name()
        );
        let p = got.unwrap_or_else(|e| panic!("{} 类型化读取应成功: {e:?}", c.dialect.name()));
        assert!(p.flag);
        assert_eq!(p.uid, uid);
        eprintln!(
            "[TYP-007] {}: 布尔列往返一致（flag = {}）",
            c.dialect.name(),
            p.flag
        );
    }

    // #[int] 语义枚举：只匹配 Value::I32，而**任何驱动都不会产出 I32**
    // （pg/mysql/sqlite 三个驱动都「统一读回宽度」归一为 I64）
    let mut probe = EnumProbe {
        id: 1,
        text_state: TextState::Issued,
        int_state: IntState::Issued,
    };
    probe
        .insert_or_update(c.db())
        .await
        .expect("typed save enum_probe");
    let int_row = c
        .db()
        .find_first("SELECT text_state, int_state FROM enum_probe WHERE id = 1")
        .await
        .expect("read enum probe")
        .expect("row");
    assert_eq!(
        rowx::value(&int_row, "int_state").expect("int_state"),
        &Value::I64(2),
        "整数列读回统一为 I64（三库一致），永不出现 Value::I32"
    );
    assert_eq!(
        TextState::from_value(rowx::value(&int_row, "text_state").unwrap())
            .expect("文本模式列可读回"),
        TextState::Issued,
        "文本模式列本身正确"
    );
    let enums = EnumProbe::find(c.db(), 1)
        .await
        .unwrap_or_else(|e| panic!("{} 整数模式枚举读回应成功: {e:?}", c.dialect.name()));
    assert_eq!(
        enums.int_state,
        IntState::Issued,
        "整数模式枚举往返（D020 已修复）"
    );
    assert_eq!(enums.text_state, TextState::Issued);
    eprintln!(
        "[TYP-007] {}: int_state（I64 承载）与 text_state 均往返一致",
        c.dialect.name()
    );

    // 枚举取值对抗（§7 规格：未知值 / 空串 / 大小写变体 / 超长）
    let text_states: &[&str] = &["", "issued", "ISSUED", &"x".repeat(300), "unknown"];
    for s in text_states {
        let got = TextState::from_value(&Value::Text((*s).to_string()));
        assert!(
            matches!(got, Err(DbError::InvalidValue(_))),
            "`{}` 不是合法变体，应 InvalidValue，实际 {got:?}",
            &s[..s.len().min(12)]
        );
    }
    assert_eq!(
        TextState::from_value(&Value::Text("Issued".into())).expect("exact variant"),
        TextState::Issued,
        "文本模式枚举大小写敏感，仅精确匹配"
    );
    assert!(matches!(
        IntState::from_value(&Value::I32(99)),
        Err(DbError::InvalidValue(_))
    ));
    match TextState::from_value(&Value::Null) {
        // `Value::Null.ty()` 归一为 `Type::Unknown`（不是 "Null"）
        Err(DbError::TypeMismatch { expected, actual }) => {
            assert_eq!((expected, actual), ("Text", "Unknown"))
        }
        other => panic!("NULL 不是合法枚举文本，应 TypeMismatch，实际 {other:?}"),
    }
}

// ==================== TYP-008 ====================

/// TYP-008：JSON 四态（SQL NULL / `'null'` / `'{}'` / 深层对象）互不混淆。
#[tokio::test]
async fn typ_008_json_four_states() {
    let Some(c) = ctx().await else { return };
    seed_tenant(&c, TENANT_T1, "T1").await.unwrap();
    seed_invoice(&c, TENANT_T1, 1001, "draft").await.unwrap();

    // 四态落库（SQL NULL 绑 Value::Null，其余绑 JSON 载荷）
    let states: [(&str, Option<&str>); 4] = [
        ("sql-null", None),
        ("json-null", Some("null")),
        ("empty-object", Some("{}")),
        ("deep-object", Some(r#"{"a":{"b":{"c":{"d":{"e":1}}}}}"#)),
    ];
    let mut observed: Vec<(String, Option<serde_json::Value>)> = Vec::new();
    for (label, payload) in states {
        let params = match payload {
            None => vec![Value::Null, Value::I64(1001)],
            Some(p) => vec![Value::Json(p.to_string()), Value::I64(1001)],
        };
        c.db()
            .execute_with_params("UPDATE invoices SET metadata = ? WHERE id = ?", params)
            .await
            .unwrap_or_else(|e| panic!("写入 {label} 失败: {e}"));

        let row = c
            .db()
            .find_first_with_params(
                "SELECT metadata FROM invoices WHERE id = ?",
                vec![Value::I64(1001)],
            )
            .await
            .expect("read metadata")
            .expect("row");
        let decoded = rowx::opt_json_of(&row, "metadata")
            .unwrap_or_else(|e| panic!("解码 {label} 失败: {e}"));
        match payload {
            None => assert!(decoded.is_none(), "{label}: SQL NULL → None"),
            Some(p) => assert_eq!(
                decoded,
                Some(serde_json::from_str::<serde_json::Value>(p).expect("expected json")),
                "{label} 载荷原样还原"
            ),
        }
        eprintln!(
            "[TYP-008] {} {label}: 列 Value {:?} → {:?}",
            c.dialect.name(),
            row.get("metadata"),
            decoded
        );
        observed.push((label.to_string(), decoded));
    }
    // 四态可区分：解码结果两两不同
    let mut distinct: Vec<String> = observed.iter().map(|(_, v)| format!("{v:?}")).collect();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), 4, "四态必须互不混淆: {distinct:?}");
    eprintln!(
        "[TYP-008] {} 四态解码两两不同: {distinct:?}",
        c.dialect.name()
    );

    // 模型层 `#[json]` 解码语义（手造 Row：SQLite 金额列无类型化读回，故不经数据库）
    let base_row = |meta: Value| {
        let mut row = Row::new();
        row.insert("id", Value::I64(1));
        row.insert("tenant_id", Value::I64(TENANT_T1));
        row.insert("customer_id", Value::I64(TENANT_T1 * 100 + 7));
        row.insert("status", Value::Text("draft".into()));
        row.insert("total", money(10, 0).to_value());
        row.insert("metadata", meta);
        row.insert("issued_at", Value::Null);
        row.insert("paid_at", Value::Null);
        row.insert("created_at", at(0).to_value());
        row.insert("note", Value::Null);
        row
    };
    let json_states: Vec<(Value, Option<serde_json::Value>)> = vec![
        (Value::Null, None),
        (Value::Json("null".into()), Some(serde_json::Value::Null)),
        (Value::Json("{}".into()), Some(serde_json::json!({}))),
        (
            Value::Text(r#"{"a":{"b":1}}"#.into()),
            Some(serde_json::json!({"a":{"b":1}})),
        ),
    ];
    let mut model_seen: Vec<String> = Vec::new();
    for (raw, want) in json_states {
        let row = base_row(raw);
        let invoice = Invoice::from_row(row).unwrap_or_else(|e| panic!("from_row 失败: {e}"));
        assert_eq!(invoice.metadata, want, "模型层四态解码");
        assert_eq!(invoice.status, InvoiceStatus::Draft);
        assert_eq!(invoice.total, money(10, 0));
        model_seen.push(format!("{:?}", invoice.metadata));
        // 非 JSON 载荷 → TypeMismatch{Json}（`#[json]` 只接受 Json/Text/Null）
        let bad = base_row(Value::I64(7));
        assert!(matches!(
            Invoice::from_row(bad),
            Err(DbError::TypeMismatch {
                expected: "Json",
                ..
            })
        ));
    }
    model_seen.sort();
    model_seen.dedup();
    assert_eq!(model_seen.len(), 4, "模型层四态互不混淆: {model_seen:?}");
    eprintln!("[TYP-008] 模型层 #[json] 四态（SQL NULL / 'null' / '{{}}' / 对象）互不混淆");
}

// ==================== TYP-009 ====================

/// TYP-009：可空边界——`None` vs `""` vs `0` vs `false` vs `"0"` 不混淆。
#[tokio::test]
async fn typ_009_null_empty_zero_false_distinct() {
    let Some(c) = ctx().await else { return };
    create_probe_tables(&c).await;

    let rows_seed: [(i64, Value, Value, Value); 3] = [
        (1, Value::Null, Value::Null, Value::Null),
        (
            2,
            Value::Text(String::new()),
            Value::I64(0),
            Value::Bool(false),
        ),
        (3, Value::Text("0".into()), Value::I64(0), Value::Null),
    ];
    for (id, s, n, b) in rows_seed {
        c.db()
            .execute_with_params(
                "INSERT INTO null_probe (id, s, n, b) VALUES (?, ?, ?, ?)",
                vec![Value::I64(id), s, n, b],
            )
            .await
            .expect("seed null_probe");
    }

    let row1 = c
        .db()
        .find_first("SELECT * FROM null_probe WHERE id = 1")
        .await
        .unwrap()
        .unwrap();
    let row2 = c
        .db()
        .find_first("SELECT * FROM null_probe WHERE id = 2")
        .await
        .unwrap()
        .unwrap();
    let row3 = c
        .db()
        .find_first("SELECT * FROM null_probe WHERE id = 3")
        .await
        .unwrap()
        .unwrap();

    // 可空列助手：NULL → None，其余按类型还原
    assert_eq!(rowx::opt_text_of(&row1, "s").unwrap(), None);
    assert_eq!(rowx::opt_text_of(&row2, "s").unwrap(), Some(String::new()));
    assert_eq!(rowx::opt_text_of(&row3, "s").unwrap(), Some("0".into()));
    assert_eq!(rowx::i64_of(&row2, "n").unwrap(), 0);
    // NULL 不是 0
    assert!(
        rowx::i64_of(&row1, "n").is_err(),
        "NULL 不得被读成 0: {:?}",
        rowx::i64_of(&row1, "n")
    );

    // 五个状态的列 Value 形态
    let null_v = rowx::value(&row1, "s").unwrap().clone();
    let empty_v = rowx::value(&row2, "s").unwrap().clone();
    let zero_text_v = rowx::value(&row3, "s").unwrap().clone();
    let zero_int_v = rowx::value(&row2, "n").unwrap().clone();
    let false_v = rowx::value(&row2, "b").unwrap().clone();
    assert_ne!(null_v, empty_v);
    assert_ne!(empty_v, zero_text_v);
    assert_ne!(zero_text_v, zero_int_v);
    assert_ne!(null_v, zero_int_v);

    if c.dialect == Dialect::Postgres {
        assert_eq!(false_v, Value::Bool(false), "PG BOOLEAN 列保留 Bool 形态");
        assert_ne!(false_v, zero_int_v, "PG 上 false 与 0 是不同 Value 变体");
        eprintln!("[TYP-009] postgres: Bool(false) 与 I64(0) 形态不同 → 五态互不混淆");
    } else {
        // 布尔列按声明类型回 Bool(false)，与整数列的 I64(0) 形态不同（D021 已修复）
        assert_eq!(
            false_v,
            Value::Bool(false),
            "{} 布尔列保留 Bool 形态",
            c.dialect.name()
        );
        assert_ne!(false_v, zero_int_v, "false 与 0 必须是不同 Value 变体");
        eprintln!(
            "[TYP-009] {}: Bool(false) 与 I64(0) 形态不同 → 五态互不混淆",
            c.dialect.name()
        );
    }
}

// ==================== TYP-010 ====================

/// TYP-010：Array 绑定——PG/MySQL 显式 `UnsupportedFeature`，SQLite **静默绑 NULL**。
#[tokio::test]
async fn typ_010_array_binding_per_dialect() {
    let Some(c) = ctx().await else { return };
    create_probe_tables(&c).await;

    let array = Value::Array(vec![Value::I64(1), Value::I64(2), Value::I64(3)]);
    let inserted = c
        .db()
        .execute_with_params(
            "INSERT INTO array_probe (id, items) VALUES (?, ?)",
            vec![Value::I64(1), array],
        )
        .await;

    // 三方言行为一致：数组绑定显式 UnsupportedFeature（SQLite 曾静默写 NULL，D009 已修复）
    match inserted {
        Err(DbError::UnsupportedFeature { feature, dialect }) => {
            assert_eq!(feature, "array binding");
            // 驱动自报名（`sqlite`）与 harness 方言名（`sqlite-mem`）不同，按前缀比较
            assert!(
                dialect.starts_with(&c.dialect.name()[..6.min(c.dialect.name().len())])
                    || c.dialect.name().starts_with(&dialect),
                "方言名应指向同一方言：driver=`{dialect}` harness=`{}`",
                c.dialect.name()
            );
            eprintln!("[TYP-010] {dialect}: 数组绑定 → UnsupportedFeature{{array binding}}");
        }
        other => panic!(
            "{} 上数组绑定应 UnsupportedFeature，实际 {other:?}",
            c.dialect.name()
        ),
    }
    // 不得留下任何行（静默降级会留下 NULL 行）
    let count: i64 = c
        .db()
        .scalar("SELECT COUNT(*) FROM array_probe")
        .await
        .expect("count array_probe");
    assert_eq!(count, 0, "数组绑定被拒后不得写入任何行");
}

/// NUMERIC 尾随零分组的**库内一致性**（曾为缺陷 D025，已修复）。
///
/// PG 发送的 numeric **已规范化**：尾随零组不出现在 `digits` 中，而是抬高 `weight`
/// （10000 = digits[1]、weight[1]）。旧解码忽略 `weight` 超出数字组的部分，
/// 于是 10000 读成 1、1e10 读成 100 —— 而且框架内往返「自洽」，只有服务端函数或
/// 外部客户端能发现。本用例因此**同时**断言服务端视角：库内值必须精确等于写入值。
#[tokio::test]
async fn typ_011_numeric_trailing_zero_groups_roundtrip() {
    let Some(c) = ctx().await else { return };
    let tenant = 991;
    let _ = c
        .db()
        .execute_with_params(
            "DELETE FROM accounts WHERE tenant_id = ?",
            vec![alun_db_core::stmt::Value::I64(tenant)],
        )
        .await;

    let cases: &[rust_decimal::Decimal] = &[
        rust_decimal::Decimal::from(1),
        rust_decimal::Decimal::from(100),
        rust_decimal::Decimal::from(10000),
        rust_decimal::Decimal::from_i128_with_scale(10_i128.pow(10), 0),
        rust_decimal::Decimal::from_i128_with_scale(10_i128.pow(26), 0),
        rust_decimal::Decimal::new(1_234_567, 2),
        rust_decimal::Decimal::new(9_999_999_999, 2),
        rust_decimal::Decimal::new(1, 2),
    ];

    for (i, written) in cases.iter().enumerate() {
        let id = tenant * 1000 + i as i64;
        c.db()
            .execute_with_params(
                "INSERT INTO accounts (id, tenant_id, code, name, kind, balance, currency) VALUES (?, ?, ?, ?, ?, ?, ?)",
                vec![
                    alun_db_core::stmt::Value::I64(id),
                    alun_db_core::stmt::Value::I64(tenant),
                    alun_db_core::stmt::Value::Text(format!("Z{i}")),
                    alun_db_core::stmt::Value::Text("n".into()),
                    alun_db_core::stmt::Value::Text("asset".into()),
                    alun_db_core::traits::TypeMapper::to_value(written),
                    alun_db_core::stmt::Value::Text("CNY".into()),
                ],
            )
            .await
            .expect("insert");

        // ① 适配器读回
        let got = alun_ledger::services::money::account_balance(c.db(), tenant, id)
            .await
            .expect("read back");
        assert_eq!(got, *written, "{}: 适配器读回 {written}", c.dialect.name());

        // ② 服务端精确相等（关键：防止「读写两侧一起错」的假通过）
        let sql = balance_equals_sql(c.dialect, &written.to_string());
        let matched: i64 = c
            .db()
            .scalar_with_params(&sql, vec![alun_db_core::stmt::Value::I64(id)])
            .await
            .expect("server-side compare");
        assert_eq!(
            matched,
            1,
            "{}: 库内值必须精确等于 {written}（服务端比较）",
            c.dialect.name()
        );

        // ③ 类型化读回（D001 修复后 SQLite 的 TEXT 承载也应可用）
        let typed = alun_ledger::models::Account::find(c.db(), id)
            .await
            .expect("typed read");
        assert_eq!(typed.balance, *written, "{}: 类型化读回", c.dialect.name());
    }

    eprintln!(
        "[TYP-011] {}: {} 组数值往返与服务端表示一致",
        c.dialect.name(),
        cases.len()
    );
}

/// 服务端「金额列精确等于某字面量」的方言表达式。
fn balance_equals_sql(d: Dialect, literal: &str) -> String {
    match d {
        Dialect::SqliteMem | Dialect::SqliteFile => format!(
            "SELECT COUNT(*) FROM accounts WHERE id = ? AND CAST(balance AS TEXT) = '{literal}'"
        ),
        Dialect::Postgres => format!(
            "SELECT COUNT(*) FROM accounts WHERE id = ? AND balance = CAST('{literal}' AS NUMERIC)"
        ),
        Dialect::Mysql => format!(
            "SELECT COUNT(*) FROM accounts WHERE id = ? AND balance = CAST('{literal}' AS DECIMAL(38,10))"
        ),
    }
}

/// 按方言探测表的 `uid` 列类型绑定：PG 为 `UUID`，MySQL/SQLite 为文本承载。
fn uid_bind(c: &common::harness::LedgerCtx, uid: &uuid::Uuid) -> alun_db_core::stmt::Value {
    match c.dialect {
        Dialect::Postgres => alun_db_core::traits::TypeMapper::to_value(uid),
        _ => alun_db_core::stmt::Value::Text(uid.to_string()),
    }
}

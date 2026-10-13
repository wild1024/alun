//! 迁移记录表：建表、读取、写入与内容指纹。
//!
//! ## 表结构（无兼容包袱，一步到位重建）
//!
//! ```sql
//! schema_migrations (
//!     kind          VARCHAR(16)  NOT NULL, -- versioned | repeatable | seed | baseline
//!     name          VARCHAR(100) NOT NULL, -- 迁移名称（自然键）
//!     version       VARCHAR(64),           -- versioned/baseline 非空
//!     checksum      VARCHAR(64),           -- 内容指纹
//!     applied_at    BIGINT       NOT NULL, -- epoch 秒
//!     applied_seq   BIGINT       NOT NULL, -- 应用顺序（回滚 LIFO 依据）
//!     execution_ms  BIGINT,                -- 执行耗时（诊断）
//!     success       BOOLEAN      NOT NULL,
//!     source        VARCHAR(255),          -- 来源文件（多源排查）
//!     PRIMARY KEY (kind, name)
//! )
//! ```
//!
//! **刻意不设自增代理键**：`AUTO_INCREMENT` / `BIGSERIAL` /
//! `INTEGER PRIMARY KEY AUTOINCREMENT` 三者语法不同，引入代理键就必须为三方言
//! 各写一份建表 DDL。`(kind, name)` 已能唯一定位一条迁移，排序交给 `applied_seq`。
//!
//! ## 内容指纹
//!
//! `checksum` 保存 `up` 脚本的 SHA-256。每次运行都与库内记录比对，不一致即
//! [`DbError::MigrationChecksum`]——**已应用**迁移被改写属人工介入场景，绝不静默跳过。
//! 历史行（旧框架导入）没有指纹，首次运行时补录。

use alun_db_core::error::{DbError, Result};
use alun_db_core::stmt::Value;
use alun_db_core::traits::TypeMapper;

use crate::db::Db;
use crate::tx::Transaction;

use super::kind::{AppliedVia, MigrationKind};
use super::version::Version;

/// 记录已应用迁移的表。
pub(crate) const MIGRATIONS_TABLE: &str = "schema_migrations";

/// 版本列索引（`status` / `plan` 按版本扫描）。
const VERSION_INDEX: &str = "idx_schema_migrations_version";

/// 建表语句（DB 无关的类型写法，三方言通用）。
const CREATE_TABLE_SQL: &str = "\
CREATE TABLE IF NOT EXISTS schema_migrations (
    kind         VARCHAR(16)  NOT NULL,
    name         VARCHAR(100) NOT NULL,
    version      VARCHAR(64),
    checksum     VARCHAR(64),
    applied_via  VARCHAR(16)  NOT NULL,
    applied_at   BIGINT       NOT NULL,
    applied_seq  BIGINT       NOT NULL,
    execution_ms BIGINT,
    success      BOOLEAN      NOT NULL,
    source       VARCHAR(255),
    PRIMARY KEY (kind, name)
)";

/// 建索引语句（单独执行，MySQL 不支持 `CREATE INDEX IF NOT EXISTS`）。
const CREATE_VERSION_INDEX_SQL: &str =
    "CREATE INDEX idx_schema_migrations_version ON schema_migrations(version)";

/// `schema_migrations` 中的一条已应用记录。
#[derive(Debug, Clone)]
pub(crate) struct AppliedRecord {
    /// 迁移种类。
    pub kind: MigrationKind,
    /// 迁移名称。
    pub name: String,
    /// 版本号；可重复 / 种子类为 `None`。
    pub version: Option<Version>,
    /// 内容指纹；历史行 / 旧框架导入的行初次写入时没有指纹（`None`）。
    pub checksum: Option<String>,
    /// 该行的 `up` 由谁执行（决定能否安全回滚）。
    pub applied_via: AppliedVia,
    /// 应用序号：越大越新，回滚按它取最近一次应用。
    pub seq: i64,
    /// 上次执行是否成功。
    pub is_success: bool,
}

/// 已应用记录表，以 `(kind, name)` 为键。
pub(crate) type AppliedMap = std::collections::HashMap<(MigrationKind, String), AppliedRecord>;

/// 一条待写入记录的字段集合。
#[derive(Debug, Clone)]
pub(crate) struct RecordWrite {
    /// 迁移种类。
    pub kind: MigrationKind,
    /// 迁移名称。
    pub name: String,
    /// 版本号。
    pub version: Option<Version>,
    /// 内容指纹。
    pub checksum: Option<String>,
    /// 该行的 `up` 由谁执行。
    pub applied_via: AppliedVia,
    /// 应用序号。
    pub seq: i64,
    /// 执行耗时（毫秒）；失败或未知时为 `None`。
    pub execution_ms: Option<i64>,
    /// 执行是否成功。
    pub is_success: bool,
    /// 来源文件路径。
    pub source: Option<String>,
}

/// 确保记录表就绪：建表 → 建版本索引 → 按需导入旧框架存量记录。
///
/// # 参数
///
/// - `db`：数据库句柄。
/// - `is_import_legacy`：是否导入旧框架 `_sqlx_migrations`。为 `false` 时
///   **完全不探测**旧表（避免在生产库上产生对既有表的存在性查询）。
///
/// # 错误
///
/// 建表、建索引、计数或旧框架导入失败时返回 [`DbError`]。
pub(crate) async fn ensure_ready(db: &Db, is_import_legacy: bool) -> Result<()> {
    db.execute(CREATE_TABLE_SQL).await?;
    ensure_version_index(db).await?;

    if !is_import_legacy {
        return Ok(());
    }

    // 新表已有记录说明已初始化过，无需重复导入旧框架历史
    let count: i64 = db
        .scalar_with_params(&format!("SELECT COUNT(*) FROM {MIGRATIONS_TABLE}"), vec![])
        .await?;
    if count > 0 {
        return Ok(());
    }
    super::compat::import_legacy_sqlx_migrations(db)
        .await
        .map(|_| ())
}

/// 为版本列建索引（先探测后建，保证幂等且兼容不支持 `IF NOT EXISTS` 的方言）。
async fn ensure_version_index(db: &Db) -> Result<()> {
    if index_exists(db).await? {
        return Ok(());
    }
    db.execute(CREATE_VERSION_INDEX_SQL).await?;
    Ok(())
}

/// 判断版本索引是否已存在（各方言用各自的元数据查询）。
async fn index_exists(db: &Db) -> Result<bool> {
    let sql = match db.dialect().name() {
        "sqlite" => format!(
            "SELECT name FROM pragma_index_list('{MIGRATIONS_TABLE}') WHERE name = '{VERSION_INDEX}'"
        ),
        "mysql" => format!(
            "SELECT index_name FROM information_schema.statistics \
             WHERE table_schema = DATABASE() AND table_name = '{MIGRATIONS_TABLE}' \
             AND index_name = '{VERSION_INDEX}'"
        ),
        _ => format!(
            "SELECT indexname FROM pg_indexes \
             WHERE schemaname = current_schema() AND tablename = '{MIGRATIONS_TABLE}' \
             AND indexname = '{VERSION_INDEX}'"
        ),
    };
    Ok(!db.find_with_params(&sql, vec![]).await?.is_empty())
}

/// 读取全部已应用记录（以 `(kind, name)` 为键）。
///
/// # 错误
///
/// 查询失败、`kind` 列取值无法识别、或列类型不符时返回 [`DbError`]。
pub(crate) async fn load_applied(db: &Db) -> Result<AppliedMap> {
    let rows = db
        .find_with_params(
            &format!(
                "SELECT kind, name, version, checksum, applied_via, applied_seq, success \
                 FROM {MIGRATIONS_TABLE}"
            ),
            vec![],
        )
        .await?;

    let mut records = AppliedMap::with_capacity(rows.len());
    for row in rows {
        let kind_text = String::from_value(
            row.get("kind")
                .ok_or_else(|| DbError::ColumnNotFound("kind".into()))?,
        )?;
        let kind = MigrationKind::from_db_str(&kind_text).ok_or_else(|| {
            DbError::Migration(format!(
                "unknown migration kind '{kind_text}' in record table"
            ))
        })?;
        let name = String::from_value(
            row.get("name")
                .ok_or_else(|| DbError::ColumnNotFound("name".into()))?,
        )?;
        let version = match row.get("version") {
            Some(Value::Null) | None => None,
            Some(value) => Some(Version::new(String::from_value(value)?)),
        };
        let checksum = match row.get("checksum") {
            Some(Value::Null) | None => None,
            Some(value) => Some(String::from_value(value)?),
        };
        // 历史行可能没有 applied_seq（NULL）→ 视为最旧
        let seq = match row.get("applied_seq") {
            Some(Value::Null) | None => 0,
            Some(value) => i64::from_value(value)?,
        };
        let applied_via_text = String::from_value(
            row.get("applied_via")
                .ok_or_else(|| DbError::ColumnNotFound("applied_via".into()))?,
        )?;
        let applied_via = AppliedVia::from_db_str(&applied_via_text).ok_or_else(|| {
            DbError::Migration(format!(
                "unknown applied_via '{applied_via_text}' in record table"
            ))
        })?;
        let is_success = match row.get("success") {
            Some(value) => value_to_bool(value)?,
            None => true,
        };

        records.insert(
            (kind, name.clone()),
            AppliedRecord {
                kind,
                name,
                version,
                checksum,
                applied_via,
                seq,
                is_success,
            },
        );
    }
    Ok(records)
}

/// 在事务内写入（或覆盖）一条记录。
///
/// 采用「先删后插」而非方言各异的 UPSERT 语法（`ON CONFLICT` / `ON DUPLICATE KEY`），
/// 保证三方言通用；常规应用路径下删除是无操作。
pub(crate) async fn upsert_in_tx(tx: &Transaction, record: &RecordWrite) -> Result<()> {
    let params = vec![
        Value::Text(record.kind.as_str().to_string()),
        Value::Text(truncate_name(&record.name)),
    ];
    tx.execute_with_params(
        &format!("DELETE FROM {MIGRATIONS_TABLE} WHERE kind = ? AND name = ?"),
        params,
    )
    .await?;

    let version = record
        .version
        .as_ref()
        .map(|v| Value::Text(v.as_str().to_string()))
        .unwrap_or(Value::Null);
    let checksum = record
        .checksum
        .as_ref()
        .map(|c| Value::Text(c.clone()))
        .unwrap_or(Value::Null);
    let execution_ms = record.execution_ms.map(Value::I64).unwrap_or(Value::Null);
    let source = record
        .source
        .as_ref()
        .map(|s| Value::Text(truncate_source(s)))
        .unwrap_or(Value::Null);

    tx.execute_with_params(
        &format!(
            "INSERT INTO {MIGRATIONS_TABLE} \
             (kind, name, version, checksum, applied_via, applied_at, applied_seq, execution_ms, success, source) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        ),
        vec![
            Value::Text(record.kind.as_str().to_string()),
            Value::Text(truncate_name(&record.name)),
            version,
            checksum,
            Value::Text(record.applied_via.as_str().to_string()),
            Value::I64(now_epoch_secs()),
            Value::I64(record.seq),
            execution_ms,
            Value::Bool(record.is_success),
            source,
        ],
    )
    .await?;
    Ok(())
}

/// 删除一条记录（回滚用；`repair` 清理已删除的可重复 / 种子记录时复用）。
pub(crate) async fn delete_record(db: &Db, kind: MigrationKind, name: &str) -> Result<()> {
    let (sql, params) = delete_statement(kind, name);
    db.execute_with_params(&sql, params).await?;
    Ok(())
}

/// 事务内删除一条记录。
pub(crate) async fn delete_in_tx(tx: &Transaction, kind: MigrationKind, name: &str) -> Result<()> {
    let (sql, params) = delete_statement(kind, name);
    tx.execute_with_params(&sql, params).await?;
    Ok(())
}

/// 渲染「按 `(kind, name)` 删除一条记录」的语句与参数。
fn delete_statement(kind: MigrationKind, name: &str) -> (String, Vec<Value>) {
    (
        format!("DELETE FROM {MIGRATIONS_TABLE} WHERE kind = ? AND name = ?"),
        vec![
            Value::Text(kind.as_str().to_string()),
            Value::Text(truncate_name(name)),
        ],
    )
}

/// 删除全部 `success = false` 的失败记录（`repair` 用），返回删除行数。
pub(crate) async fn delete_unsuccessful(db: &Db) -> Result<u64> {
    db.execute_with_params(
        &format!("DELETE FROM {MIGRATIONS_TABLE} WHERE success = ?"),
        vec![Value::Bool(false)],
    )
    .await
}

/// 覆盖一条记录的内容指纹（补录历史行 / `repair` 接受当前内容）。
///
/// 只更新 `checksum` 列，不改动 `applied_at` / `applied_seq` 等历史字段——
/// 修指纹是对既有事实的补充，不得重写历史顺序。
pub(crate) async fn set_checksum(
    db: &Db,
    kind: MigrationKind,
    name: &str,
    checksum: &str,
) -> Result<()> {
    db.execute_with_params(
        &format!("UPDATE {MIGRATIONS_TABLE} SET checksum = ? WHERE kind = ? AND name = ?"),
        vec![
            Value::Text(checksum.to_string()),
            Value::Text(kind.as_str().to_string()),
            Value::Text(truncate_name(name)),
        ],
    )
    .await?;
    Ok(())
}

/// 下一个应用序号（当前最大序号 + 1；空表从 1 开始）。
pub(crate) fn next_seq(applied: &AppliedMap) -> i64 {
    applied.values().map(|r| r.seq).max().unwrap_or(0) + 1
}

/// 判断错误是否为「表不存在」。
///
/// 用于存量兼容与 `status` / `plan` 的裸库探测：跨库时驱动返回的表缺失错误
/// 形式不同，且常被驱动再包一层（PostgreSQL / MySQL 把真实错误包成 `db error`），
/// 故必须遍历**整条** `source` 链比对特征串：
/// - SQLite：`no such table`
/// - PostgreSQL：`relation "x" does not exist`（SQLSTATE `42P01`）
/// - MySQL：`Table 'db.x' doesn't exist`（errno `1146`）
pub(crate) fn is_missing_table(err: &DbError) -> bool {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(source) = current {
        if looks_like_missing_table(&source.to_string()) {
            return true;
        }
        current = source.source();
    }
    false
}

/// 「表不存在」的特征串判定（大小写不敏感）。
fn looks_like_missing_table(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("no such table")
        || message.contains("doesn't exist")
        || message.contains("42p01")
        || message.contains("1146")
        // `column ... does not exist`（42703）不得算表缺失：要求同时出现 relation/table
        || (message.contains("does not exist")
            && (message.contains("relation") || message.contains("table")))
}

/// 把迁移名称截断到 `name` 列宽（`VARCHAR(100)`），按字符而非字节计。
fn truncate_name(name: &str) -> String {
    name.chars().take(100).collect()
}

/// 把来源路径截断到 `source` 列宽（`VARCHAR(255)`）。
fn truncate_source(source: &str) -> String {
    source.chars().take(255).collect()
}

/// 当前 UTC 距 Unix 纪元的整秒数。
fn now_epoch_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 把驱动的布尔承载（各协议可能是布尔 / 整数 / 文本）统一解释为布尔。
fn value_to_bool(value: &Value) -> Result<bool> {
    match value {
        Value::Bool(v) => Ok(*v),
        Value::I8(v) => Ok(*v != 0),
        Value::I16(v) => Ok(*v != 0),
        Value::I32(v) => Ok(*v != 0),
        Value::I64(v) => Ok(*v != 0),
        Value::U8(v) => Ok(*v != 0),
        Value::U16(v) => Ok(*v != 0),
        Value::U32(v) => Ok(*v != 0),
        Value::U64(v) => Ok(*v != 0),
        Value::Text(text) => Ok(matches!(
            text.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "t"
        )),
        other => Err(DbError::TypeMismatch {
            expected: "bool",
            actual: match other {
                Value::Null => "null",
                _ => "non-boolean",
            },
        }),
    }
}

/// 迁移内容指纹（SHA-256）。
///
/// 框架不引入哈希依赖，故内置标准 SHA-256（FIPS 180-4）实现；已知答案测试见
/// 本模块单测。指纹用于发现**已应用**迁移的 `up` 脚本被改写：
/// `down` 脚本不参与指纹（允许为已应用的单向迁移补写回退脚本）。
pub(crate) mod checksum {
    /// 计算内容的十六进制 SHA-256（小写）。
    ///
    /// 换行统一为 `\n`（逐行哈希），使 CRLF/LF 差异与文件末尾换行数量不影响
    /// 指纹——指纹只反映**语义内容**，避免跨平台误报篡改。
    pub fn content_checksum(sql: &str) -> String {
        let mut normalized = String::with_capacity(sql.len() + 1);
        for line in sql.lines() {
            normalized.push_str(line);
            normalized.push('\n');
        }
        hex(&sha256(normalized.as_bytes()))
    }

    /// SHA-256 初始状态（FIPS 180-4 §5.3.3）。
    const INITIAL: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];

    /// SHA-256 轮常量（FIPS 180-4 §4.2.2，前 64 个素数的立方根小数部分）。
    const ROUND: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    /// 计算 SHA-256 摘要。
    fn sha256(data: &[u8]) -> [u8; 32] {
        // 填充：0x80 + 若干 0，使长度 ≡ 56 (mod 64)，再附 8 字节大端位长
        let mut padded = Vec::with_capacity(data.len() + 72);
        padded.extend_from_slice(data);
        padded.push(0x80);
        while padded.len() % 64 != 56 {
            padded.push(0);
        }
        padded.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_be_bytes());

        let mut state = INITIAL;
        for block in padded.chunks_exact(64) {
            let mut w = [0_u32; 64];
            for (i, word) in block.chunks_exact(4).enumerate() {
                w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }

            let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let temp1 = h
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(ROUND[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let temp2 = s0.wrapping_add(maj);

                h = g;
                g = f;
                f = e;
                e = d.wrapping_add(temp1);
                d = c;
                c = b;
                b = a;
                a = temp1.wrapping_add(temp2);
            }

            for (slot, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
                *slot = slot.wrapping_add(value);
            }
        }

        let mut digest = [0_u8; 32];
        for (chunk, word) in digest.chunks_exact_mut(4).zip(state.iter()) {
            chunk.copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    /// 十六进制编码（小写）。
    fn hex(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(HEX[usize::from(byte >> 4)] as char);
            out.push(HEX[usize::from(byte & 0x0f)] as char);
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// FIPS 180-4 已知答案向量。
        #[test]
        fn sha256_matches_known_vectors() {
            assert_eq!(
                hex(&sha256(b"")),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            );
            assert_eq!(
                hex(&sha256(b"abc")),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            );
            assert_eq!(
                hex(&sha256(
                    b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
                )),
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
            );
        }

        /// 指纹对换行风格不敏感，但内容改变必然改变指纹。
        #[test]
        fn checksum_normalizes_newlines_and_detects_changes() {
            let lf = content_checksum("CREATE TABLE t (id BIGINT);\n");
            let crlf = content_checksum("CREATE TABLE t (id BIGINT);\r\n");
            let no_final_newline = content_checksum("CREATE TABLE t (id BIGINT);");
            assert_eq!(lf, crlf);
            assert_eq!(lf, no_final_newline);
            assert_eq!(lf.len(), 64);
            assert_ne!(lf, content_checksum("DROP TABLE t;"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 布尔承载的跨协议解释：整数 / 文本 / 布尔都要能判定。
    #[test]
    fn interprets_boolean_carriers() {
        assert!(value_to_bool(&Value::Bool(true)).expect("bool"));
        assert!(value_to_bool(&Value::I64(1)).expect("i64"));
        assert!(value_to_bool(&Value::Text("TRUE".into())).expect("text"));
        assert!(!value_to_bool(&Value::I64(0)).expect("i64 zero"));
        assert!(!value_to_bool(&Value::Text("f".into())).expect("text false"));
    }

    /// 应用序号从当前最大值递增。
    #[test]
    fn computes_next_seq() {
        let mut applied = AppliedMap::new();
        assert_eq!(next_seq(&applied), 1);
        applied.insert(
            (MigrationKind::Versioned, "a".into()),
            AppliedRecord {
                kind: MigrationKind::Versioned,
                name: "a".into(),
                version: Some(Version::new("1")),
                checksum: None,
                applied_via: AppliedVia::Executed,
                seq: 7,
                is_success: true,
            },
        );
        assert_eq!(next_seq(&applied), 8);
    }
}

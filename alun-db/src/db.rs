/// Db 门面 —— 统一数据库访问入口
///
/// 所有操作通过 Db 统一入口，内部按数据库类型分发。

use sqlx::{PgPool, MySqlPool, SqlitePool, AnyPool, Column, Row as SqlxRow};
use crate::{Row, DbResult, DbError, IdKind};
use alun_core::PageQuery;
use serde_json::{Value, Number};

/// 抽象数据库连接池 —— 统一封装四种后端连接池
///
/// 通过 `factory::create_db()` 根据配置自动选择对应的变体创建。
#[derive(Clone)]
pub enum DbPool {
    /// PostgreSQL 连接池（`sqlx::PgPool`）
    Postgres(PgPool),
    /// MySQL 连接池（`sqlx::MySqlPool`）
    Mysql(MySqlPool),
    /// SQLite 连接池（`sqlx::SqlitePool`）
    Sqlite(SqlitePool),
    /// 运行时确定数据库类型的通用连接池（`sqlx::AnyPool`）
    Any(AnyPool),
}

/// Db 门面 —— 配置驱动的数据库访问入口
///
/// 封装 PostgreSQL/MySQL/SQLite 三种后端，提供统一的 CRUD 接口。
/// 通过 `factory::create_db()` 从 `DatabaseConfig` 自动创建并连接测试。
///
/// # 示例
///
/// ```ignore
/// let db = create_db(&config.database).await?;
/// let user = db.find_by_id("user", 1).await?;
/// ```
#[derive(Clone)]
pub struct Db {
    /// 数据库连接池
    pool: DbPool,
}

// ── 每个数据库类型的查询/写入实现宏 ──

/// 为指定数据库类型生成后端查询/写入函数，包含列级 Row 转换
macro_rules! impl_db_ops {
    ($pool_ty:ty, $db_mod:ident) => {
        paste::paste! {
            fn [<typed_row_to_row_ $db_mod:snake>](
                row: &<sqlx::$db_mod as sqlx::Database>::Row
            ) -> Row {
                use chrono::{DateTime, Utc};

                let mut r = Row::default();
                for col in row.columns() {
                    let name = col.name().to_string();
                    let idx: usize = col.ordinal();
                    if let Ok(v) = row.try_get::<i64, usize>(idx) {
                        r.data.insert(name, Value::Number(v.into()));
                    } else if let Ok(v) = row.try_get::<i32, usize>(idx) {
                        r.data.insert(name, Value::Number((v as i64).into()));
                    } else if let Ok(v) = row.try_get::<i16, usize>(idx) {
                        r.data.insert(name, Value::Number((v as i64).into()));
                    } else if let Ok(v) = row.try_get::<String, usize>(idx) {
                        r.data.insert(name, Value::String(v));
                    } else if let Ok(v) = row.try_get::<sqlx::types::Uuid, usize>(idx) {
                        r.data.insert(name, Value::String(v.to_string()));
                    } else if let Ok(v) = row.try_get::<DateTime<Utc>, usize>(idx) {
                        r.data.insert(name, Value::String(v.to_rfc3339()));
                    } else if let Ok(v) = row.try_get::<f64, usize>(idx) {
                        if let Some(n) = Number::from_f64(v) {
                            r.data.insert(name, Value::Number(n));
                        }
                    } else if let Ok(v) = row.try_get::<sqlx::types::BigDecimal, usize>(idx) {
                        let s = v.to_string();
                        if let Ok(n) = s.parse::<serde_json::Number>() {
                            r.data.insert(name, Value::Number(n));
                        }
                    } else if let Ok(v) = row.try_get::<bool, usize>(idx) {
                        r.data.insert(name, Value::Bool(v));
                    } else if let Ok(v) = row.try_get::<serde_json::Value, usize>(idx) {
                        r.data.insert(name, v);
                    }
                }
                r
            }

            async fn [<query_one_ $pool_ty:snake>](
                pool: &$pool_ty, sql: &str, params: &[&str],
            ) -> DbResult<Option<Row>> {
                let mut q = sqlx::query::<sqlx::$db_mod>(sql);
                for p in params { q = q.bind(*p); }
                Ok(q.fetch_optional(pool).await?.as_ref()
                    .map([<typed_row_to_row_ $db_mod:snake>]))
            }

            async fn [<query_all_ $pool_ty:snake>](
                pool: &$pool_ty, sql: &str, params: &[&str],
            ) -> DbResult<Vec<Row>> {
                let mut q = sqlx::query::<sqlx::$db_mod>(sql);
                for p in params { q = q.bind(*p); }
                let rows = q.fetch_all(pool).await?;
                Ok(rows.iter().map([<typed_row_to_row_ $db_mod:snake>]).collect())
            }

            async fn [<count_ $pool_ty:snake>](
                pool: &$pool_ty, sql: &str, params: &[&str],
            ) -> DbResult<u64> {
                let mut q = sqlx::query_scalar::<sqlx::$db_mod, i64>(sql);
                for p in params { q = q.bind(*p); }
                Ok(q.fetch_optional(pool).await?.unwrap_or(0) as u64)
            }

            async fn [<execute_ $pool_ty:snake>](
                pool: &$pool_ty, sql: &str, params: &[&str],
            ) -> DbResult<u64> {
                let mut q = sqlx::query::<sqlx::$db_mod>(sql);
                for p in params { q = q.bind(*p); }
                q.execute(pool).await.map_err(DbError::from).map(|r| r.rows_affected())
            }
        }
    };
}

/// 为不支持 BigDecimal 的数据库类型（SQLite）生成后端查询/写入函数
macro_rules! impl_db_ops_no_bigdecimal {
    ($pool_ty:ty, $db_mod:ident) => {
        paste::paste! {
            fn [<typed_row_to_row_ $db_mod:snake>](
                row: &<sqlx::$db_mod as sqlx::Database>::Row
            ) -> Row {
                use chrono::{DateTime, Utc};

                let mut r = Row::default();
                for col in row.columns() {
                    let name = col.name().to_string();
                    let idx: usize = col.ordinal();
                    if let Ok(v) = row.try_get::<i64, usize>(idx) {
                        r.data.insert(name, Value::Number(v.into()));
                    } else if let Ok(v) = row.try_get::<i32, usize>(idx) {
                        r.data.insert(name, Value::Number((v as i64).into()));
                    } else if let Ok(v) = row.try_get::<i16, usize>(idx) {
                        r.data.insert(name, Value::Number((v as i64).into()));
                    } else if let Ok(v) = row.try_get::<String, usize>(idx) {
                        r.data.insert(name, Value::String(v));
                    } else if let Ok(v) = row.try_get::<sqlx::types::Uuid, usize>(idx) {
                        r.data.insert(name, Value::String(v.to_string()));
                    } else if let Ok(v) = row.try_get::<DateTime<Utc>, usize>(idx) {
                        r.data.insert(name, Value::String(v.to_rfc3339()));
                    } else if let Ok(v) = row.try_get::<f64, usize>(idx) {
                        if let Some(n) = Number::from_f64(v) {
                            r.data.insert(name, Value::Number(n));
                        }
                    } else if let Ok(v) = row.try_get::<bool, usize>(idx) {
                        r.data.insert(name, Value::Bool(v));
                    } else if let Ok(v) = row.try_get::<serde_json::Value, usize>(idx) {
                        r.data.insert(name, v);
                    }
                }
                r
            }

            async fn [<query_one_ $pool_ty:snake>](
                pool: &$pool_ty, sql: &str, params: &[&str],
            ) -> DbResult<Option<Row>> {
                let mut q = sqlx::query::<sqlx::$db_mod>(sql);
                for p in params { q = q.bind(*p); }
                Ok(q.fetch_optional(pool).await?.as_ref()
                    .map([<typed_row_to_row_ $db_mod:snake>]))
            }

            async fn [<query_all_ $pool_ty:snake>](
                pool: &$pool_ty, sql: &str, params: &[&str],
            ) -> DbResult<Vec<Row>> {
                let mut q = sqlx::query::<sqlx::$db_mod>(sql);
                for p in params { q = q.bind(*p); }
                let rows = q.fetch_all(pool).await?;
                Ok(rows.iter().map([<typed_row_to_row_ $db_mod:snake>]).collect())
            }

            async fn [<count_ $pool_ty:snake>](
                pool: &$pool_ty, sql: &str, params: &[&str],
            ) -> DbResult<u64> {
                let mut q = sqlx::query_scalar::<sqlx::$db_mod, i64>(sql);
                for p in params { q = q.bind(*p); }
                Ok(q.fetch_optional(pool).await?.unwrap_or(0) as u64)
            }

            async fn [<execute_ $pool_ty:snake>](
                pool: &$pool_ty, sql: &str, params: &[&str],
            ) -> DbResult<u64> {
                let mut q = sqlx::query::<sqlx::$db_mod>(sql);
                for p in params { q = q.bind(*p); }
                q.execute(pool).await.map_err(DbError::from).map(|r| r.rows_affected())
            }
        }
    };
}

impl_db_ops!(PgPool, Postgres);
impl_db_ops!(MySqlPool, MySql);
// SQLite 不支持 BigDecimal 类型，使用不含 BigDecimal 处理的变体
impl_db_ops_no_bigdecimal!(SqlitePool, Sqlite);

async fn query_one_any(pool: &AnyPool, sql: &str, params: &[&str]) -> DbResult<Option<Row>> {
    let mut q = sqlx::query(sql);
    for p in params { q = q.bind(*p); }
    Ok(q.fetch_optional(pool).await?.as_ref().map(typed_row_to_row_any))
}

async fn query_all_any(pool: &AnyPool, sql: &str, params: &[&str]) -> DbResult<Vec<Row>> {
    let mut q = sqlx::query(sql);
    for p in params { q = q.bind(*p); }
    let rows = q.fetch_all(pool).await?;
    Ok(rows.iter().map(typed_row_to_row_any).collect())
}

fn typed_row_to_row_any(row: &sqlx::any::AnyRow) -> Row {
    let mut r = Row::default();
    for col in row.columns() {
        let name = col.name().to_string();
        let idx: usize = col.ordinal();
        if let Ok(v) = row.try_get::<i64, usize>(idx) {
            r.data.insert(name, Value::Number(v.into()));
        } else if let Ok(v) = row.try_get::<i32, usize>(idx) {
            r.data.insert(name, Value::Number((v as i64).into()));
        } else if let Ok(v) = row.try_get::<String, usize>(idx) {
            r.data.insert(name, Value::String(v));
        } else if let Ok(v) = row.try_get::<f64, usize>(idx) {
            if let Some(n) = Number::from_f64(v) {
                r.data.insert(name, Value::Number(n));
            }
        } else if let Ok(v) = row.try_get::<bool, usize>(idx) {
            r.data.insert(name, Value::Bool(v));
        }
    }
    r
}

async fn count_any(pool: &AnyPool, sql: &str, params: &[&str]) -> DbResult<u64> {
    let mut q = sqlx::query_scalar::<sqlx::Any, i64>(sql);
    for p in params { q = q.bind(*p); }
    Ok(q.fetch_optional(pool).await?.unwrap_or(0) as u64)
}

async fn execute_any(pool: &AnyPool, sql: &str, params: &[&str]) -> DbResult<u64> {
    let mut q = sqlx::query(sql);
    for p in params { q = q.bind(*p); }
    Ok(q.execute(pool).await.map_err(DbError::from)?.rows_affected())
}

impl Db {
    /// 从 PostgreSQL 连接池创建 Db 实例
    pub fn postgres(pool: PgPool) -> Self { Self { pool: DbPool::Postgres(pool) } }
    /// 从 MySQL 连接池创建 Db 实例
    pub fn mysql(pool: MySqlPool) -> Self { Self { pool: DbPool::Mysql(pool) } }
    /// 从 SQLite 连接池创建 Db 实例
    pub fn sqlite(pool: SqlitePool) -> Self { Self { pool: DbPool::Sqlite(pool) } }

    /// 获取 PostgreSQL 连接池引用（非 PG 则 panic）
    pub fn pg_pool(&self) -> &PgPool { match &self.pool { DbPool::Postgres(p) => p, _ => panic!("不是 PG"), } }
    /// 获取 MySQL 连接池引用（非 MySQL 则 panic）
    pub fn mysql_pool(&self) -> &MySqlPool { match &self.pool { DbPool::Mysql(p) => p, _ => panic!("不是 MySQL"), } }
    /// 获取 SQLite 连接池引用（非 SQLite 则 panic）
    pub fn sqlite_pool(&self) -> &SqlitePool { match &self.pool { DbPool::Sqlite(p) => p, _ => panic!("不是 SQLite"), } }

    // ── 查询 ─────────────────────────────────────────

    /// 按主键 ID 查询单条记录（主键默认 `id`）
    ///
    /// 自动识别 ID 类型（UUID / 整数 / 字符串），添加正确的 SQL 类型转换后缀。
    ///
    /// # 参数
    ///
    /// - `table`: 表名
    /// - `id`: 主键值（支持 `i64`、`&str`、`Uuid` 等实现了 `Into<Value>` 的类型）
    ///
    /// # 返回值
    ///
    /// - `Ok(Some(row))`: 记录存在
    /// - `Ok(None)`: 记录不存在
    /// - `Err(_)`: 数据库错误
    pub async fn find_by_id(&self, table: &str, id: impl Into<serde_json::Value>) -> DbResult<Option<Row>> {
        let value: serde_json::Value = id.into();
        let pk = "id";
        let id_str = value_to_string(&value);
        let sql = format!("SELECT * FROM {} WHERE {}=$1{}", table, pk, id_cast(&value));
        let params = vec![id_str.as_str()];
        self.query_one(&sql, &params).await
    }

    /// 执行原始 SQL 查询（单条），返回 `Option<Row>`
    ///
    /// 参数使用 `$1`、`$2` 占位符，按顺序绑定。
    pub async fn query_one(&self, sql: &str, params: &[&str]) -> DbResult<Option<Row>> {
        match &self.pool {
            DbPool::Postgres(pool) => query_one_pg_pool(pool, sql, params).await,
            DbPool::Mysql(pool)    => query_one_my_sql_pool(pool, sql, params).await,
            DbPool::Sqlite(pool)   => query_one_sqlite_pool(pool, sql, params).await,
            DbPool::Any(pool)      => query_one_any(pool, sql, params).await,
        }
    }

    /// 执行原始 SQL 查询（多条），返回 `Vec<Row>`
    pub async fn query(&self, sql: &str, params: &[&str]) -> DbResult<Vec<Row>> {
        match &self.pool {
            DbPool::Postgres(pool) => query_all_pg_pool(pool, sql, params).await,
            DbPool::Mysql(pool)    => query_all_my_sql_pool(pool, sql, params).await,
            DbPool::Sqlite(pool)   => query_all_sqlite_pool(pool, sql, params).await,
            DbPool::Any(pool)      => query_all_any(pool, sql, params).await,
        }
    }

    /// 分页查询：自动包裹 COUNT 和 LIMIT/OFFSET
    ///
    /// 返回 `(数据列表, 总条数)`。传入的 SQL 应为无 LIMIT/OFFSET 的完整查询。
    pub async fn query_page(&self, sql: &str, params: &[&str], page: &PageQuery) -> DbResult<(Vec<Row>, u64)> {
        let count_sql = format!("SELECT COUNT(*) as cnt FROM ({}) AS _count_sub", sql);
        let total = self.count(&count_sql, params).await?;
        let page_sql = format!("{} LIMIT {} OFFSET {}", sql, page.limit(), page.offset());
        let rows = self.query(&page_sql, params).await?;
        Ok((rows, total))
    }

    /// 执行 COUNT 查询，返回行数（自动转换为 u64）
    pub async fn count(&self, sql: &str, params: &[&str]) -> DbResult<u64> {
        match &self.pool {
            DbPool::Postgres(pool) => count_pg_pool(pool, sql, params).await,
            DbPool::Mysql(pool)    => count_my_sql_pool(pool, sql, params).await,
            DbPool::Sqlite(pool)   => count_sqlite_pool(pool, sql, params).await,
            DbPool::Any(pool)      => count_any(pool, sql, params).await,
        }
    }

    // ── 增删改 ───────────────────────────────────────

    /// 插入单条记录（Row 需设置 `table` 和字段值）
    ///
    /// PostgreSQL 使用 `RETURNING *` 直接返回插入后的完整行；
    /// MySQL/SQLite 插入后通过主键回查。
    ///
    /// # 错误
    ///
    /// - Row 缺少表名 → `Argument` 错误
    /// - 没有变更字段 → `Argument` 错误
    pub async fn insert(&self, row: &Row) -> DbResult<Row> {
        let table = row.table.as_deref().ok_or(DbError::Argument("Row 缺少表名".into()))?;
        let columns: Vec<&String> = row.changes.iter().collect();
        if columns.is_empty() { return Err(DbError::Argument("没有变更的字段".into())); }

        let placeholders: Vec<String> = columns.iter().enumerate().map(|(i, c)| {
            let cast = row.data.get(*c).map(|v| value_cast(v)).unwrap_or("");
            format!("${}{}", i + 1, cast)
        }).collect();
        let col_str = columns.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ");
        let values: Vec<String> = columns.iter()
            .filter_map(|c| row.data.get(*c)).map(value_to_string).collect();
        let val_refs: Vec<&str> = values.iter().map(|s| s.as_str()).collect();

        if matches!(&self.pool, DbPool::Postgres(_)) {
            let sql = format!("INSERT INTO {} ({}) VALUES ({}) RETURNING *", table, col_str, placeholders.join(", "));
            self.query_one(&sql, &val_refs).await?.ok_or_else(|| DbError::Other("INSERT 返回空".into()))
        } else {
            let sql = format!("INSERT INTO {} ({}) VALUES ({})", table, col_str, placeholders.join(", "));
            self.execute(&sql, &val_refs).await?;
            let pk_val = row.data.get("id");
            match pk_val {
                Some(v) => self.find_by_id(table, v.clone()).await?.ok_or(DbError::Other("INSERT 后查不到".into())),
                None => Err(DbError::Argument("非 PG 数据库需 Row 含主键".into())),
            }
        }
    }

    /// 批量插入记录，返回受影响行数
    ///
    /// 所有 Row 必须来自同一张表且变更字段一致。
    pub async fn batch_insert(&self, rows: &[Row]) -> DbResult<u64> {
        if rows.is_empty() { return Ok(0); }
        let table = rows[0].table.as_deref().ok_or(DbError::Argument("Row 缺少表名".into()))?;
        let columns: Vec<&String> = rows[0].changes.iter().collect();
        if columns.is_empty() { return Err(DbError::Argument("没有变更的字段".into())); }

        let col_names = columns.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ");
        let mut all_params: Vec<String> = Vec::new();
        let mut groups: Vec<String> = Vec::new();
        for (ri, row) in rows.iter().enumerate() {
            let offset = ri * columns.len();
            let ph: Vec<String> = columns.iter().enumerate().map(|(ci, c)| {
                let cast = row.data.get(*c).map(|v| value_cast(v)).unwrap_or("");
                format!("${}{}", offset + ci + 1, cast)
            }).collect();
            groups.push(format!("({})", ph.join(", ")));
            for c in &columns {
                all_params.push(row.data.get(*c).map(value_to_string).unwrap_or_default());
            }
        }
        let sql = format!("INSERT INTO {} ({}) VALUES {}", table, col_names, groups.join(", "));
        let val_refs: Vec<&str> = all_params.iter().map(|s| s.as_str()).collect();
        self.execute(&sql, &val_refs).await
    }

    /// 更新单条记录（根据 Row 的 `changes` 和主键）
    ///
    /// 仅更新 `changes` 中标记的字段，主键必须存在于 `data` 中。
    /// PostgreSQL 使用 `RETURNING *` 返回更新后的行。
    ///
    /// 对 `Value::Null` 字段生成 `column = NULL`，不占用参数占位符。
    pub async fn update(&self, row: &Row) -> DbResult<Option<Row>> {
        let table = row.table.as_deref().ok_or(DbError::Argument("Row 缺少表名".into()))?;

        // 构建 SET 子句：非 null 值使用参数占位符，null 值直接写 NULL
        let mut sets: Vec<String> = Vec::with_capacity(row.changes.len());
        let mut params: Vec<String> = Vec::with_capacity(row.changes.len() + 1);
        let mut param_idx = 0usize;

        for col in &row.changes {
            if let Some(value) = row.data.get(col) {
                if value.is_null() {
                    sets.push(format!("{} = NULL", col));
                } else {
                    param_idx += 1;
                    let cast = value_cast(value);
                    sets.push(format!("{} = ${}{}", col, param_idx, cast));
                    params.push(value_to_string(value));
                }
            }
        }

        if sets.is_empty() {
            return Err(DbError::Argument("没有要更新的字段".into()));
        }

        let pk = row.primary_keys.first().map(|s| s.as_str()).unwrap_or("id");
        let id_value = row.data.get(pk).ok_or(DbError::Argument("Row 缺少主键".into()))?;

        let where_param_idx = param_idx + 1;
        params.push(value_to_string(id_value));
        let val_refs: Vec<&str> = params.iter().map(|s| s.as_str()).collect();

        let id_cast_sql = id_cast(id_value);
        if matches!(&self.pool, DbPool::Postgres(_)) {
            let sql = format!("UPDATE {} SET {} WHERE {}=${}{} RETURNING *",
                table, sets.join(", "), pk, where_param_idx, id_cast_sql);
            self.query_one(&sql, &val_refs).await
        } else {
            let sql = format!("UPDATE {} SET {} WHERE {}=${}{}",
                table, sets.join(", "), pk, where_param_idx, id_cast_sql);
            let n = self.execute(&sql, &val_refs).await?;
            if n > 0 { self.find_by_id(table, id_value.clone()).await } else { Ok(None) }
        }
    }

    /// 批量更新（按 WHERE 条件），返回受影响行数
    ///
    /// - `sets`: 要更新的字段值（Row 含 changes）
    /// - `where_sql`: WHERE 子句（不含 `WHERE` 关键字），参数使用 `$1` 占位符
    /// - `where_params`: WHERE 子句的参数值
    pub async fn batch_update(&self, table: &str, sets: &Row, where_sql: &str, where_params: &[&str]) -> DbResult<u64> {
        if sets.changes.is_empty() { return Err(DbError::Argument("没有要更新的字段".into())); }
        let set_clauses: Vec<String> = sets.changes.iter().enumerate()
            .map(|(i, col)| {
                let cast = sets.data.get(col).map(|v| value_cast(v)).unwrap_or("");
                format!("{} = ${}{}", col, i + 1, cast)
            }).collect();
        let set_values: Vec<String> = sets.changes.iter()
            .filter_map(|c| sets.data.get(c)).map(value_to_string).collect();

        let offset = sets.changes.len();
        let adjusted_where = adjust_param_indices_with_casts(where_sql, offset, where_params);
        let sql = format!("UPDATE {} SET {} WHERE {}", table, set_clauses.join(", "), adjusted_where);
        let mut all: Vec<String> = set_values;
        all.extend(where_params.iter().map(|s| s.to_string()));
        let val_refs: Vec<&str> = all.iter().map(|s| s.as_str()).collect();
        self.execute(&sql, &val_refs).await
    }

    /// 按主键删除记录，返回是否成功删除
    pub async fn delete_by_id(&self, table: &str, id: impl Into<serde_json::Value>) -> DbResult<bool> {
        let value: serde_json::Value = id.into();
        let pk = "id";
        let id_str = value_to_string(&value);
        let sql = format!("DELETE FROM {} WHERE {}=$1{}",
            table, pk, id_cast(&value));
        let n = self.execute(&sql, &[&id_str]).await?;
        Ok(n > 0)
    }

    /// 批量按主键删除，返回受影响行数
    pub async fn batch_delete_by_ids(&self, table: &str, ids: &[impl AsRef<str>]) -> DbResult<u64> {
        if ids.is_empty() { return Ok(0); }
        let is_uuid = ids.first().map(|id| {
            let s = id.as_ref();
            s.len() == 36 && s.chars().filter(|&c| c == '-').count() == 4
        }).unwrap_or(false);
        let cast = if is_uuid { "::uuid" } else { "" };
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("${}{}", i, cast)).collect();
        let sql = format!("DELETE FROM {} WHERE id IN ({})", table, placeholders.join(", "));
        let params: Vec<&str> = ids.iter().map(|id| id.as_ref()).collect();
        self.execute(&sql, &params).await
    }

    /// 执行 INSERT/UPDATE/DELETE 等写操作，返回受影响行数
    pub async fn execute(&self, sql: &str, params: &[&str]) -> DbResult<u64> {
        match &self.pool {
            DbPool::Postgres(pool) => execute_pg_pool(pool, sql, params).await,
            DbPool::Mysql(pool)    => execute_my_sql_pool(pool, sql, params).await,
            DbPool::Sqlite(pool)   => execute_sqlite_pool(pool, sql, params).await,
            DbPool::Any(pool)      => execute_any(pool, sql, params).await,
        }
    }

    // ── 事务 ─────────────────────────────────────────

    /// 在事务闭包中执行操作，自动 BEGIN/COMMIT/ROLLBACK
    ///
    /// 闭包返回 `Ok` 则 COMMIT，返回 `Err` 则 ROLLBACK。
    /// 可通过 `ActiveTx::set_rollback_only()` 强制回滚。
    ///
    /// # 示例
    ///
    /// ```ignore
    /// db.transaction(|mut tx| async move {
    ///     let user = Row::table("user").id(1);
    ///     tx.execute("UPDATE user SET name=$1 WHERE id=$2", &["Alice", "1"]).await?;
    ///     (tx, Ok(()))
    /// }).await?;
    /// ```
    pub async fn transaction<F, Fut, T>(&self, f: F) -> DbResult<T>
    where
        F: FnOnce(crate::tx::ActiveTx) -> Fut + Send,
        Fut: std::future::Future<Output = (crate::tx::ActiveTx, DbResult<T>)> + Send,
        T: Send,
    {
        let mut rollback_only = false;
        crate::tx::execute_transaction(&self.pool, crate::tx::Isolation::ReadCommitted, &mut rollback_only, f).await
    }
}

// ── 辅助函数 ───────────────────────────────────────

pub(crate) fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn adjust_param_indices_with_casts(sql: &str, offset: usize, params: &[&str]) -> String {
    let re = regex::Regex::new(r"\$(\d+)").unwrap();
    if offset == 0 {
        return re.replace_all(sql, |caps: &regex::Captures| {
            let n: usize = caps[1].parse().unwrap_or(0);
            let cast = params.get(n.wrapping_sub(1)).map(|v| {
                let s: &str = v;
                if s.len() == 36 && s.chars().filter(|&c| c == '-').count() == 4 { "::uuid" }
                else if s.parse::<i64>().is_ok() { "::bigint" }
                else if s.parse::<f64>().is_ok() { "::double precision" }
                else { "" }
            }).unwrap_or("");
            format!("${}{}", n, cast)
        }).to_string();
    }
    re.replace_all(sql, |caps: &regex::Captures| {
        let n: usize = caps[1].parse().unwrap_or(0);
        let cast = params.get(n.wrapping_sub(1)).map(|v| {
            let s: &str = v;
            if s.len() == 36 && s.chars().filter(|&c| c == '-').count() == 4 { "::uuid" }
            else if s.parse::<i64>().is_ok() { "::bigint" }
            else if s.parse::<f64>().is_ok() { "::double precision" }
            else { "" }
        }).unwrap_or("");
        format!("${}{}", n + offset, cast)
    }).to_string()
}

fn id_cast(value: &Value) -> &'static str {
    match IdKind::detect(value) {
        IdKind::Uuid => "::uuid",
        IdKind::I64 => "::bigint",
        _ => "",
    }
}

fn value_cast(value: &Value) -> &'static str {
    match value {
        Value::Object(_) | Value::Array(_) => "::jsonb",
        Value::String(s) => {
            if is_inet_format(s) {
                "::inet"
            } else {
                match IdKind::detect(value) {
                    IdKind::Uuid => "::uuid",
                    IdKind::I64 => "::bigint",
                    IdKind::F64 => "::double precision",
                    IdKind::Bool => "::boolean",
                    _ => "",
                }
            }
        }
        _ => match IdKind::detect(value) {
            IdKind::Uuid => "::uuid",
            IdKind::I64 => "::bigint",
            IdKind::F64 => "::double precision",
            IdKind::Bool => "::boolean",
            _ => "",
        },
    }
}

fn is_inet_format(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() == 4 && parts.iter().all(|p| p.parse::<u8>().is_ok()) {
        return true;
    }
    if s.contains("::") {
        return true;
    }
    if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() >= 2 && parts.len() <= 8 {
            return parts.iter().all(|p| p.is_empty() || u16::from_str_radix(p, 16).is_ok());
        }
    }
    false
}



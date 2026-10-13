//! PostgreSQL 连接。封装 tokio-postgres `Client`。

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use alun_db_core::error::{DbError, Result};
use alun_db_core::statement::Statement;
use alun_db_core::traits::dialect::{Dialect, RawDbError};
use alun_db_core::traits::driver::PoolConfig;
use alun_db_core::traits::executor::{ExecuteResult, Rows};
use alun_db_sql::dialects::postgres::PostgresDialect;
use alun_db_sql::rewrite_placeholders;

use crate::convert;

/// 每个连接预编译语句缓存的上限。
///
/// 超出后整体清空重置：`CREATE`/`DROP` 等高频率 DDL 之外，业务 SQL 数量
/// 有限，清空成本可接受，避免缓存无限增长占用服务端与客户端内存。
const STMT_CACHE_CAP: usize = 128;

/// PostgreSQL 连接。
///
/// `tokio_postgres::Client` 是 `Send + Sync` 的（协议 IO 由连接创建时
/// spawn 的驱动任务独占处理），无需 SQLite 驱动的 take / restore 舞步。
///
/// `tokio_postgres` 的 `Client::execute(sql, …)` 每次调用都会在服务端
/// 重新 `PREPARE` + 执行（一次额外网络往返）。本连接以 `Mutex` 维护
/// 一个「SQL → 服务端 prepared statement」缓存，重复的 CRUD 语句复用
/// 既有的服务端句柄，省去重复编译。`tokio_postgres::Statement` 内部
/// 持连接状态的 `Arc`（Send + Sync），可在同一连接内合法共享；锁仅在
/// 查表 / 插入的瞬时持用（不跨 `await`），std `Mutex` 足够且保持连接
/// 为 `Sync`（满足 facade 的 `Send` future 约束）。
pub struct PostgresConnection {
    /// tokio-postgres 客户端。协议 IO 由 spawn 的任务处理。
    client: tokio_postgres::Client,
    /// SQL → 服务端 prepared statement 缓存。
    ///
    /// `Mutex` 保证并发访问安全（`Statement.is_sync`）；但连接由池独占
    /// 派发，实际几乎无竞争。事务边界（BEGIN / COMMIT / ROLLBACK）时清空：
    /// PG 中事务内准备的语句在事务结束后不可再用，缓存在会复用到脏计划。
    stmt_cache: Mutex<HashMap<String, tokio_postgres::Statement>>,
    /// 当前会话已生效的租户 schema（`None` 表示默认 `search_path`）。
    ///
    /// 连接在池中跨请求复用，`search_path` 是会话级状态；记录上次生效值，
    /// 目标 schema 未变时跳过重复 `SET`，避免每次借出都多一次网络往返。
    session_schema: Mutex<Option<String>>,
    /// 连接创建时刻。用于 `max_lifetime` 检查。
    created_at: Instant,
}

impl PostgresConnection {
    /// 建立连接。
    ///
    /// 连接的协议驱动任务随连接 spawn：任务退出即连接终止，
    /// 此后 [`PostgresConnection::is_available`] 返回 `false`。
    ///
    /// # 参数
    ///
    /// - `url`：`postgres://user:pass@host:port/db` 形式的连接 URL
    ///
    /// # 错误
    ///
    /// TCP 连接、TLS 或认证失败时返回 [`DbError::Connection`]。
    pub async fn connect(url: &str) -> Result<Self> {
        let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
            .await
            .map_err(|e| DbError::Connection(Box::new(e)))?;
        // 协议驱动任务：错误仅意味着连接终止，连接体销毁即可，
        // 断连状态由 client.is_closed() 反映
        tokio::spawn(async move {
            let _ = connection.await;
        });
        Ok(PostgresConnection {
            client,
            stmt_cache: Mutex::new(HashMap::new()),
            session_schema: Mutex::new(None),
            created_at: Instant::now(),
        })
    }

    /// 连接是否仍可复用：未断开且未超过最大存活时长。
    pub fn is_available(&self, config: &PoolConfig) -> bool {
        if self.client.is_closed() {
            return false;
        }
        if let Some(max_lifetime) = config.max_lifetime
            && self.created_at.elapsed() > max_lifetime
        {
            return false;
        }
        true
    }

    /// 按租户 schema 设置本连接的 `search_path`（PG schema-per-tenant）。
    ///
    /// `schema = Some(name)` 时执行 `SET search_path TO "name"`；`None` 时，
    /// 若此前设置过租户 schema，则执行 `RESET search_path` 回到默认（`public`）。
    /// 目标与上次生效值一致时直接返回，不产生额外网络往返。
    ///
    /// 连接由池独占借出，且本方法在业务语句（或事务 `BEGIN`）之前调用，
    /// 会话级设置对本次借出可见。注意**不要**在事务内调用：`SET` 位于事务块内
    /// 会随回滚失效，使缓存的「已生效值」失真。
    ///
    /// # 错误
    ///
    /// `name` 含双引号或空字节等非法字符，或 `SET` / `RESET` 执行失败时返回
    /// [`DbError`]。
    pub async fn apply_search_path(&self, schema: Option<&str>) -> Result<()> {
        // 与上次生效值一致（含「本就是默认」）则跳过，避免重复往返
        if self.lock_schema().as_deref() == schema {
            return Ok(());
        }
        match schema {
            Some(name) => {
                let escaped = escape_schema_name(name)?;
                self.client
                    .batch_execute(&format!("SET search_path TO \"{escaped}\""))
                    .await
                    .map_err(|e| map_pg_error(e, "SET search_path"))?;
                *self.lock_schema() = Some(name.to_string());
            }
            None => {
                self.client
                    .batch_execute("RESET search_path")
                    .await
                    .map_err(|e| map_pg_error(e, "RESET search_path"))?;
                *self.lock_schema() = None;
            }
        }
        Ok(())
    }

    /// 将 `Statement` 归一化为 `(sql, params)`。
    ///
    /// Raw 语句按 `?` 书写，执行前改写为 PG 的 `$n` 风格；
    /// 非 Raw 语句由方言渲染器直接产出 `$n` 占位符。
    fn prepare(&self, stmt: &Statement) -> (String, Vec<alun_db_core::stmt::Value>) {
        match stmt {
            Statement::Raw(raw) => {
                let sql = rewrite_placeholders(&raw.sql, &PostgresDialect);
                (sql, raw.params.clone())
            }
            _ => {
                let rendered = PostgresDialect.render_statement(stmt);
                (rendered.sql, rendered.params)
            }
        }
    }

    /// 执行 INSERT / UPDATE / DELETE。
    ///
    /// PostgreSQL 无 `last_insert_id` 语义：自增主键取回由 facade 的
    /// `INSERT ... RETURNING` 路径完成，此处恒返回 `None`。
    ///
    /// # 错误
    ///
    /// 参数编码或语句执行失败时返回 [`DbError`]，服务端错误码经方言
    /// `map_error` 映射（如 `23505` → `UniqueViolation`）。
    pub async fn execute_stmt(&self, stmt: &Statement) -> Result<ExecuteResult> {
        let (sql, params) = self.prepare(stmt);
        let pg_params = convert::to_pg_params(&params)?;
        let param_refs = convert::as_param_refs(&pg_params);
        let statement = self.cached_stmt(&sql).await?;
        let rows_affected = self
            .client
            .execute(&statement, &param_refs)
            .await
            .map_err(|e| map_pg_error(e, &sql))?;
        Ok(ExecuteResult {
            rows_affected,
            last_insert_id: None,
        })
    }

    /// 执行 SELECT。
    ///
    /// # 错误
    ///
    /// 参数编码、执行或列读取失败时返回 [`DbError`]。
    pub async fn query_stmt(&self, stmt: &Statement) -> Result<Rows> {
        let (sql, params) = self.prepare(stmt);
        let pg_params = convert::to_pg_params(&params)?;
        let param_refs = convert::as_param_refs(&pg_params);
        let statement = self.cached_stmt(&sql).await?;
        let pg_rows = self
            .client
            .query(&statement, &param_refs)
            .await
            .map_err(|e| map_pg_error(e, &sql))?;
        let mut rows = Vec::with_capacity(pg_rows.len());
        for pg_row in &pg_rows {
            rows.push(convert::pg_row_to_row(pg_row)?);
        }
        Ok(Rows(rows))
    }

    /// 执行多语句 SQL 脚本（`;` 分隔的完整脚本，不支持参数绑定）。
    ///
    /// 底层使用 tokio-postgres 的 `batch_execute`（simple query 协议），
    /// 走协议而非预编译语句，因此不经过 `stmt_cache`，也不受其影响。
    ///
    /// # 错误
    ///
    /// 任一语句执行失败时返回 [`DbError`]，错误码经方言 `map_error` 映射。
    pub async fn execute_script_async(&self, sql: &str) -> Result<()> {
        self.client
            .batch_execute(sql)
            .await
            .map_err(|e| map_pg_error(e, sql))?;
        Ok(())
    }

    /// 取回 SQL 对应的服务端 prepared statement。
    ///
    /// 命中缓存直接复用；未命中则先行 `PREPARE`（一次网络往返）并缓存。
    /// 缓存以 `clone`（`Arc` 引用计数）返回，避免长时间持锁影响后续借用。
    ///
    /// # 错误
    ///
    /// 服务端准备失败时返回 [`DbError`]，错误码经方言 `map_error` 映射。
    async fn cached_stmt(&self, sql: &str) -> Result<tokio_postgres::Statement> {
        if let Some(stmt) = self.lock_cache().get(sql) {
            return Ok(stmt.clone());
        }
        let stmt = self
            .client
            .prepare(sql)
            .await
            .map_err(|e| map_pg_error(e, sql))?;
        let mut cache = self.lock_cache();
        // 防缓存无限增长：超上限整体清空（业务 SQL 有限，清空成本可接受）
        if cache.len() >= STMT_CACHE_CAP {
            cache.clear();
        }
        cache.insert(sql.to_string(), stmt.clone());
        Ok(stmt)
    }

    /// 加锁取得语句缓存的可变访问。锁中毒（持锁 panic）时取回已毒化的
    /// 数据继续使用，避免把一次异常放大为整个连接不可用。
    fn lock_cache(&self) -> MutexGuard<'_, HashMap<String, tokio_postgres::Statement>> {
        self.stmt_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 清空预编译语句缓存。
    ///
    /// 事务边界（见 [`Self::begin`] / [`Self::commit`] / [`Self::rollback`]）
    /// 调用：PG 中事务内准备的语句在事务结束后不可复用，逐出避免脏计划。
    fn purge_cache(&self) {
        self.lock_cache().clear();
    }

    /// 加锁取得「已生效租户 schema」的可变访问。锁中毒时取回已毒化数据继续
    /// 使用，避免把一次异常放大为整个连接不可用。
    fn lock_schema(&self) -> MutexGuard<'_, Option<String>> {
        self.session_schema
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// 开始事务。
    ///
    /// # 错误
    ///
    /// `BEGIN` 执行失败时返回 [`DbError::Transaction`]。
    pub async fn begin(&self) -> Result<()> {
        // 事务边界逐出缓存：事务内准备的语句生命周期结束后不可复用
        self.purge_cache();
        self.client
            .execute("BEGIN", &[])
            .await
            .map_err(|e| DbError::Transaction(e.to_string()))?;
        Ok(())
    }

    /// 提交事务。
    ///
    /// # 错误
    ///
    /// `COMMIT` 执行失败时返回 [`DbError::Transaction`]。
    pub async fn commit(&self) -> Result<()> {
        self.purge_cache();
        self.client
            .execute("COMMIT", &[])
            .await
            .map_err(|e| DbError::Transaction(e.to_string()))?;
        Ok(())
    }

    /// 回滚事务。
    ///
    /// # 错误
    ///
    /// `ROLLBACK` 执行失败时返回 [`DbError::Transaction`]。
    pub async fn rollback(&self) -> Result<()> {
        self.purge_cache();
        self.client
            .execute("ROLLBACK", &[])
            .await
            .map_err(|e| DbError::Transaction(e.to_string()))?;
        Ok(())
    }
}

/// 校验租户 schema 名（用于 `SET search_path TO "…"`）。
///
/// schema 名由租户 ID 派生，而租户 ID 来自不可信的 JWT claim，因此此处做
/// 纵深防御：拒绝双引号 / 反斜杠 / 空字节 / 空白，再由调用方以双引号包裹。
/// 字符集与长度合法性由上层 `alun-db` 在派生 schema 名时先行校验。
///
/// # 错误
///
/// 名称为空或含非法字符时返回 [`DbError::InvalidValue`]。
fn escape_schema_name(name: &str) -> Result<String> {
    let is_unsafe = name.is_empty()
        || name
            .bytes()
            .any(|b| b == b'"' || b == b'\\' || b == 0 || b.is_ascii_whitespace());
    if is_unsafe {
        return Err(DbError::InvalidValue(format!(
            "invalid tenant schema name `{name}`"
        )));
    }
    Ok(name.to_string())
}

/// 将 tokio-postgres 错误映射为统一的 `DbError`。
///
/// 服务端错误（`SqlState` 码如 `23505` 唯一约束）经方言 `map_error`
/// 转换；其余错误落入 `Query` 变体并保留 SQL 上下文。
fn map_pg_error(e: tokio_postgres::Error, sql: &str) -> DbError {
    // 先借用提取错误码与消息，再消耗 e 作为底层错误源
    let db_error = e
        .as_db_error()
        .map(|db| (db.code().code().to_string(), db.message().to_string()));
    match db_error {
        Some((code, message)) => PostgresDialect.map_error(RawDbError {
            code: Some(code),
            message,
            source: Some(Box::new(e)),
        }),
        None => DbError::Query {
            sql: sql.to_string(),
            source: Box::new(e),
        },
    }
}

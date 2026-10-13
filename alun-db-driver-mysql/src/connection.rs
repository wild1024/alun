//! MySQL 连接。封装 mysql_async `Conn`。

use std::time::Instant;

use alun_db_core::error::{DbError, Result};
use alun_db_core::statement::Statement;
use alun_db_core::traits::dialect::{Dialect, RawDbError};
use alun_db_core::traits::driver::PoolConfig;
use alun_db_core::traits::executor::{ExecuteResult, Rows};
use alun_db_sql::dialects::mysql::MysqlDialect;
use mysql_async::prelude::Queryable;

use crate::convert;

/// MySQL 连接。
///
/// mysql_async 的 `Conn` 以 `&mut self` 执行查询（连接上同时只能有
/// 一个进行中的查询），因此本类型的方法均要求可变借用；池化句柄
/// [`crate::pool::PooledMysqlConnection`] 相应提供 `conn_mut`。
pub struct MysqlConnection {
    /// mysql_async 连接本体。
    conn: mysql_async::Conn,
    /// 连接创建时刻。用于 `max_lifetime` 检查。
    created_at: Instant,
}

impl MysqlConnection {
    /// 建立连接。
    ///
    /// # 参数
    ///
    /// - `url`：`mysql://user:pass@host:port/db` 形式的连接 URL
    ///
    /// # 错误
    ///
    /// TCP 连接、认证或 URL 解析失败时返回 [`DbError::Connection`]。
    pub async fn connect(url: &str) -> Result<Self> {
        let conn = mysql_async::Conn::from_url(url)
            .await
            .map_err(|e| DbError::Connection(Box::new(e)))?;
        Ok(MysqlConnection {
            conn,
            created_at: Instant::now(),
        })
    }

    /// 连接是否仍可复用：未断开且未超过最大存活时长。
    pub fn is_available(&self, config: &PoolConfig) -> bool {
        if self.conn.is_disconnected() {
            return false;
        }
        if let Some(max_lifetime) = config.max_lifetime
            && self.created_at.elapsed() > max_lifetime
        {
            return false;
        }
        true
    }

    /// 将 `Statement` 归一化为 `(sql, params)`。
    ///
    /// MySQL 占位符即 `?`，Raw 语句无需改写；非 Raw 语句由方言渲染器
    /// 产出（同样是 `?` 风格）。
    fn prepare(&self, stmt: &Statement) -> (String, Vec<alun_db_core::stmt::Value>) {
        match stmt {
            Statement::Raw(raw) => (raw.sql.clone(), raw.params.clone()),
            _ => {
                let rendered = MysqlDialect.render_statement(stmt);
                (rendered.sql, rendered.params)
            }
        }
    }

    /// 执行 INSERT / UPDATE / DELETE。
    ///
    /// 自增主键经 `QueryResult::last_insert_id` 取回（无 AUTO_INCREMENT
    /// 的语句返回 `None`）。
    ///
    /// # 错误
    ///
    /// 参数编码或语句执行失败时返回 [`DbError`]，服务端错误码经方言
    /// `map_error` 映射（如 `1062` → `UniqueViolation`）。
    pub async fn execute_stmt(&mut self, stmt: &Statement) -> Result<ExecuteResult> {
        let (sql, params) = self.prepare(stmt);
        let mysql_params = convert::to_mysql_params(&params)?;
        let result = self
            .conn
            .exec_iter(sql.as_str(), mysql_async::Params::Positional(mysql_params))
            .await
            .map_err(|e| map_mysql_error(e, &sql))?;
        let rows_affected = result.affected_rows();
        // AUTO_INCREMENT id 显式收窄：超出 i64 视为异常而非静默丢弃
        let last_insert_id = match result.last_insert_id() {
            Some(id) => match i64::try_from(id) {
                Ok(signed) => Some(signed),
                Err(_) => {
                    return Err(DbError::InvalidValue(
                        "last insert id exceeds i64 range".into(),
                    ));
                }
            },
            None => None,
        };
        // 消费（可能存在的）附加结果集：主语句的 OK packet 已确认成功，
        // 触发器 / 存储过程产生的附加结果集对调用方无意义，静默清理；
        // 清理失败意味着连接进入坏状态，下一次查询自然报错
        let _ = result.collect_and_drop::<mysql_async::Row>().await;
        Ok(ExecuteResult {
            rows_affected,
            last_insert_id,
        })
    }

    /// 执行 SELECT。
    ///
    /// # 错误
    ///
    /// 参数编码、执行或行读取失败时返回 [`DbError`]。
    pub async fn query_stmt(&mut self, stmt: &Statement) -> Result<Rows> {
        let (sql, params) = self.prepare(stmt);
        let mysql_params = convert::to_mysql_params(&params)?;
        let result = self
            .conn
            .exec_iter(sql.as_str(), mysql_async::Params::Positional(mysql_params))
            .await
            .map_err(|e| map_mysql_error(e, &sql))?;
        // FromRow for mysql_async::Row 是恒等实现，collect 无 panic 风险
        let mysql_rows: Vec<mysql_async::Row> = result
            .collect_and_drop()
            .await
            .map_err(|e| map_mysql_error(e, &sql))?;
        let mut rows = Vec::with_capacity(mysql_rows.len());
        for mysql_row in &mysql_rows {
            rows.push(convert::mysql_row_to_row(mysql_row)?);
        }
        Ok(Rows(rows))
    }

    /// 开始事务。
    ///
    /// # 错误
    ///
    /// `START TRANSACTION` 执行失败时返回 [`DbError::Transaction`]。
    pub async fn begin(&mut self) -> Result<()> {
        self.execute_simple("START TRANSACTION").await
    }

    /// 提交事务。
    ///
    /// # 错误
    ///
    /// `COMMIT` 执行失败时返回 [`DbError::Transaction`]。
    pub async fn commit(&mut self) -> Result<()> {
        self.execute_simple("COMMIT").await
    }

    /// 回滚事务。
    ///
    /// # 错误
    ///
    /// `ROLLBACK` 执行失败时返回 [`DbError::Transaction`]。
    pub async fn rollback(&mut self) -> Result<()> {
        self.execute_simple("ROLLBACK").await
    }

    /// 执行无参数的单语句（事务控制命令）。
    ///
    /// 结果集消费干净后才返回，避免残留错误延迟到下一次查询。
    async fn execute_simple(&mut self, sql: &'static str) -> Result<()> {
        self.conn
            .query_iter(sql)
            .await
            .map_err(|e| DbError::Transaction(e.to_string()))?
            .collect_and_drop::<mysql_async::Row>()
            .await
            .map_err(|e| DbError::Transaction(e.to_string()))?;
        Ok(())
    }
}

/// 将 mysql_async 错误映射为统一的 `DbError`。
///
/// 服务端错误（如 `1062` 唯一约束、`1452` / `1451` 外键、`3819` CHECK）
/// 经方言 `map_error` 转换；其余错误落入 `Query` 变体并保留 SQL 上下文。
fn map_mysql_error(e: mysql_async::Error, sql: &str) -> DbError {
    // 先借用提取错误码与消息，再消耗 e 作为底层错误源
    let server_error = match &e {
        mysql_async::Error::Server(server) => Some((server.code, server.message.clone())),
        _ => None,
    };
    match server_error {
        Some((code, message)) => MysqlDialect.map_error(RawDbError {
            code: Some(code.to_string()),
            message,
            source: Some(Box::new(e)),
        }),
        None => DbError::Query {
            sql: sql.to_string(),
            source: Box::new(e),
        },
    }
}

//! `Db` 门面。

use alun_db_core::error::{DbError, Result};
use alun_db_core::page::Page;
use alun_db_core::row::Row;
use alun_db_core::statement::{RawStmt, Statement};
use alun_db_core::stmt::Value;
use alun_db_core::traits::TypeMapper;

use crate::builder::{CountBuilder, FindBuilder, PageBuilder};
use crate::config::DatabaseConfig;
use crate::executor::RawExecutor;
use crate::factory::create_driver;
use crate::query::can_append_limit;

/// 数据库门面。
///
/// `Clone` 共享底层连接池（`Arc`）：克隆体与原句柄复用同一批连接，
/// 适合 axum state 等共享场景。
#[derive(Clone)]
pub struct Db {
    /// 底层执行器。按配置选择的驱动连接池。
    pub(crate) executor: std::sync::Arc<dyn RawExecutor>,
    /// 数据库配置。
    pub(crate) config: DatabaseConfig,
}

impl Db {
    /// 按 URL 建立连接。
    ///
    /// 连接串示例：`sqlite://./db.sqlite`、`postgres://user:pass@host/db`、
    /// `mysql://user:pass@host/db`。建池成功后立即执行一次 `SELECT 1`
    /// 做连接测试，使认证失败 / 库不可达在启动期快速暴露。
    ///
    /// # 错误
    ///
    /// 连接串无法解析或连接测试失败时返回 [`DbError`]。
    pub async fn connect(url: &str) -> Result<Self> {
        let config = DatabaseConfig::from_url(url);
        Self::connect_with(url, config).await
    }

    /// 按 URL 与自定义配置建立连接。
    ///
    /// 在 [`connect`](Self::connect) 基础上允许覆盖连接池等配置；显式传入的
    /// `url` 始终生效。连接建立后同样执行一次 `SELECT 1` 连接测试。
    ///
    /// # 错误
    ///
    /// 连接建立或连接测试失败时返回 [`DbError`]。
    pub async fn connect_with(url: &str, config: DatabaseConfig) -> Result<Self> {
        // 显式传入的 url 始终生效，覆盖 config 内的同名字段
        let mut config = config;
        config.url = url.to_string();
        let executor = create_driver(&config).await?;
        // 连接测试：建池后立即执行一次 SELECT 1，使认证失败 / 库不可达等
        // 启动期问题在此快速暴露，而非延迟到首次业务查询
        executor
            .query_raw(Statement::Raw(RawStmt {
                sql: "SELECT 1".into(),
                params: vec![],
            }))
            .await?;
        Ok(Db { executor, config })
    }

    /// 返回当前数据库配置。
    pub fn config(&self) -> &DatabaseConfig {
        &self.config
    }

    /// 当前驱动的 SQL 方言。
    ///
    /// 供 Builder 等需要渲染 SQL 片段的入口使用。
    pub fn dialect(&self) -> &'static dyn alun_db_core::traits::Dialect {
        self.executor.dialect()
    }

    // ==================== 查询 ====================

    /// 执行 SELECT，返回所有行。
    pub async fn find(&self, sql: &str) -> Result<Vec<Row>> {
        self.find_with_params(sql, vec![]).await
    }

    /// 执行 SELECT，返回首行。
    pub async fn find_first(&self, sql: &str) -> Result<Option<Row>> {
        self.find_first_with_params(sql, vec![]).await
    }

    /// 执行 SELECT，返回单值。
    pub async fn scalar<T: TypeMapper>(&self, sql: &str) -> Result<T> {
        self.scalar_with_params(sql, vec![]).await
    }

    /// 带参数执行 SELECT，返回单值。
    pub async fn scalar_with_params<T: TypeMapper>(
        &self,
        sql: &str,
        params: Vec<Value>,
    ) -> Result<T> {
        let row = self
            .find_first_with_params(sql, params)
            .await?
            .ok_or(DbError::NotFound)?;
        let value = row
            .values()
            .next()
            .ok_or_else(|| DbError::ColumnNotFound("no columns".into()))?;
        T::from_value(value)
    }

    /// 带参数的查询。
    pub async fn find_with_params(&self, sql: &str, params: Vec<Value>) -> Result<Vec<Row>> {
        self.find_owned(sql.to_string(), params).await
    }

    /// 带参数的查询（SQL 已是拥有所有权的字符串，避免再克隆一份）。
    ///
    /// 调用方（Builder 渲染、`SqlExecutor` 链）本就把 SQL 做成 `String`，
    /// 走 [`find_with_params`](Self::find_with_params) 会白白多一次整串拷贝。
    pub(crate) async fn find_owned(&self, sql: String, params: Vec<Value>) -> Result<Vec<Row>> {
        let stmt = Statement::Raw(RawStmt { sql, params });
        // F10：所有 SELECT 最终汇聚于此（Builder / `sql!` / `ModelQuery` 都经此），
        // 因此只在此埋点即可覆盖全读路径，无需在每条链路上重复计时。
        #[cfg(feature = "metrics")]
        let rows = crate::metrics::instrument(
            "query",
            crate::metrics::TABLE_RAW,
            self.executor.query_raw(stmt),
        )
        .await?;
        #[cfg(not(feature = "metrics"))]
        let rows = self.executor.query_raw(stmt).await?;
        Ok(rows.0)
    }

    /// 带参数的查询，返回首行。
    ///
    /// 当 SQL 为不含 `LIMIT` 的 SELECT 类语句时自动追加 `LIMIT 1`，
    /// 避免全量取回后再取首行；INSERT ... RETURNING 等非查询语句不受影响。
    pub async fn find_first_with_params(
        &self,
        sql: &str,
        params: Vec<Value>,
    ) -> Result<Option<Row>> {
        // 追加 `LIMIT 1` 与构造语句一次成型，不再经通用查询路径二次拷贝 SQL
        let sql = if can_append_limit(sql) {
            format!("{sql} LIMIT 1")
        } else {
            sql.to_string()
        };
        self.find_first_owned(sql, params).await
    }

    /// 带参数的查询并只保留首行（SQL 已拥有所有权）。
    ///
    /// 缺少 `LIMIT` 时由调用方追加；本方法不再改动 SQL，也不构造整段结果
    /// 再去掉首行以外的行。
    pub(crate) async fn find_first_owned(
        &self,
        sql: String,
        params: Vec<Value>,
    ) -> Result<Option<Row>> {
        let stmt = Statement::Raw(RawStmt { sql, params });
        let rows = self.executor.query_raw(stmt).await?;
        Ok(rows.0.into_iter().next())
    }

    // ==================== 分页 ====================

    /// 分页查询。
    ///
    /// 页码与页大小由调用方给出（Web 场景下通常来自框架的分页参数
    /// `PageQuery` 的 `page` / `page_size` 字段）；本方法自身也做下界保护。
    ///
    /// # 参数
    ///
    /// - `sql`：基础查询（不含 LIMIT / OFFSET）
    /// - `page`：页码，从 1 开始；传入 0 时按 1 处理
    /// - `size`：每页大小，至少 1；传入 0 时按 1 处理
    pub async fn paginate(&self, sql: &str, page: u64, size: u64) -> Result<Page<Row>> {
        self.paginate_with_params(sql, vec![], page, size).await
    }

    /// 带参数的分页查询。
    ///
    /// # 参数
    ///
    /// - `sql`：基础查询（不含 LIMIT / OFFSET），可含占位符
    /// - `params`：占位符对应的绑定参数
    /// - `page`：页码，从 1 开始；传入 0 时按 1 处理
    /// - `size`：每页大小，至少 1；传入 0 时按 1 处理
    pub async fn paginate_with_params(
        &self,
        sql: &str,
        params: Vec<Value>,
        page: u64,
        size: u64,
    ) -> Result<Page<Row>> {
        // 页码与页大小的下界保护，避免无符号下溢
        let page = page.max(1);
        let size = size.max(1);
        let offset = (page - 1) * size;

        // 计数
        let count_sql = format!("SELECT COUNT(*) FROM ({sql}) AS _count");
        let total: i64 = self.scalar_with_params(&count_sql, params.clone()).await?;

        // 数据：LIMIT / OFFSET 内联为字面量（见 `Dialect::push_limit_offset`：
        // 参数化 LIMIT 会让 SQLite 每次查询多花约 2.7 µs）
        let mut data_sql = String::with_capacity(sql.len() + 24);
        data_sql.push_str(sql);
        self.dialect()
            .push_limit_offset(Some(size), Some(offset), &mut data_sql);
        let list = self.find_owned(data_sql, params).await?;

        Ok(Page {
            list,
            total: total as u64,
            page,
            size,
            pages: Page::<Row>::calc_pages(total as u64, size),
        })
    }

    // ==================== 执行 ====================

    /// 执行 INSERT / UPDATE / DELETE，返回受影响行数。
    pub async fn execute(&self, sql: &str) -> Result<u64> {
        self.execute_with_params(sql, vec![]).await
    }

    /// 执行多语句 SQL 脚本（迁移 / 初始化 / 测试种子数据场景）。
    ///
    /// 依赖驱动的多语句执行能力：SQLite、PostgreSQL 原生支持；MySQL 驱动
    /// 未启用多语句模式时返回 [`DbError::UnsupportedFeature`]。
    pub async fn execute_script(&self, sql: &str) -> Result<()> {
        #[cfg(feature = "metrics")]
        {
            crate::metrics::instrument(
                "script",
                crate::metrics::TABLE_RAW,
                self.executor.execute_script(sql),
            )
            .await
        }
        #[cfg(not(feature = "metrics"))]
        {
            self.executor.execute_script(sql).await
        }
    }

    /// 带参数执行。
    pub async fn execute_with_params(&self, sql: &str, params: Vec<Value>) -> Result<u64> {
        self.execute_owned(sql.to_string(), params).await
    }

    /// 带参数执行（SQL 已拥有所有权，避免再克隆一份）。
    pub(crate) async fn execute_owned(&self, sql: String, params: Vec<Value>) -> Result<u64> {
        let stmt = Statement::Raw(RawStmt { sql, params });
        // F10：写路径汇聚点（同 `find_owned`）
        #[cfg(feature = "metrics")]
        let result = crate::metrics::instrument(
            "exec",
            crate::metrics::TABLE_RAW,
            self.executor.execute_raw(stmt),
        )
        .await?;
        #[cfg(not(feature = "metrics"))]
        let result = self.executor.execute_raw(stmt).await?;
        Ok(result.rows_affected)
    }

    /// 执行 INSERT 并返回自增主键。
    pub async fn insert_returning_id(&self, sql: &str, params: Vec<Value>) -> Result<i64> {
        let stmt = Statement::Raw(RawStmt {
            sql: sql.to_string(),
            params,
        });
        let result = self.executor.execute_raw(stmt).await?;
        result.last_insert_id.ok_or_else(|| DbError::Query {
            sql: sql.to_string(),
            source: "no last_insert_id returned".into(),
        })
    }

    // ==================== Builder 入口 ====================

    /// 创建查询 Builder。链式 `table` / `filter` / `order_by` 后以
    /// [`FindBuilder::exec`] 执行。
    pub fn query(&self) -> FindBuilder {
        FindBuilder::new()
    }

    /// 创建分页 Builder。基于页码的 OFFSET 分页，或 `page_after` 键值（keyset）
    /// 分页。
    pub fn page(&self, page: u64, size: u64) -> PageBuilder {
        PageBuilder::new(page, size)
    }

    /// 创建计数 Builder。以 [`CountBuilder::exec`] 返回 COUNT 结果。
    pub fn count(&self) -> CountBuilder {
        CountBuilder::new()
    }
}

//! 动态行（`Row`）的写操作。
//!
//! 无 `Model` 时，可借助 [`Row`] 携带的实体元数据（表名 + 主键字段名，
//! 见 [`Row::with_entity`]）直接完成插入 / 更新 / 删除，省去手写 SQL。
//! 适用于脚本、AI 生成等「动态实体」场景。

use std::future::Future;

use alun_db_core::error::{DbError, Result};
use alun_db_core::row::Row;
use alun_db_core::stmt::Value;
use alun_db_core::traits::Dialect;

use crate::db::Db;
use crate::persist::max_bind_params;
use crate::query::SqlExecutor;

/// 校验并引用一个标识符（表名 / 列名），杜绝非法标识符 / 注入。
///
/// # 错误
///
/// 标识符含非法字符（非 `[a-zA-Z_][a-zA-Z0-9_]*`）时返回
/// [`DbError::InvalidValue`]。
fn quote_identifier(dialect: &dyn Dialect, name: &str) -> Result<String> {
    alun_db_sql::render::validate_identifier(name)?;
    Ok(dialect.quote_identifier(name))
}

impl Db {
    /// 按主键查询单条记录。
    ///
    /// # 参数
    ///
    /// - `table`：表名
    /// - `pk_column`：主键列名
    /// - `pk`：主键值
    ///
    /// # 返回
    ///
    /// 命中的记录行；未命中返回 `None`。
    ///
    /// # 错误
    ///
    /// 表名 / 主键列名不合法时返回 [`DbError::InvalidValue`]。
    pub async fn find_row_by_pk(
        &self,
        table: &str,
        pk_column: &str,
        pk: impl Into<Value>,
    ) -> Result<Option<Row>> {
        let table_q = quote_identifier(self.dialect(), table)?;
        let pk_q = quote_identifier(self.dialect(), pk_column)?;
        let sql = format!("SELECT * FROM {table_q} WHERE {pk_q} = ?");
        self.find_first_with_params(&sql, vec![pk.into()])
            .await
            .map(|opt| opt.map(|r| r.with_entity(table, pk_column)))
    }

    /// 插入动态行并返回完整行（含数据库生成的主键）。
    ///
    /// # 行为
    ///
    /// - 主键值为 `Null`、缺失或数值 `0` 时视为自增：该列不参与 INSERT，
    ///   返回的行由方言回填（支持 `RETURNING` 的方言直接取回，其余方言
    ///   通过自增主键回查）。
    /// - 主键为显式值（如 UUID / 自然键）时参与 INSERT，插入后按主键回查。
    ///
    /// # 错误
    ///
    /// 行缺少表名元数据（[`Row::table`] 为 `None`）或没有任何可插入字段时
    /// 返回 [`DbError::InvalidValue`]。
    pub async fn insert_row(&self, row: &Row) -> Result<Row> {
        let table = row
            .table()
            .ok_or_else(|| DbError::InvalidValue("row 缺少表名元数据（table）".into()))?;
        let table_q = quote_identifier(self.dialect(), table)?;
        let pk = row.primary_key();
        // 自增主键判定：缺失 / Null / 数值 0 视为自增
        let auto_pk = row.get(pk).map(|v| v.is_unset_pk()).unwrap_or(true);

        let columns: Vec<String> = row
            .iter()
            .filter(|(k, _)| !(auto_pk && *k == pk))
            .map(|(k, _)| quote_identifier(self.dialect(), k))
            .collect::<Result<Vec<String>>>()?;
        if columns.is_empty() {
            return Err(DbError::InvalidValue("row 没有可插入字段".into()));
        }
        let placeholders = vec!["?"; columns.len()];
        let sql = format!(
            "INSERT INTO {table_q} ({}) VALUES ({})",
            columns.join(", "),
            placeholders.join(", ")
        );
        let params: Vec<Value> = row
            .iter()
            .filter(|(k, _)| !(auto_pk && *k == pk))
            .map(|(_, v)| v.clone())
            .collect();

        // 支持 RETURNING 的方言直接取回完整行
        if self.dialect().capability().returning_clause {
            let returning_sql = format!("{sql} RETURNING *");
            return self
                .find_first_with_params(&returning_sql, params)
                .await?
                .map(|r| r.with_entity(table, pk))
                .ok_or_else(|| DbError::Query {
                    sql: returning_sql,
                    source: "INSERT ... RETURNING 未返回行".into(),
                });
        }

        // 其余方言：执行后按主键回查
        let pk_value = if auto_pk {
            Value::I64(self.insert_returning_id(&sql, params).await?)
        } else {
            row.get(pk)
                .cloned()
                .ok_or_else(|| DbError::InvalidValue(format!("row 缺少主键 `{pk}` 的值")))?
        };
        let pk_q = quote_identifier(self.dialect(), pk)?;
        let find_sql = format!("SELECT * FROM {table_q} WHERE {pk_q} = ?");
        self.find_first_with_params(&find_sql, vec![pk_value])
            .await?
            .map(|r| r.with_entity(table, pk))
            .ok_or(DbError::NotFound)
    }

    /// 更新动态行（按主键元数据定位），返回受影响行数。
    ///
    /// 主键列不参与 SET；更新字段取行内全部非主键字段。
    ///
    /// # 错误
    ///
    /// 行缺少表名元数据、主键值或可更新字段时返回 [`DbError::InvalidValue`]。
    pub async fn update_row(&self, row: &Row) -> Result<u64> {
        let table = row
            .table()
            .ok_or_else(|| DbError::InvalidValue("row 缺少表名元数据（table）".into()))?;
        let table_q = quote_identifier(self.dialect(), table)?;
        let pk = row.primary_key();
        let pk_q = quote_identifier(self.dialect(), pk)?;
        let pk_value = row
            .get(pk)
            .cloned()
            .ok_or_else(|| DbError::InvalidValue(format!("row 缺少主键 `{pk}` 的值")))?;
        let sets: Vec<String> = row
            .iter()
            .filter(|(k, _)| *k != pk)
            .map(|(k, _)| {
                let q = quote_identifier(self.dialect(), k)?;
                Ok(format!("{q} = ?"))
            })
            .collect::<Result<Vec<String>>>()?;
        if sets.is_empty() {
            return Err(DbError::InvalidValue("row 没有可更新字段".into()));
        }
        let sql = format!("UPDATE {table_q} SET {} WHERE {pk_q} = ?", sets.join(", "));
        let mut params: Vec<Value> = row
            .iter()
            .filter(|(k, _)| *k != pk)
            .map(|(_, v)| v.clone())
            .collect();
        params.push(pk_value);
        self.execute_with_params(&sql, params).await
    }

    /// 删除动态行（按主键元数据定位），返回受影响行数。
    ///
    /// # 错误
    ///
    /// 行缺少表名元数据或主键值时返回 [`DbError::InvalidValue`]。
    pub async fn delete_row(&self, row: &Row) -> Result<u64> {
        let table = row
            .table()
            .ok_or_else(|| DbError::InvalidValue("row 缺少表名元数据（table）".into()))?;
        let table_q = quote_identifier(self.dialect(), table)?;
        let pk = row.primary_key();
        let pk_q = quote_identifier(self.dialect(), pk)?;
        let pk_value = row
            .get(pk)
            .cloned()
            .ok_or_else(|| DbError::InvalidValue(format!("row 缺少主键 `{pk}` 的值")))?;
        let sql = format!("DELETE FROM {table_q} WHERE {pk_q} = ?");
        self.execute_with_params(&sql, vec![pk_value]).await
    }
}

// ==================== 异构批量写 ====================

/// 动态行的批量写扩展。
///
/// 对 `Db` 与 `Transaction` 同时可用（泛化于 [`SqlExecutor`]），因此在事务里
/// 也能批量写且不另开事务。
///
/// ## 与 `PersistExt::insert_many` 的分工
///
/// - `insert_many`：强类型、同构（同一模型，列集天然一致），适合大批量等结构数据；
/// - `batch_insert`：动态行、**异构**（各行列集可不同），对齐 aifei 的
///   `Db.batchInsert(table, List<Row>)`，适合 Excel 导入这类列不齐的场景。
pub trait RowBatchExt: SqlExecutor {
    /// 异构批量插入，返回写入行数。
    ///
    /// 各行的插入列为其自身携带的字段（未赋值的主键列由数据库生成，自动排除）。
    /// 列集相同的行合并为一条多行 `VALUES`（按方言绑定参数上限分块）；多条语句
    /// 时整批包在一个事务里，要么全部写入、要么整体回滚。
    ///
    /// 不触发任何 Model Hook——动态行没有模型元数据；需要 Hook 请走 `insert` /
    /// `insert_or_update`。
    fn batch_insert(&self, table: &str, rows: &[Row]) -> impl Future<Output = Result<u64>> + Send;

    /// 异构批量更新，返回受影响行数之和。
    ///
    /// 每行按自身主键元数据（[`Row::primary_key`]，默认 `id`）定位，只更新该行
    /// 携带的非主键字段；多条语句整批包在一个事务里。
    fn batch_update(&self, table: &str, rows: &[Row]) -> impl Future<Output = Result<u64>> + Send;
}

/// 计算动态行的插入列与值（列名升序，值与之一一对应）。
///
/// 未赋值的主键列（缺失 / `Null` / 数值 `0`）不参与 INSERT，交由数据库生成。
fn row_insert_pairs(row: &Row) -> Result<(Vec<String>, Vec<Value>)> {
    let pk = row.primary_key();
    let pk_unset = row.get(pk).map(|v| v.is_unset_pk()).unwrap_or(true);
    let mut pairs: Vec<(String, Value)> = row
        .iter()
        .filter(|(k, _)| !(pk_unset && *k == pk))
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect();
    if pairs.is_empty() {
        return Err(DbError::InvalidValue("row 没有可插入字段".into()));
    }
    // 升序排列使「列集相同」的判定与书写顺序无关，异构行才能合并成一条语句
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok((
        pairs.iter().map(|(k, _)| k.clone()).collect(),
        pairs.into_iter().map(|(_, v)| v).collect(),
    ))
}

/// 执行一批已渲染语句：单条直连，多条包在一个事务里保证原子性。
async fn run_statements<E: SqlExecutor>(ex: &E, stmts: Vec<(String, Vec<Value>)>) -> Result<u64> {
    if stmts.len() == 1 {
        let (sql, params) = stmts.into_iter().next().expect("len == 1");
        return ex.exec_sql(sql, params).await;
    }
    ex.in_tx(|tx| {
        Box::pin(async move {
            let mut total = 0;
            for (sql, params) in stmts {
                total += tx.exec_sql(sql, params).await?;
            }
            Ok(total)
        })
    })
    .await
}

impl<E: SqlExecutor> RowBatchExt for E {
    async fn batch_insert(&self, table: &str, rows: &[Row]) -> Result<u64> {
        if rows.is_empty() {
            return Ok(0);
        }
        let dialect = self.sql_dialect();
        let table_q = quote_identifier(dialect, table)?;

        // 先按列集分组：列集相同的行合并成一条多行 INSERT
        let mut groups: Vec<(Vec<String>, Vec<Vec<Value>>)> = Vec::new();
        for row in rows {
            let (cols, values) = row_insert_pairs(row)?;
            match groups.iter_mut().find(|(c, _)| *c == cols) {
                Some((_, bucket)) => bucket.push(values),
                None => groups.push((cols, vec![values])),
            }
        }

        let mut stmts: Vec<(String, Vec<Value>)> = Vec::new();
        for (cols, bucket) in &groups {
            let quoted: Vec<String> = cols
                .iter()
                .map(|c| quote_identifier(dialect, c))
                .collect::<Result<Vec<String>>>()?;
            // 按绑定参数上限分块（与 insert_many 同一套限额推导）
            let per_row = cols.len();
            let chunk = (max_bind_params(dialect) / per_row).max(1);
            let row_placeholders = format!("({})", vec!["?"; per_row].join(", "));
            for chunk_rows in bucket.chunks(chunk) {
                let values_sql = vec![row_placeholders.as_str(); chunk_rows.len()].join(", ");
                let sql = format!(
                    "INSERT INTO {table_q} ({}) VALUES {values_sql}",
                    quoted.join(", ")
                );
                let params = chunk_rows.iter().flatten().cloned().collect();
                stmts.push((sql, params));
            }
        }

        run_statements(self, stmts).await
    }

    async fn batch_update(&self, table: &str, rows: &[Row]) -> Result<u64> {
        if rows.is_empty() {
            return Ok(0);
        }
        let dialect = self.sql_dialect();
        let table_q = quote_identifier(dialect, table)?;

        let mut stmts: Vec<(String, Vec<Value>)> = Vec::with_capacity(rows.len());
        for row in rows {
            let pk = row.primary_key();
            let pk_q = quote_identifier(dialect, pk)?;
            let pk_value = row
                .get(pk)
                .cloned()
                .ok_or_else(|| DbError::InvalidValue(format!("row 缺少主键 `{pk}` 的值")))?;
            let sets: Vec<String> = row
                .iter()
                .filter(|(k, _)| *k != pk)
                .map(|(k, _)| Ok(format!("{} = ?", quote_identifier(dialect, k)?)))
                .collect::<Result<Vec<String>>>()?;
            if sets.is_empty() {
                return Err(DbError::InvalidValue("row 没有可更新字段".into()));
            }
            let mut params: Vec<Value> = row
                .iter()
                .filter(|(k, _)| *k != pk)
                .map(|(_, v)| v.clone())
                .collect();
            params.push(pk_value);
            stmts.push((
                format!("UPDATE {table_q} SET {} WHERE {pk_q} = ?", sets.join(", ")),
                params,
            ));
        }

        run_statements(self, stmts).await
    }
}

-- 可重复迁移：验证嵌入来源同样支持 `R__` 语义（内容变化即重跑）。
CREATE VIEW emb_view AS SELECT id, n FROM emb_t1

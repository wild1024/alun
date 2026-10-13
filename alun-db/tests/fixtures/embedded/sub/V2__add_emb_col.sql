-- 放在子目录里，验证 `migrate!` 与目录扫描一样递归收集。
ALTER TABLE emb_t1 ADD COLUMN n INTEGER NOT NULL DEFAULT 0

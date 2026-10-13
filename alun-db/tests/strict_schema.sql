-- 供 `#[model(strict, schema = "...")]` 编译期校验的 DDL fixture。
-- 包含表级约束（PRIMARY KEY）与多列，验证解析器正确处理。

CREATE TABLE widgets (
    id   INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT    NOT NULL,
    qty  INTEGER NOT NULL DEFAULT 0,
    UNIQUE (name)
);

CREATE TABLE sprockets (
    id     INTEGER PRIMARY KEY,
    label  TEXT NOT NULL,
    CHECK (id > 0)
);
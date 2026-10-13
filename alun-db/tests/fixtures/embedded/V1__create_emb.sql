-- 编译期嵌入迁移的测试夹具：`alun_db::migrate!("tests/fixtures/embedded")`。
-- 内容无实际业务含义，只用于验证「宏搬运 + 运行时解析」这条路径。
CREATE TABLE emb_t1 (id INTEGER NOT NULL PRIMARY KEY)

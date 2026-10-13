CREATE TABLE usage_records (id BIGINT NOT NULL PRIMARY KEY, subscription_id BIGINT NOT NULL, usage_date DATETIME(6) NOT NULL, quantity BIGINT NOT NULL, metadata JSON)

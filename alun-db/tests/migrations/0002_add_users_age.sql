-- +migrate Up
ALTER TABLE users ADD COLUMN age INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_users_email ON users(email);

-- +migrate Down
DROP INDEX idx_users_email;
ALTER TABLE users DROP COLUMN age;
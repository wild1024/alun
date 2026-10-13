-- +migrate Up
CREATE TABLE users (
    id          BIGINT PRIMARY KEY,
    name        TEXT NOT NULL,
    email       TEXT NOT NULL UNIQUE,
    deleted_at  TEXT
);

-- +migrate Down
DROP TABLE users;
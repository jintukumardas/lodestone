-- Database is created via the CLICKHOUSE_DB env var, but we ensure it exists
-- and switch context for the remaining migrations.
CREATE DATABASE IF NOT EXISTS kg;

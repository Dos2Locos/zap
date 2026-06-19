-- SQLite supports DROP COLUMN since 3.35 (bundled with libsqlite3-sys here).
ALTER TABLE ssh_servers DROP COLUMN advanced_config;

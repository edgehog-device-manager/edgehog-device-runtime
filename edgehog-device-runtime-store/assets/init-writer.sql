-- writer config
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = true;
-- 64 megabytes default is -1
PRAGMA journal_size_limit = 67108864;
-- Reduces the size of the database
PRAGMA auto_vacuum = INCREMENTAL;
-- Per connection
PRAGMA temp_store = MEMORY;
PRAGMA busy_timeout = 5000;
PRAGMA cache_size = -2000;

VACUUM;

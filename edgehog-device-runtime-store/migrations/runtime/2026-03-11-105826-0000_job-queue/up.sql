CREATE TABLE IF NOT EXISTS job_queue (
    id BLOB NOT NULL,
    job_type INTEGER NOT NULL,
    status INTEGER NOT NULL,
    version INTEGER NOT NULL,
    tag INTEGER NOT NULL,
    data BLOB NOT NULL,
    created_at INTEGER NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (id, job_type)
);

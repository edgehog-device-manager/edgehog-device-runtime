CREATE TABLE IF NOT EXISTS file_binds (
    id BLOB NOT NULL PRIMARY KEY,
    status SMALLINT NOT NULL,
    target_id TEXT NOT NULL,
    target_type INTEGER NOT NULL,
    mountpoint TEXT NOT NULL,
    options TEXT
);

CREATE TABLE IF NOT EXISTS container_file_binds (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    file_bind_id BLOB NOT NULL REFERENCES file_binds (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    PRIMARY KEY (container_id, file_bind_id)
);


CREATE TABLE IF NOT EXISTS container_missing_file_binds (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    file_bind_id BLOB NOT NULL,
    PRIMARY KEY (container_id, file_bind_id)
);

CREATE TABLE IF NOT EXISTS env_files (
    id BLOB NOT NULL PRIMARY KEY,
    status SMALLINT NOT NULL,
    target_id TEXT NOT NULL,
    target_type INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS container_env_files (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    env_file_id BLOB NOT NULL REFERENCES env_files (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    PRIMARY KEY (container_id, env_file_id)
);


CREATE TABLE IF NOT EXISTS container_missing_env_files (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    env_file_id BLOB NOT NULL,
    PRIMARY KEY (container_id, env_file_id)
);

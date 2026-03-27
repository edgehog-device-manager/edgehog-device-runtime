CREATE TABLE IF NOT EXISTS container_port_bindings_v2 (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    port TEXT NOT NULL,
    idx UNSIGNED BIG INT NOT NULL,
    host_ip TEXT,
    host_port INTEGER,
    PRIMARY KEY (container_id, port, idx)
);

INSERT INTO container_port_bindings_v2 (
    container_id,
    port,
    idx,
    host_ip,
    host_port
)
SELECT
    container_id,
    port,
    row_number()
        OVER (PARTITION BY container_id, port ORDER BY port, host_ip, host_port)
    - 1 AS idx,
    host_ip,
    host_port
FROM container_port_bindings;

DROP TABLE container_port_bindings;
ALTER TABLE container_port_bindings_v2 RENAME TO container_port_bindings;

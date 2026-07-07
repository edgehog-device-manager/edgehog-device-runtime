CREATE TABLE IF NOT EXISTS container_port_bindings_old (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    port TEXT NOT NULL,
    host_ip TEXT,
    host_port INTEGER,
    PRIMARY KEY (container_id, port, host_ip, host_port)
);

INSERT OR IGNORE INTO container_port_bindings_old (
    container_id,
    port,
    host_ip,
    host_port
)
SELECT
    container_id,
    port,
    host_ip,
    host_port
FROM container_port_bindings;

DROP TABLE container_port_bindings;
ALTER TABLE container_port_bindings_old RENAME TO container_port_bindings;

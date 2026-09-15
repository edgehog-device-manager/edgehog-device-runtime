CREATE TABLE IF NOT EXISTS device_requests (
    id BLOB NOT NULL PRIMARY KEY,
    status SMALLINT NOT NULL,
    driver TEXT,
    count BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS device_requests_device_ids (
    device_request_id BLOB NOT NULL REFERENCES device_requests (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    device_id TEXT NOT NULL,
    PRIMARY KEY (device_request_id, device_id)
);

CREATE TABLE IF NOT EXISTS device_requests_capabilities (
    device_request_id BLOB NOT NULL REFERENCES device_requests (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    idx INTEGER NOT NULL,
    capability TEXT NOT NULL,
    PRIMARY KEY (device_request_id, idx, capability)
);

CREATE TABLE IF NOT EXISTS device_requests_options (
    device_request_id BLOB NOT NULL REFERENCES device_requests (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    name TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (device_request_id, name)
);

CREATE TABLE IF NOT EXISTS container_device_requests (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    device_request_id BLOB NOT NULL REFERENCES device_requests (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    PRIMARY KEY (container_id, device_request_id)
);
CREATE TABLE IF NOT EXISTS container_missing_device_requests (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON UPDATE CASCADE ON DELETE CASCADE,
    device_request_id BLOB NOT NULL,
    PRIMARY KEY (container_id, device_request_id)
);

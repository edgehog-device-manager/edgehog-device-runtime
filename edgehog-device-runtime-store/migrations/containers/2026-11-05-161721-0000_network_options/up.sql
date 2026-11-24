CREATE TABLE IF NOT EXISTS network_options (
    id BLOB PRIMARY KEY NOT NULL,
    container_id BLOB NOT NULL,
    network_id BLOB NOT NULL,
    status INTEGER NOT NULL,
    ipv4_address TEXT,
    ipv6_address TEXT,
    mac_address TEXT,
    gw_priority BIGINT,
    gateway TEXT,
    ip_address TEXT,
    ip_prefix_len INTEGER,
    ipv6_gateway TEXT,
    global_ipv6_address TEXT,
    global_ipv6_prefix_len BIGINT,

    UNIQUE (network_id, container_id),

    FOREIGN KEY (network_id) REFERENCES networks(id),
    FOREIGN KEY (container_id) REFERENCES containers(id)
);

CREATE TABLE IF NOT EXISTS network_options_link_local_ips (
    network_options_id BLOB NOT NULL,
    ip TEXT NOT NULL,
    PRIMARY KEY (network_options_id, ip),
    FOREIGN KEY (network_options_id)
        REFERENCES network_options(id)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS network_options_links (
    network_options_id BLOB NOT NULL,
    link TEXT NOT NULL,
    PRIMARY KEY (network_options_id, link),
    FOREIGN KEY (network_options_id)
        REFERENCES network_options(id)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS network_options_aliases (
    network_options_id BLOB NOT NULL,
    alias TEXT NOT NULL,
    PRIMARY KEY (network_options_id, alias),
    FOREIGN KEY (network_options_id)
        REFERENCES network_options(id)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS network_options_dns_names (
    network_options_id BLOB NOT NULL,
    dns_name TEXT NOT NULL,
    PRIMARY KEY (network_options_id, dns_name),
    FOREIGN KEY (network_options_id)
        REFERENCES network_options(id)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS network_options_driver_opts (
    network_options_id BLOB NOT NULL,
    option_key TEXT NOT NULL,
    option_value TEXT NOT NULL,
    PRIMARY KEY (network_options_id, option_key),
    FOREIGN KEY (network_options_id)
        REFERENCES network_options(id)
        ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS container_cmds (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    idx UNSIGNED BIG INT NOT NULL,
    cmd TEXT NOT NULL,
    PRIMARY KEY (container_id, idx)
);
CREATE TABLE IF NOT EXISTS container_healthcheck_test (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    idx UNSIGNED BIG INT NOT NULL,
    cmd TEXT NOT NULL,
    PRIMARY KEY (container_id, idx)
);
CREATE TABLE IF NOT EXISTS container_entrypoints (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    idx UNSIGNED BIG INT NOT NULL,
    cmd TEXT NOT NULL,
    PRIMARY KEY (container_id, idx)
);
CREATE TABLE IF NOT EXISTS container_labels (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    'key' TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (container_id, 'key')
);
CREATE TABLE IF NOT EXISTS container_exposed_ports (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    port TEXT NOT NULL,
    PRIMARY KEY (container_id, port)
);
CREATE TABLE IF NOT EXISTS container_device_cgroup_rules (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    rule TEXT NOT NULL,
    PRIMARY KEY (container_id, rule)
);
CREATE TABLE IF NOT EXISTS container_ulimits (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    name TEXT NOT NULL,
    soft INTEGER NOT NULL,
    hard INTEGER NOT NULL,
    PRIMARY KEY (container_id, name)
);
CREATE TABLE IF NOT EXISTS container_dns (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    dns TEXT NOT NULL,
    PRIMARY KEY (container_id, dns)
);
CREATE TABLE IF NOT EXISTS container_dns_options (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    dns_option TEXT NOT NULL,
    PRIMARY KEY (container_id, dns_option)
);
CREATE TABLE IF NOT EXISTS container_dns_search (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    dns_search TEXT NOT NULL,
    PRIMARY KEY (container_id, dns_search)
);
CREATE TABLE IF NOT EXISTS container_group_add (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    group_add TEXT NOT NULL,
    PRIMARY KEY (container_id, group_add)
);
CREATE TABLE IF NOT EXISTS container_sysctls (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    'key' TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (container_id, 'key')
);
CREATE TABLE IF NOT EXISTS container_log_config (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    'key' TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (container_id, 'key')
);
CREATE TABLE IF NOT EXISTS container_blkio_weight_device (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    'path' TEXT NOT NULL,
    weight INTEGER NOT NULL,
    PRIMARY KEY (container_id, 'path')
);
CREATE TABLE IF NOT EXISTS container_blkio_device_read_bps (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    'path' TEXT NOT NULL,
    rate BIGINT NOT NULL,
    PRIMARY KEY (container_id, 'path')
);
CREATE TABLE IF NOT EXISTS container_blkio_device_write_bps (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    'path' TEXT NOT NULL,
    rate BIGINT NOT NULL,
    PRIMARY KEY (container_id, 'path')
);
CREATE TABLE IF NOT EXISTS container_blkio_device_read_iops (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    'path' TEXT NOT NULL,
    rate BIGINT NOT NULL,
    PRIMARY KEY (container_id, 'path')
);
CREATE TABLE IF NOT EXISTS container_blkio_device_write_iops (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    'path' TEXT NOT NULL,
    rate BIGINT NOT NULL,
    PRIMARY KEY (container_id, 'path')
);
CREATE TABLE IF NOT EXISTS container_securityopts (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    opt TEXT NOT NULL,
    PRIMARY KEY (container_id, opt)
);
CREATE TABLE IF NOT EXISTS container_masked_paths (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    'path' TEXT NOT NULL,
    PRIMARY KEY (container_id, 'path')
);
CREATE TABLE IF NOT EXISTS container_readonly_paths (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    'path' TEXT NOT NULL,
    PRIMARY KEY (container_id, 'path')
);

ALTER TABLE containers ADD COLUMN domainname TEXT;
ALTER TABLE containers ADD COLUMN user TEXT;
ALTER TABLE containers ADD COLUMN health_check_interval UNSIGNED BIG INT;
ALTER TABLE containers ADD COLUMN health_check_timeout UNSIGNED BIG INT;
ALTER TABLE containers ADD COLUMN health_check_retries INTEGER;
ALTER TABLE containers ADD COLUMN health_check_start_period UNSIGNED BIG INT;
ALTER TABLE containers ADD COLUMN health_check_start_interval UNSIGNED BIG INT;
ALTER TABLE containers ADD COLUMN working_dir TEXT;
ALTER TABLE containers ADD COLUMN network_disabled BOOLEAN;
ALTER TABLE containers ADD COLUMN stop_signal TEXT;
ALTER TABLE containers ADD COLUMN stop_timeout INTEGER;
ALTER TABLE containers ADD COLUMN restart_policy_maximum_retry_count INTEGER;
ALTER TABLE containers ADD COLUMN cpu_shares INTEGER;
ALTER TABLE containers ADD COLUMN cpuset_cpus TEXT;
ALTER TABLE containers ADD COLUMN auto_remove BOOLEAN;
ALTER TABLE containers ADD COLUMN cgroupns_mode SMALLINT;
ALTER TABLE containers ADD COLUMN ipc_mode TEXT;
ALTER TABLE containers ADD COLUMN oom_score_adj INTEGER;
ALTER TABLE containers ADD COLUMN userns_mode TEXT;
ALTER TABLE containers ADD COLUMN shm_size BIGINT;
ALTER TABLE containers ADD COLUMN log_type TEXT;
ALTER TABLE containers ADD COLUMN blkio_weight SMALLINT;
ALTER TABLE containers ADD COLUMN pid_mode TEXT;
ALTER TABLE containers ADD COLUMN runtime TEXT;

ALTER TABLE containers ADD COLUMN network_mode_v2 TEXT;
UPDATE containers SET network_mode_v2 = network_mode;
ALTER TABLE containers DROP COLUMN network_mode;
ALTER TABLE containers RENAME COLUMN network_mode_v2 TO network_mode;

ALTER TABLE containers ADD COLUMN restart_policy_v2 INTEGER;
UPDATE containers SET restart_policy_v2 = restart_policy;
ALTER TABLE containers DROP COLUMN restart_policy;
ALTER TABLE containers RENAME COLUMN restart_policy_v2 TO restart_policy;

ALTER TABLE containers ADD COLUMN read_only_rootfs_v2 BOOLEAN;
UPDATE containers SET read_only_rootfs_v2 = read_only_rootfs;
ALTER TABLE containers DROP COLUMN read_only_rootfs;
ALTER TABLE containers RENAME COLUMN read_only_rootfs_v2 TO read_only_rootfs;

ALTER TABLE containers ADD COLUMN privileged_v2 BOOLEAN;
UPDATE containers SET privileged_v2 = privileged;
ALTER TABLE containers DROP COLUMN privileged;
ALTER TABLE containers RENAME COLUMN privileged_v2 TO privileged;

ALTER TABLE device_requests ADD COLUMN count_v2 BIGINT;
UPDATE device_requests SET count_v2 = count;
ALTER TABLE device_requests DROP COLUMN count;
ALTER TABLE device_requests RENAME COLUMN count_v2 TO count;

ALTER TABLE device_requests ADD COLUMN count_v2 BIGINT;
UPDATE device_requests SET count_v2 = count;
ALTER TABLE device_requests DROP COLUMN count;
ALTER TABLE device_requests RENAME COLUMN count_v2 TO count;

ALTER TABLE container_tmpfs ADD COLUMN options_v2 TEXT NOT NULL;
UPDATE container_tmpfs SET options_v2 = IFNULL(options, "");
ALTER TABLE container_tmpfs DROP COLUMN options;
ALTER TABLE container_tmpfs RENAME COLUMN options_v2 TO options;

ALTER TABLE container_storage_options ADD COLUMN value_v2 TEXT NOT NULL;
UPDATE container_storage_options SET value_v2 = IFNULL(value, "");
ALTER TABLE container_storage_options DROP COLUMN value;
ALTER TABLE container_storage_options RENAME COLUMN value_v2 TO value;

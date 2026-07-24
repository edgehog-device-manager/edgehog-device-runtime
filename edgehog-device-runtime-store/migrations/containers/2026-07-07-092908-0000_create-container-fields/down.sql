ALTER TABLE container_tmpfs ADD COLUMN options_v2 TEXT;
UPDATE container_tmpfs SET options_v2 = options;
ALTER TABLE container_tmpfs DROP COLUMN options;
ALTER TABLE container_tmpfs RENAME COLUMN options_v2 TO options;

ALTER TABLE container_storage_options ADD COLUMN value_v2 TEXT;
UPDATE container_storage_options SET value_v2 = value;
ALTER TABLE container_storage_options DROP COLUMN value;
ALTER TABLE container_storage_options RENAME COLUMN value_v2 TO value;

UPDATE device_requests SET count = 0
WHERE count IS null;
ALTER TABLE device_requests ADD COLUMN count_v2 BIGINT NOT NULL;
UPDATE device_requests SET count_v2 = count;
ALTER TABLE device_requests DROP COLUMN count;
ALTER TABLE device_requests RENAME COLUMN count_v2 TO count;

UPDATE containers SET privileged = false
WHERE privileged IS null;
ALTER TABLE containers ADD COLUMN privileged_v2 BOOLEAN NOT NULL;
UPDATE containers SET privileged_v2 = privileged;
ALTER TABLE containers DROP COLUMN privileged;
ALTER TABLE containers RENAME COLUMN privileged_v2 TO privileged;

UPDATE containers SET read_only_rootfs = false
WHERE read_only_rootfs IS null;
ALTER TABLE containers ADD COLUMN read_only_rootfs_v2 BOOLEAN NOT NULL;
UPDATE containers SET read_only_rootfs_v2 = read_only_rootfs;
ALTER TABLE containers DROP COLUMN read_only_rootfs;
ALTER TABLE containers RENAME COLUMN read_only_rootfs_v2 TO read_only_rootfs;

UPDATE containers SET restart_policy = ""
WHERE restart_policy IS null;
ALTER TABLE containers ADD COLUMN restart_policy_v2 INTEGER NOT NULL;
UPDATE containers SET restart_policy_v2 = restart_policy;
ALTER TABLE containers DROP COLUMN restart_policy;
ALTER TABLE containers RENAME COLUMN restart_policy_v2 TO restart_policy;

UPDATE containers SET network_mode = "bridge"
WHERE network_mode IS null;
ALTER TABLE containers ADD COLUMN network_mode_v2 TEXT NOT NULL;
UPDATE containers SET network_mode_v2 = network_mode;
ALTER TABLE containers DROP COLUMN network_mode;
ALTER TABLE containers RENAME COLUMN network_mode_v2 TO network_mode;

ALTER TABLE containers DROP COLUMN runtime;
ALTER TABLE containers DROP COLUMN pid_mode;
ALTER TABLE containers DROP COLUMN blkio_weight;
ALTER TABLE containers DROP COLUMN log_type;
ALTER TABLE containers DROP COLUMN shm_size;
ALTER TABLE containers DROP COLUMN userns_mode;
ALTER TABLE containers DROP COLUMN oom_score_adj;
ALTER TABLE containers DROP COLUMN ipc_mode;
ALTER TABLE containers DROP COLUMN cgroupns_mode;
ALTER TABLE containers DROP COLUMN auto_remove;
ALTER TABLE containers DROP COLUMN cpuset_cpus;
ALTER TABLE containers DROP COLUMN cpu_shares;
ALTER TABLE containers DROP COLUMN restart_policy_maximum_retry_count;
ALTER TABLE containers DROP COLUMN stop_timeout;
ALTER TABLE containers DROP COLUMN stop_signal;
ALTER TABLE containers DROP COLUMN network_disabled;
ALTER TABLE containers DROP COLUMN working_dir;
ALTER TABLE containers DROP COLUMN health_check_start_interval;
ALTER TABLE containers DROP COLUMN health_check_start_period;
ALTER TABLE containers DROP COLUMN health_check_retries;
ALTER TABLE containers DROP COLUMN health_check_timeout;
ALTER TABLE containers DROP COLUMN health_check_interval;
ALTER TABLE containers DROP COLUMN user;
ALTER TABLE containers DROP COLUMN domainname;

DROP TABLE IF EXISTS container_readonly_paths;
DROP TABLE IF EXISTS container_masked_paths;
DROP TABLE IF EXISTS container_securityopts;
DROP TABLE IF EXISTS container_blkio_device_write_iops;
DROP TABLE IF EXISTS container_blkio_device_read_iops;
DROP TABLE IF EXISTS container_blkio_device_write_bps;
DROP TABLE IF EXISTS container_blkio_device_read_bps;
DROP TABLE IF EXISTS container_blkio_weight_device;
DROP TABLE IF EXISTS container_log_config;
DROP TABLE IF EXISTS container_sysctls;
DROP TABLE IF EXISTS container_group_add;
DROP TABLE IF EXISTS container_dns_search;
DROP TABLE IF EXISTS container_dns_options;
DROP TABLE IF EXISTS container_dns;
DROP TABLE IF EXISTS container_ulimits;
DROP TABLE IF EXISTS container_device_cgroup_rules;
DROP TABLE IF EXISTS container_exposed_ports;
DROP TABLE IF EXISTS container_labels;
DROP TABLE IF EXISTS container_entrypoints;
DROP TABLE IF EXISTS container_healthcheck_test;
DROP TABLE IF EXISTS container_cmds;

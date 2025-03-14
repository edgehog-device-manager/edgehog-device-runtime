-- This file is part of Edgehog.
--
-- Copyright 2025 SECO Mind Srl
--
-- Licensed under the Apache License, Version 2.0 (the "License");
-- you may not use this file except in compliance with the License.
-- You may obtain a copy of the License at
--
--    http://www.apache.org/licenses/LICENSE-2.0
--
-- Unless required by applicable law or agreed to in writing, software
-- distributed under the License is distributed on an "AS IS" BASIS,
-- WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
-- See the License for the specific language governing permissions and
-- limitations under the License.
--
-- SPDX-License-Identifier: Apache-2.0

-- Your SQL goes here
CREATE TABLE IF NOT EXISTS container_extra_hosts (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    value TEXT NOT NULL,
    PRIMARY KEY (container_id, value)
);
CREATE TABLE IF NOT EXISTS container_add_capabilities (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    value TEXT NOT NULL,
    PRIMARY KEY (container_id, value)
);
CREATE TABLE IF NOT EXISTS container_drop_capabilities (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    value TEXT NOT NULL,
    PRIMARY KEY (container_id, value)
);
CREATE TABLE IF NOT EXISTS container_storage_options (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    name TEXT NOT NULL,
    value TEXT,
    PRIMARY KEY (container_id, name)
);
CREATE TABLE IF NOT EXISTS container_tmpfs (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    path TEXT NOT NULL,
    options TEXT,
    PRIMARY KEY (container_id, path)
);
CREATE TABLE IF NOT EXISTS device_mappings (
    id BLOB PRIMARY KEY NOT NULL,
    status SMALLINT NOT NULL,
    path_on_host TEXT NOT NULL,
    path_in_container TEXT NOT NULL,
    cgroup_permissions TEXT
);
CREATE TABLE IF NOT EXISTS container_device_mappings (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    device_mapping_id BLOB NOT NULL REFERENCES device_mappings (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    PRIMARY KEY (container_id, device_mapping_id)
);
CREATE TABLE IF NOT EXISTS container_missing_device_mappings (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON UPDATE CASCADE ON DELETE CASCADE,
    device_mapping_id BLOB NOT NULL,
    PRIMARY KEY (container_id, device_mapping_id)
);
ALTER TABLE containers ADD COLUMN cpu_period UNSIGNED BIG INT;
ALTER TABLE containers ADD COLUMN cpu_quota UNSIGNED BIG INT;
ALTER TABLE containers ADD COLUMN cpu_realtime_period UNSIGNED BIG INT;
ALTER TABLE containers ADD COLUMN cpu_realtime_runtime UNSIGNED BIG INT;
ALTER TABLE containers ADD COLUMN memory UNSIGNED BIG INT;
ALTER TABLE containers ADD COLUMN memory_reservation UNSIGNED BIG INT;
ALTER TABLE containers ADD COLUMN memory_swap BIG INT;
ALTER TABLE containers ADD COLUMN memory_swappiness UNSIGNED SMALL INT;
ALTER TABLE containers ADD COLUMN volume_driver TEXT;
ALTER TABLE containers ADD COLUMN read_only_rootfs BOOLEAN NOT NULL;

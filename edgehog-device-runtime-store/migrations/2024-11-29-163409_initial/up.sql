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

CREATE TABLE IF NOT EXISTS images (
    id BLOB PRIMARY KEY NOT NULL,
    local_id TEXT,
    status INTEGER NOT NULL,
    reference TEXT NOT NULL,
    registry_auth TEXT
);

CREATE TABLE IF NOT EXISTS networks (
    id BLOB PRIMARY KEY NOT NULL,
    local_id TEXT,
    status INTEGER NOT NULL,
    driver TEXT NOT NULL,
    internal BOOLEAN NOT NULL,
    enable_ipv6 BOOLEAN NOT NULL
);

CREATE TABLE IF NOT EXISTS network_driver_opts (
    network_id BLOB NOT NULL,
    name TEXT NOT NULL,
    value TEXT,
    PRIMARY KEY (network_id, name),
    FOREIGN KEY (network_id) REFERENCES networks (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE IF NOT EXISTS volumes (
    id BLOB PRIMARY KEY NOT NULL,
    status INTEGER NOT NULL,
    driver TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS volume_driver_opts (
    volume_id BLOB NOT NULL,
    name TEXT NOT NULL,
    value TEXT,
    PRIMARY KEY (volume_id, name),
    FOREIGN KEY (volume_id) REFERENCES volumes (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE IF NOT EXISTS containers (
    id BLOB PRIMARY KEY NOT NULL,
    local_id TEXT,
    image_id BLOB REFERENCES images (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    status INTEGER NOT NULL,
    network_mode TEXT NOT NULL,
    hostname TEXT NOT NULL,
    restart_policy INTEGER NOT NULL,
    privileged BOOLEAN NOT NULL
);

CREATE TABLE IF NOT EXISTS container_missing_images (
    container_id BLOB PRIMARY KEY NOT NULL REFERENCES containers (
        id
    ) ON UPDATE CASCADE ON DELETE CASCADE,
    image_id BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS container_networks (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    network_id BLOB NOT NULL REFERENCES networks (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    PRIMARY KEY (container_id, network_id)
);

CREATE TABLE IF NOT EXISTS container_missing_networks (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON UPDATE CASCADE ON DELETE CASCADE,
    network_id BLOB NOT NULL,
    PRIMARY KEY (container_id, network_id)
);

CREATE TABLE IF NOT EXISTS container_volumes (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    volume_id BLOB NOT NULL REFERENCES volumes (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    PRIMARY KEY (container_id, volume_id)
);

CREATE TABLE IF NOT EXISTS container_missing_volumes (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON UPDATE CASCADE ON DELETE CASCADE,
    volume_id BLOB NOT NULL,
    PRIMARY KEY (container_id, volume_id)
);

CREATE TABLE IF NOT EXISTS container_env (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    value TEXT NOT NULL,
    PRIMARY KEY (container_id, value)
);

CREATE TABLE IF NOT EXISTS container_binds (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    value TEXT NOT NULL,
    PRIMARY KEY (container_id, value)
);

CREATE TABLE IF NOT EXISTS container_port_bindings (
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    port TEXT NOT NULL,
    host_ip TEXT,
    host_port INTEGER,
    PRIMARY KEY (container_id, port, host_ip, host_port)
);

CREATE TABLE IF NOT EXISTS deployments (
    id BLOB NOT NULL PRIMARY KEY,
    status INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS deployment_containers (
    deployment_id BLOB NOT NULL REFERENCES deployments (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    container_id BLOB NOT NULL REFERENCES containers (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    PRIMARY KEY (deployment_id, container_id)
);

CREATE TABLE IF NOT EXISTS deployment_missing_containers (
    deployment_id BLOB NOT NULL REFERENCES deployments (
        id
    ) ON DELETE CASCADE ON UPDATE CASCADE,
    container_id BLOB NOT NULL,
    PRIMARY KEY (deployment_id, container_id)
);

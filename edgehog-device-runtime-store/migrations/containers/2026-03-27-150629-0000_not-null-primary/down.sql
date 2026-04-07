-- This file is part of Edgehog.
--
-- Copyright 2026 SECO Mind Srl
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

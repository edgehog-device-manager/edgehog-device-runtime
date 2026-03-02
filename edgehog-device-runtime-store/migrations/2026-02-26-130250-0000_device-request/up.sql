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

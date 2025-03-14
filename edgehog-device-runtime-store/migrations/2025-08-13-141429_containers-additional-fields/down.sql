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

-- This file should undo anything in `up.sql`
ALTER TABLE containers DROP COLUMN read_only_rootfs;
ALTER TABLE containers DROP COLUMN volume_driver;
ALTER TABLE containers DROP COLUMN memory_swappiness;
ALTER TABLE containers DROP COLUMN memory_swap;
ALTER TABLE containers DROP COLUMN memory_reservation;
ALTER TABLE containers DROP COLUMN memory;
ALTER TABLE containers DROP COLUMN cpu_realtime_runtime;
ALTER TABLE containers DROP COLUMN cpu_realtime_period;
ALTER TABLE containers DROP COLUMN cpu_quota;
ALTER TABLE containers DROP COLUMN cpu_period;
DROP TABLE container_missing_device_mappings;
DROP TABLE container_device_mappings;
DROP TABLE device_mappings;
DROP TABLE container_tmpfs;
DROP TABLE container_storage_options;
DROP TABLE container_drop_capabilities;
DROP TABLE container_add_capabilities;
DROP TABLE container_extra_hosts;

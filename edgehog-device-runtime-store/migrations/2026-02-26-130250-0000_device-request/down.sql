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

-- This file should undo anything in `up.sql`
DROP TABLE container_missing_device_requests;
DROP TABLE container_device_requests;
DROP TABLE container_device_requests;
DROP TABLE device_requests_options;
DROP TABLE device_requests_capabilities;
DROP TABLE device_requests_device_ids;
DROP TABLE device_requests;

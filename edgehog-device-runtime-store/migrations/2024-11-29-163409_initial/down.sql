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
DROP TABLE deployment_containers;
DROP TABLE deployments;
DROP TABLE container_port_bindings;
DROP TABLE container_binds;
DROP TABLE container_env;
DROP TABLE container_missing_images;
DROP TABLE container_missing_networks;
DROP TABLE container_missing_volumes;
DROP TABLE container_volumes;
DROP TABLE container_networks;
DROP TABLE containers;
DROP TABLE volume_driver_opts;
DROP TABLE volumes;
DROP TABLE network_driver_opts;
DROP TABLE networks;
DROP TABLE images;

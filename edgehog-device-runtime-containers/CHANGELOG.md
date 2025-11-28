<!--
This file is part of Edgehog.

Copyright 2025 SECO Mind Srl

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

   http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.

SPDX-License-Identifier: Apache-2.0
-->

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.10.1] - 2025-11-28

## [0.10.0] - 2025-11-05

### Changed

- Implement the new Container features and statistics.

## [0.9.1] - 2025-09-12

### Changed

- Forward port release v0.8.4
  [#596](https://github.com/edgehog-device-manager/edgehog-device-runtime/pull/596)

## [0.9.0] - 2025-03-11

### Added

- Store the information into a SQLite database
- Create the container service to manage and deploy containers
- Added support for the following container interfaces:
  - `io.edgehog.devicemanager.apps.AvailableContainers`
  - `io.edgehog.devicemanager.apps.AvailableDeployments`
  - `io.edgehog.devicemanager.apps.AvailableImages`
  - `io.edgehog.devicemanager.apps.AvailableNetworks`
  - `io.edgehog.devicemanager.apps.AvailableVolumes`
  - `io.edgehog.devicemanager.apps.CreateContainerRequest`
  - `io.edgehog.devicemanager.apps.CreateDeploymentRequest`
  - `io.edgehog.devicemanager.apps.CreateImageRequest`
  - `io.edgehog.devicemanager.apps.CreateNetworkRequest`
  - `io.edgehog.devicemanager.apps.CreateVolumeRequest`
  - `io.edgehog.devicemanager.apps.DeploymentCommand`
  - `io.edgehog.devicemanager.apps.DeploymentEvent`
  - `io.edgehog.devicemanager.apps.DeploymentUpdate`

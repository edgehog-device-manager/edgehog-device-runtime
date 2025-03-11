# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.9.0] - Unreleased

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

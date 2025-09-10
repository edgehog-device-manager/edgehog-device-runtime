# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Wait for reboot to happen before continuing the OTA procedure.
  [#591](https://github.com/edgehog-device-manager/edgehog-device-runtime/pull/591)

## [0.7.2] - 2024-05-28

### Fixed

- Update sdk dependency to fix a purge property bug
  [#341](https://github.com/astarte-platform/astarte-device-sdk-rust/issues/341)

## [0.7.1] - 2023-07-03

### Added

- Add Astarte Message Hub library support.

## [0.7.0] - 2023-06-05

### Added

- Add support for `io.edgehog.devicemanager.OTAEvent` interface.
- Add support for update/cancel operation in `io.edgehog.devicemanager.OTARequest` interface.

### Removed

- Remove support for `io.edgehog.devicemanager.OTAResponse` interface.

## [0.6.0] - 2023-02-10

### Changed

- Update Astarte Device SDK to 0.5.1 release.

## [0.5.0] - 2022-10-10

### Added

- Initial Edgehog Device Runtime release.

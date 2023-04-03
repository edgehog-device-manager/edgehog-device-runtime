<!---
  Copyright 2022 SECO Mind Srl

  SPDX-License-Identifier: Apache-2.0
-->

# OS Requirements

Edgehog Device Runtime has a number of requirements in order to provide device management features.

## Linux

### Dependencies
* **Rust** >= 1.59
* **libsystemd** (optional)
* **libudev**: Gathering information about network interfaces.

### Runtime Dependencies
* **[dbus](https://www.freedesktop.org/wiki/Software/dbus/)** (optional): Needed for communicating with 3rd party services, such as RAUC.
* **[RAUC](https://rauc.io/) ~> v1.5** (optional): Needed for OS updates.
* **[UPower](https://upower.freedesktop.org/)**: (optional) Needed to gather information about the battery status.

### Filesystem Layout
* **/tmp**: Software updates will be downloaded here.
* **/data**: Edgehog Device Runtime will store its state here during the OTA update process.
* **[/etc/os-release](https://www.freedesktop.org/software/systemd/man/os-release.html)**: NAME,
  VERSION_ID, BUILD_ID, IMAGE_ID, IMAGE_VERSION entries are used for OSInfo and BaseImage.

### Optional features
* **systemd**: If `edgehog-device-runtime` is a `systemd` service, it can notify `systemd` of its status changes. This is provided via the `rust-systemd` crate, a Rust interface to `libsystemd/libelogind` APIs.
To build the `runtime` make sure you have `libsystemd-dev` installed on your system and the systemd feature enabled.

      ```shell
      cargo build --features systemd
      ```

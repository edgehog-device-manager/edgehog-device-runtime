<!---
  Copyright 2022 SECO Mind Srl

  SPDX-License-Identifier: Apache-2.0
-->

# OS Requirements

Edgehog Device Runtime has a number of requirements in order to provide device management features.

## Linux

### Dependencies

- **Rust** >= 1.72.0
- **libsystemd** (optional)
- **libudev**: Gathering information about network interfaces.

### Runtime Dependencies

- **[dbus](https://www.freedesktop.org/wiki/Software/dbus/)** (optional): Needed for communicating
  with 3rd party services, such as RAUC.
- **[RAUC](https://rauc.io/) ~> v1.5** (optional): Needed for OS updates.
- **[UPower](https://upower.freedesktop.org/)**: (optional) Needed to gather information about the
  battery status.

### Filesystem Layout

- **/tmp**: Software updates will be downloaded here.
- **/data**: Edgehog Device Runtime will store its state here during the OTA update process.
- **[/etc/os-release](https://www.freedesktop.org/software/systemd/man/os-release.html)**: NAME,
  VERSION_ID, BUILD_ID, IMAGE_ID, IMAGE_VERSION entries are used for OSInfo and BaseImage.

### Optional features

- **systemd**: If `edgehog-device-runtime` is a `systemd` service, it can notify `systemd` of its
  status changes. This is provided via the `rust-systemd` crate, a Rust interface to
  `libsystemd/libelogind` APIs. To build the `runtime` make sure you have `libsystemd-dev` installed
  on your system and the systemd feature enabled.
  ```sh
  cargo build --features systemd
  ```
- **[ttyd](https://github.com/tsl0922/ttyd)**: command-line tool for sharing terminal over the web.
  To provide the remote terminal functionality, the program must run on the default port `7681` with
  the flag `-W` enabled, which enables the write-mode on the remote terminal.
  To start ttyd locally run the command `ttyd -W bash`.
  Note: It is possible to specify a different kind of shell, depending on the ones installed.
  To build the `runtime` make sure you have ttyd installed on your system and the forwarder feature enabled.
  ```sh
  cargo build --features forwarder
  ```
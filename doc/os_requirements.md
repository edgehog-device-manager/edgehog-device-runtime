<!--
This file is part of Edgehog.

Copyright 2022 - 2025 SECO Mind Srl

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

# OS Requirements

Edgehog Device Runtime has a number of requirements in order to provide device management features.

## Linux

### Dependencies

- **Rust** >= 1.72.0
- **libsystemd** (optional)
- **libudev**: Gathering information about network interfaces.
- **ttyd** >= 1.7.4

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

#### Systemd

If `edgehog-device-runtime` is a `systemd` service, it can notify `systemd` of its status changes.
This is provided via the `rust-systemd` crate, a Rust interface to `libsystemd/libelogind` APIs. To
build the `runtime` make sure you have `libsystemd-dev` installed on your system and the systemd
feature enabled.

```sh
cargo build --features systemd
```

#### Forwarder

The forwarder requires [ttyd](https://github.com/tsl0922/ttyd) for sharing terminal over the web.

To provide the remote terminal functionality, the program must run on the default port `7681` with
the flag `-W` enabled, which enables the write-mode on the remote terminal. To start ttyd locally
run the command `ttyd -W bash` (the `-W` option is only available for ttyd >= 1.7.4). Note: It is
possible to specify a different kind of shell, depending on the ones installed. To build the
`runtime` make sure you have ttyd installed on your system and the forwarder feature enabled.

```sh
cargo build --features forwarder
```

#### Containers

To enable the container service you'll need to enable the `containers` features and a container
runtime with a [Docker compatible REST API](https://docs.docker.com/reference/api/engine/).

To specify the host where the container you need to export the `DOCKER_HOST` environment variable,
pointing to either a Docker API Endpoint or a unix domain socket.

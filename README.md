<!---
  Copyright 2022 SECO Mind Srl

  SPDX-License-Identifier: Apache-2.0
-->

# Edgehog Device Runtime

[![CI](https://github.com/edgehog-device-manager/edgehog-device-runtime/actions/workflows/ci.yaml/badge.svg?branch=main)](https://github.com/edgehog-device-manager/edgehog-device-runtime/actions/workflows/ci.yaml?branch=main)
[![codecov](https://codecov.io/gh/edgehog-device-manager/edgehog-device-runtime/branch/main/graph/badge.svg)](https://app.codecov.io/gh/edgehog-device-manager)

Edgehog Device Runtime is a portable middleware written in [Rust](https://www.rust-lang.org/), that
enables remote device management using [Edgehog](https://github.com/edgehog-device-manager/edgehog).

## Supported Operating System

At the moment only Linux-based systems are supported.

See also [OS requirements](doc/os_requirements.md) for further information.

## Implemented Features

The following information are sent to remote Edgehog instance:

- OS info (data is read from `/etc/os-release`)
- Hardware info
- System status (data is read from proc filesystem)
- Runtime info and compiler version
- OTA update using RAUC
- `Edgehog Device Runtime` status changes via systemd.
- Network interface info
- Base image (data is read from `/etc/os-release`)
- Battery status data
- Remote Terminal

## How it Works

Edgehog Device Runtime relies on [Astarte](https://github.com/astarte-platform/astarte) in order to
communicate with the remote Edgehog instance.

Edgehog Device Runtime is a reference implementation of
[Edgehog Astarte Interfaces](https://github.com/edgehog-device-manager/edgehog-astarte-interfaces).
Astarte interfaces describe how data are exchanged with the remote instance, and what kind of
features are implemented.

## Configuration

Edgehog Device Runtime can be configured using a [TOML](https://en.wikipedia.org/wiki/TOML) file
located either in $PWD/edgehog-config.toml or /etc/edgehog/config.toml, or in a custom path, run
`cargo run -- --help` for more information.

### Supported Astarte transport libraries

Edgehog Device Runtime supports the following libraries to communicate with the remote Edgehog
instance:

1. `astarte-device-sdk`
2. `astarte-message-hub`

#### 1. [Astarte Device SDK Rust](https://github.com/astarte-platform/astarte-device-sdk-rust)

The Astarte Device SDK for Rust is a ready to use library that provides communication and pairing
primitives to an Astarte Cluster.

Example configuration:

```toml
astarte_library = "astarte-device-sdk"
interfaces_directory = "/usr/share/edgehog/astarte-interfaces/"
store_directory = "/var/lib/edgehog/"
download_directory = "/var/tmp/edgehog-updates/"
[astarte_device_sdk]
credentials_secret = "YOUR_CREDENTIAL_SECRET"
device_id = "YOUR_UNIQUE_DEVICE_ID"
pairing_url = "https://api.astarte.EXAMPLE.COM/pairing"
realm = "examplerealm"
[[telemetry_config]]
interface_name = "io.edgehog.devicemanager.SystemStatus"
enabled = true
period = 60
```

#### [Astarte Message Hub](https://github.com/astarte-platform/astarte-message-hub)

A central service that runs on (Linux) devices for collecting and delivering messages from N apps
using 1 MQTT connection to Astarte.

**N.B.** When using this option, the Astarte Message Hub should already be installed and running on
your system.

Example configuration:

```toml
astarte_library = "astarte-message-hub"
interfaces_directory = "/usr/share/edgehog/astarte-interfaces/"
store_directory = "/var/lib/edgehog/"
download_directory = "/var/tmp/edgehog-updates/"
[astarte_message_hub]
endpoint = "http://[::1]:50051"
[[telemetry_config]]
interface_name = "io.edgehog.devicemanager.SystemStatus"
enabled = true
period = 60
```

## Telemetry

Edgehog Device Runtime sends telemetry data from interfaces defined in the
[edgehog-astarte-interfaces](https://github.com/edgehog-device-manager/edgehog-astarte-interfaces)
repository. Here's how to configure some key values.

### Image ID and Version

The device runtime extracts the image name and version from the `/etc/os-release` file. Example:

```sh
# /etc/os-release
IMAGE_ID="..."
IMAGE_VERSION="..."
```

### Serial and Part Number

Set the model and part number as environment variables:

- `EDGEHOG_SYSTEM_SERIAL_NUMBER`
- `EDGEHOG_SYSTEM_PART_NUMBER`

For example, in a systemd service file, refer to
[this buildroot package](https://github.com/edgehog-device-manager/edgehog-buildroot-packages/blob/d3fdb188b7c683d3951c255d32ee2781be416e83/package/edgehog-device-runtime/edgehog-device-runtime.service#L17-L18).

## Contributing

We are open to any contribution:
[pull requests](https://github.com/edgehog-device-manager/edgehog-device-runtime/pulls),
[bug reports and feature requests](https://github.com/edgehog-device-manager/edgehog-device-runtime/issues)
are welcome.

## License

Edgehog Device Runtime source code is released under the Apache 2.0 License.

Check the [LICENSE](LICENSE) file for more information.

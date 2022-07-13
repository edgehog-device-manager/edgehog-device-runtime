<!---
  Copyright 2022 SECO Mind Srl

  SPDX-License-Identifier: Apache-2.0
-->

# Edgehog Device Runtime

![](https://github.com/edgehog-device-manager/edgehog-device-runtime/actions/workflows/build.yaml/badge.svg?branch=main)
[![codecov](https://codecov.io/gh/edgehog-device-manager/edgehog-device-runtime/branch/main/graph/badge.svg)](https://app.codecov.io/gh/edgehog-device-manager)

Edgehog Device Runtime is a portable middleware written in [Rust](https://www.rust-lang.org/), that
enables remote device management using
[Edgehog](https://github.com/edgehog-device-manager/edgehog).

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

## How it Works

Edgehog Device Runtime relies on [Astarte](https://github.com/astarte-platform/astarte) in order to
communicate with the remote Edgehog instance.

Edgehog Device Runtime is a reference implementation of
[Edgehog Astarte Interfaces](https://github.com/edgehog-device-manager/edgehog-astarte-interfaces).
Astarte interfaces describe how data are exchanged with the remote instance, and what kind of
features are implemented.

## Configuration

Edgehog Device Runtime can be configured using a [TOML](https://en.wikipedia.org/wiki/TOML) file located either in $PWD/edgehog-config.toml or /etc/edgehog/config.toml, or in a custom path, run `cargo run -- --help` for more informations.

Example configuration:
```toml
credentials_secret = "YOUR_CREDENTIAL_SECRET"
device_id = "YOUR_UNIQUE_DEVIDE_ID"
pairing_url = "https://api.astarte.EXAMPLE.COM/pairing"
realm = "examplerealm"
interfaces_directory = "/usr/share/edgehog/astarte-interfaces/"
store_directory = "/var/lib/edgehog/"
download_directory = "/var/tmp/edgehog-updates/"
```

## Contributing

We are open to any contribution:
[pull requests](https://github.com/edgehog-device-manager/edgehog-device-runtime/pulls),
[bug reports and feature requests](https://github.com/edgehog-device-manager/edgehog-device-runtime/issues)
are welcome.

## License

Edgehog Device Runtime source code is released under the Apache 2.0 License.

Check the [LICENSE](LICENSE) file for more information.

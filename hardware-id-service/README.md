<!---
  Copyright 2022 SECO Mind Srl

  SPDX-License-Identifier: Apache-2.0
-->

# Hardware ID dbus Service Example
Reference project of a service that either fetches hardware id from SMBIOS/DMI
or from the kernel command line.

## Setup
Copy `io.edgehog.Device.conf` to the dbus system configuration directory
and reboot your system so dbus reloads its configuration.

```bash
cp ./io.edgehog.Device.conf /etc/dbus-1/system.d/
reboot
```

Build with cargo
```bash
cargo build --release
```

Run as superuser
```bash
sudo target/release/hardware_id_service
```

## Troubleshooting

### Unable to read file
```rust
thread 'main' panicked at 'Unable to read file: Os { code: 13, kind: PermissionDenied, message: "Permission denied" }', src/main.rs:22:73
```
Run the service as superuser.

### Connection is not allowed to own the service
```rust
Error: FDO(AccessDenied("Connection \":1.96\" is not allowed to own the service \"io.edgehog.Device\" due to security policies in the configuration file"))
```
Install dbus configuration file.

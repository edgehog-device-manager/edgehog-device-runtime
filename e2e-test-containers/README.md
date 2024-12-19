<!---
  Copyright 2023-2024 SECO Mind Srl

  SPDX-License-Identifier: Apache-2.0
-->

# e2e-test-containers

Tests the container service by sending and receiving the container interfaces.

## Config

To test run `cargo e2e-test-containers` with the appropriate flags.

```
$ cargo e2e-test-containers -h
Usage: e2e-test-containers --realm <REALM> --device-id <DEVICE_ID> --credentials-secret <CREDENTIALS_SECRET> --pairing-url <PAIRING_URL> --interfaces-dir <INTERFACES_DIR> --store-dir <STORE_DIR> <COMMAND>

Commands:
  send     Send the data to astarte
  receive
  help     Print this message or the help of the given subcommand(s)

Options:
      --realm <REALM>
          Realm of the device [env: ASTARTE_REALM=]
      --device-id <DEVICE_ID>
          Astarte device id [env: ASTARTE_DEVICE_ID=2TBn-jNESuuHamE2Zo1anA]
      --credentials-secret <CREDENTIALS_SECRET>
          Credential secret [env: ASTARTE_CREDENTIALS_SECRET=3pBM0vTYVZbD3B4QaORgCi2yBvFNCbBXbt+sqcjsXac=]
      --pairing-url <PAIRING_URL>
          Astarte pairing url [env: ASTARTE_PAIRING_URL=http://api.astarte.localhost/pairing]
      --interfaces-dir <INTERFACES_DIR>
          Astarte interfaces directory [env: ASTARTE_INTERFACES_DIR=]
      --store-dir <STORE_DIR>
          Astarte storage directory [env: ASTARTE_STORE_DIR=]
  -h, --help
          Print help
```

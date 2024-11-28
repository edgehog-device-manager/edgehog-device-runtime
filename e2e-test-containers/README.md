<!--
This file is part of Edgehog.

Copyright 2023 - 2025 SECO Mind Srl

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

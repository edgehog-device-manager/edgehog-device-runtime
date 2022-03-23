<!---
  Copyright 2022 SECO Mind Srl

  SPDX-License-Identifier: Apache-2.0
-->

# OS Requirements

Edgehog Device Runtime has a number of requirements in order to provide device management features.

## Required Libraries

## Runtime Dependencies
* **[dbus](https://www.freedesktop.org/wiki/Software/dbus/)** (optional): Needed for communicating with 3rd party services, such as RAUC.
* **[RAUC](https://rauc.io/) ~> v1.5** (optional): Needed for OS updates.

## Filesystem Layout
* **/tmp**: Software updates will be downloaded here.
* **/data**: Edgehog Device Runtime will store its state here during OTA update process.
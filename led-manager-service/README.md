<!---
  Copyright 2022 SECO Mind Srl

  SPDX-License-Identifier: Apache-2.0
-->

# Led Manager D-Bus Service Example

Reference project of a service that exposes available LEDs and allows to set them.

**Caveat**

In this example the `Set` method is mocked and always returns `true`.

## Setup

Run with cargo

```bash
cargo run --release
```

# Usage

**Service name**: `io.edgehog.LedManager`

**Service path**: `/io/edgehog/LedManager`

**Service interface**: `io.edgehog.LedManager1`

## Methods

```
Insert ( IN String   led_id);
List (OUT Array<String> led_id);
Set (IN String led_id
     IN Boolean status
     OUT Boolean result);
```

### Insert

Add a new LED id.

```bash
$ dbus-send --print-reply --dest=io.edgehog.LedManager \
 /io/edgehog/LedManager io.edgehog.LedManager1.Insert \
 string:gpio1

method return time=1664275555.355858 sender=:1.1 -> destination=:1.3 serial=4 reply_serial=2
```

### List

List all available LEds.

```bash
$ dbus-send --print-reply --dest=io.edgehog.LedManager \
  /io/edgehog/LedManager io.edgehog.LedManager1.List

method return time=1664275587.650016 sender=:1.1 -> destination=:1.4 serial=5 reply_serial=2
   array [
      string "gpio1"
   ]
```

### Set

Set the status of the given LED.

```bash
$ dbus-send --print-reply --dest=io.edgehog.LedManager \
/io/edgehog/LedManager io.edgehog.LedManager1.Set \
string:gpio1 boolean:true

method return time=1664275640.809741 sender=:1.1 -> destination=:1.6 serial=6 reply_serial=2
   boolean true
```

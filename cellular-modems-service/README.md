<!---
  Copyright 2024 SECO Mind Srl
  SPDX-License-Identifier: Apache-2.0
-->

# Cellular Properties dbus Service Example

Reference project of a service that expose APIs to add and retrieve cellular modems data.

# Setup

Build with cargo

```bash
cargo build --release
```

Run

```bash
cargo run
```

# Usage

**Service name**: `io.edgehog.CellularModems`

**Service path**: `/io/edgehog/CellularModems`

**Service interface**: `io.edgehog.CellularModems1`

## Methods

```
Insert ( IN String   modem_id
         IN String   apn
         IN String   imei
         IN String   imsi );
List (OUT Array<String> modem_ids);
Get ( IN String modem_id
      OUT Dict<String, String> modem_properties )
```

### Insert

Add a new modem dictionary.

```bash
$ dbus-send --print-reply --dest=io.edgehog.CellularModems /io/edgehog/CellularModems io.edgehog.CellularModems1.Insert \
string:"1" string:"apn.cxn" string:"100474636527166" string:"310170845466094"
```

### List

List all available modems

```bash
$ dbus-send --print-reply --dest=io.edgehog.CellularModems /io/edgehog/CellularModems io.edgehog.CellularModems1.List

method return time=1663161698.312568 sender=:1.1 -> destination=:1.12 serial=20 reply_serial=2
   array [
      string "1"
   ]
```

### Get

Get a modem dictionary

```bash
$ dbus-send --print-reply --dest=io.edgehog.CellularModems /io/edgehog/CellularModems io.edgehog.CellularModems1.Get string:1

method return time=1663161753.656539 sender=:1.1 -> destination=:1.13 serial=21 reply_serial=2
   array [
      dict entry(
         string "apn"
         variant             string "apn.cxn"
      )
      dict entry(
         string "imei"
         variant             string "100474636527166"
      )
      dict entry(
         string "imsi"
         variant             string "310170845466094"
      )
   ]
```

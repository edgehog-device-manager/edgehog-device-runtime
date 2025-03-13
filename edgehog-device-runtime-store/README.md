<!--
This file is part of Edgehog.

Copyright 2025 SECO Mind Srl

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

# edgehog-device-runtime-containers-store

Separate crate with the persistent state for the container service.

This is in its own separate crate to reduce the compile times for the main container service.

## Tools

It uses the [diesel](https://docs.rs/diesel/latest/diesel/) ORM to generate safe SQL queries.

To generate new migration you'll need the `diesel-cli`

```sh
export DATABASE_URL=sqlite:///tmp/edgehog-containers/state.db
diesel migration generate create_some_migration
```

There are also script to run in the `scripts` directory to generate the SQLite file or run all the
migrations.

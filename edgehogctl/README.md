<!--
This file is part of Edgehog.

Copyright 2026 SECO Mind Srl

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

# edgehogctl

Local tool to interact with the Edgehog Device Runtime.

## File Transfer

To use the file transfer feature you'll need an Astarte instance, an HTTP server to download the
file and an S3 bucket for the upload.

You can follow
[Astarte in 5 minutes](https://docs.astarte-platform.org/astarte/latest/010-astarte_in_5_minutes.html)
to setup a local astarte cluster. It's necessary to create an `astartectl` context for the realm
created on the local cluster.

To setup an S3 compatible server you must use [RustFS](https://hub.docker.com/r/rustfs/rustfs) on as
a container running on local host.

> At the moment we don't let configure other provider

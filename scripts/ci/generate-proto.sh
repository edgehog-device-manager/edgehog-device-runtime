#!/usr/bin/env bash

# This file is part of Edgehog.
#
# Copyright 2025, 2026 SECO Mind Srl
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# SPDX-License-Identifier: Apache-2.0

set -exEuo pipefail

# Trap -e errors
trap 'echo "Exit status $? at line $LINENO from: $BASH_COMMAND"' ERR

manifest=$(cargo metadata --no-deps --format-version 1)
root_dir=$(echo "$manifest" | jq '.workspace_root' --raw-output)
target_dir=$(echo "$manifest" | jq '.target_directory' --raw-output)
workingDir="$target_dir/edgehog-protos"

rm -rf "$workingDir" || true
mkdir -p "$workingDir"

cargo run -p proto-codegen --locked -- --protos ./deps/forwarder-proto/proto/ --output "$workingDir"

mv "$workingDir/edgehog.device.forwarder.rs" "$root_dir/edgehog-device-forwarder-proto/src/proto.rs"

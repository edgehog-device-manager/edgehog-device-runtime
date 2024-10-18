#!/usr/bin/env bash
# This file is part of Edgehog.
#
# Copyright 2024 SECO Mind Srl
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#   http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# SPDX-License-Identifier: Apache-2.0

####
# Wrapper to run a command with the environment variables set to have a device registered with
# astarte.
#
# This scrips need the following environment variables to be set:
#
# - KEY: path to the private key for astarte
# - INTERFACES_DIR: path to the interfaces to sync with astarte
#
# Example:
#
# ./scripts/register-device.sh cargo run --example retention

set -exEuo pipefail

if [[ -z $KEY ]]; then
    echo "Export the \$KEY environment variable as the path to the private key for astarte"
    exit 1
fi

export RUST_LOG=${RUST_LOG:-debug}
astartectl realm-management interfaces sync -y \
    -u http://api.astarte.localhost \
    -r test \
    -k "$KEY" \
    "$INTERFACES"/*.json

export ASTARTE_REALM='test'
export ASTARTE_API_URL='http://api.astarte.localhost/appengine'
export ASTARTE_PAIRING_URL='http://api.astarte.localhost/pairing'
export ASTARTE_IGNORE_SSL=true
export ASTARTE_INTERFACES_DIR=$INTERFACES

ASTARTE_DEVICE_ID="$(astartectl utils device-id generate-random)"
ASTARTE_CREDENTIALS_SECRET="$(astartectl pairing agent register --compact-output -r test -u http://api.astarte.localhost -k "$KEY" -- "$ASTARTE_DEVICE_ID")"
ASTARTE_TOKEN="$(astartectl utils gen-jwt all-realm-apis -u http://api.astarte.localhost -k "$KEY")"
ASTARTE_STORE_DIR="$(mktemp -d)"

export ASTARTE_DEVICE_ID
export ASTARTE_CREDENTIALS_SECRET
export ASTARTE_TOKEN
export ASTARTE_STORE_DIR

"$@"

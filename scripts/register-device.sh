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

if [[ -z $INTERFACES ]]; then
    echo "Export the \$INTERFACES environment variable as the path to the interfaces directory"
    exit 1
fi

if [[ -z ${1:-} ]]; then
    echo "You need to pass the configuration file"
    echo "For example: $0 config.toml"
    exit 1
fi

config=$1

current_cluster=$(astartectl config current-cluster)

if [[ -z $current_cluster ]]; then
    echo "You need to setup an astartectl cluster"
    exit 1
fi

api_url=$(
    astartectl config clusters show "$current_cluster" |
        grep 'Astarte API URL' |
        cut -f 2- -d ':' |
        tr -d ' '
)

ssl=""
if [[ $api_url == "http:"* ]]; then
    ssl="ignore_ssl=true"
fi

astartectl realm-management interfaces sync -y "$INTERFACES"/*.json

store=$(mktemp -d)
device_id=$(astartectl utils device-id generate-random)
credential=$(astartectl pairing agent register "$device_id" --compact-output)

cat >"$config" <<EOF
astarte_library = "astarte-device-sdk"
interfaces_directory = "$INTERFACES"
store_directory = "$store"
download_directory = "$store/download"

[astarte_device_sdk]
realm = "test"
device_id = "$device_id"
credentials_secret = "$credential"
pairing_url = "$api_url/pairing"
$ssl
EOF

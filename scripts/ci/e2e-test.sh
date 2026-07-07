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

export E2E_REALM=${E2E_REALM:-test}
export E2E_BASE_DOMAIN=${E2E_BASE_DOMAIN:-autotest.astarte-platform.org}
export E2E_IGNORE_SSL=${E2E_IGNORE_SSL:-false}
export E2E_API_URL=${E2E_API_URL:-https://api.$E2E_BASE_DOMAIN}
export E2E_INTERFACE_DIR=${E2E_INTERFACE_DIR:-deps/interfaces}

# Install interfaces
astartectl realm-management interfaces sync --non-interactive "$E2E_INTERFACE_DIR"/*.json
astartectl realm-management interfaces ls

# Register
E2E_DEVICE_ID=$(astartectl utils device-id generate-random)
E2E_TOKEN=$(astartectl utils gen-jwt all-realm-apis)
E2E_PAIRING_TOKEN=$(astartectl utils gen-jwt pairing)
E2E_STORE_DIR=$(mktemp -d)

export E2E_DEVICE_ID E2E_TOKEN E2E_PAIRING_TOKEN E2E_STORE_DIR

cargo run --locked -p e2e-test

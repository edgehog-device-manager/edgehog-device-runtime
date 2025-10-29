#!/usr/bin/env bash

# This file is part of Edgehog.
#
# Copyright 2025 SECO Mind Srl
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

authdir=./.tmp/private-registry/auth

if command -v docker; then
    command="docker"
elif command -v podman; then
    command="podman"
else
    echo specify container runtime >&2
    exit 1
fi

mkdir -p $authdir

# Create htpasswd
$command run \
    --entrypoint htpasswd \
    httpd:2 -Bbn testuser testpassword >$authdir/htpasswd

$command run -d \
    -p 5000:5000 \
    --restart=always \
    --name registry \
    -v $authdir:/auth \
    -e "REGISTRY_AUTH=htpasswd" \
    -e "REGISTRY_AUTH_HTPASSWD_REALM=Registry Realm" \
    -e REGISTRY_AUTH_HTPASSWD_PATH=/auth/htpasswd \
    --replace \
    registry:3

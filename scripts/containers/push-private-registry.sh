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

if command -v docker; then
    command="docker"
elif command -v podman; then
    command="podman"
else
    echo specify container runtime >&2
    exit 1
fi

$command pull docker.io/library/nginx:stable-alpine-slim

$command tag docker.io/library/nginx:stable-alpine-slim localhost:5000/library/nginx:stable-alpine-slim

$command login localhost:5000 --username testusername --password testpassword

$command push localhost:5000/library/nginx:stable-alpine-slim

$command logout localhost:5000

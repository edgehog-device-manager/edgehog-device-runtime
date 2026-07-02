#!/usr/bin/env bash

# This file is part of Edgehog.
#
# Copyright 2026 SECO Mind Srl
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

if [ $# != 2 ]; then
    base=${BASE_REF:-main}
    head=${HEAD_REF:-HEAD}
else
    base=$1
    head=$2
fi

git_file_names() {
    git diff --name-only "$base" "$head"
}

git_file_names | xargs --max-args 1 -P "$(nproc)" ./scripts/ci/copyright.sh

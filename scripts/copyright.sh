#!/usr/bin/env bash

# This file is part of Edgehog.
#
# Copyright 2025 SECO Mind Srl
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#    http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# SPDX-License-Identifier: Apache-2.0

##
# Annotates the files passed from stdin
#
# For example
#
#   git status --short | cut -f 2 -d ' ' | ./scripts/copyright.sh
#

set -exEuo pipefail

annotate() {
    reuse annotate \
        --copyright 'SECO Mind Srl' \
        --copyright-prefix string \
        --merge-copyrights \
        --license 'Apache-2.0' \
        --template apache-2 \
        "$@"
}

if [[ $# != 0 ]]; then
    annotate "$@"

    exit
fi

# Read from stdin line by line
while read -r line; do
    if [[ $line == '' ]]; then
        echo "Empty line, skipping" 1>&2
        continue
    fi

    annotate "$line"
done </dev/stdin

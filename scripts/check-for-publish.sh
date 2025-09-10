#!/usr/bin/env bash

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

set -exEuo pipefail

# Check if the crate can be compiled with only the files that will be packaged when publishing

# List files in a package
listPackage() {
    cargo package --allow-dirty -l -p "$1" | xargs -I '{}' echo "$1/{}"
}

pkgsFiles=$(
    cat <(cargo package --allow-dirty -l -p "@MAIN_CRATE@") \
        <(listPackage "@CRATE@") |
        sort
)
localFiles=$(
    git ls-files -cdmo | sort -u
)

# List files unique to localFiles and not present in pkgsFiles
toCopy=$(comm -12 <(echo "$localFiles") <(echo "$pkgsFiles"))

workingDir="$(mktemp -d)"

mkdir -p "$workingDir"

echo "$toCopy" | while read -r file; do
    parent=$(dirname "$file")
    mkdir -p "$workingDir/${parent}"

    cp -v "$file" "$workingDir/$file"
done

cargo check --manifest-path "$workingDir/Cargo.toml" --workspace --all-features --locked

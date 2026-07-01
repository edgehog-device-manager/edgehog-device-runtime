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

####
# ENV
#
# EXPORT_BASE_COMMIT:   sha of the commit for diff coverage
# EXPORT_FOR_CI:        copies the coverage files to the CWD
#
# TOOLS
#
# cargo-llvm-cov:     used to merge coverages for many crates
# genhtml:  better branch and function coverage

# Output directories for the profile files and coverage
#
# You'll find the coverage report and `lcov` file under: $CARGO_TARGET_DIR/debug/coverage/
#
# Use absolute paths every where
CARGO_TARGET_DIR=$(
    cargo metadata --format-version 1 --no-deps --locked |
        jq '.target_directory' --raw-output
)
export CARGO_TARGET_DIR
export CARGO_INCREMENTAL=0

gethtml_wrapper() {

    # Better branch coverage information
    if ! command -v genhtml; then
        echo 'Command genhtml not found'
        return
    fi

    mkdir -p "$COVERAGE_OUT_DIR/genhtml"
    genhtml \
        --show-details \
        --legend \
        --branch-coverage \
        --dark-mode \
        --missed \
        --demangle-cpp rustfilt \
        --output-directory "$COVERAGE_OUT_DIR/$1/genhtml" \
        "$COVERAGE_OUT_DIR/$1/lcov.info"

}

crate="edgehog-device-runtime"

if [[ -n "${EXPORT_FOR_CI:-}" ]]; then
    out_path="$PWD/coverage-$crate.info"
else
    mkdir -p "$CARGO_TARGET_DIR/lcov"
    out_path="$CARGO_TARGET_DIR/lcov/coverage-$crate.info"
fi

# Currently branch coverage can be broken on nightly
cargo +nightly llvm-cov \
    --all-features \
    -p "edgehog-device-runtime" \
    -p "edgehog-device-runtime-containers" \
    -p "edgehog-device-runtime-forwarder" \
    --lcov \
    --output-path "$out_path"

cargo llvm-cov report

if [[ -z "${EXPORT_FOR_CI:-}" ]]; then
    cargo llvm-cov report
    cargo llvm-cov report --html
else
    {
        echo '# Code Coverage'
        echo ''
        echo '```'
        cargo llvm-cov report
        echo '```'
    } >>"$GITHUB_STEP_SUMMARY"
fi

if [[ -n "${EXPORT_BASE_COMMIT:-}" ]]; then
    commit=$(git rev-parse HEAD)
    cp -v "$out_path" "$COVERAGE_OUT_DIR/baseline-$commit.info"
    echo "$commit" >"$COVERAGE_OUT_DIR/baseline-commit.txt"
fi

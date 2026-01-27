#!/usr/bin/env bash

# This file is part of Edgehog.
#
# Copyright 2025, 2026 SECO Mind Srl
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

####
# ENV
#
# EXPORT_BASE_COMMIT:   sha of the commit for diff coverage
# EXPORT_FOR_CI:        copies the coverage files to the CWD
#
# TOOLS
#
# lcov:     used to merge coverages for many crates
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

filter_lcov() {
    local src
    if [[ $1 == "$src_crate" ]]; then
        src="$SRC_DIR"
    else
        src="$SRC_DIR/$1"
    fi

    obj_args=$(object_files -p "$1")

    mkdir -p "$COVERAGE_OUT_DIR/$1/lcov-show"
    $LLVM_COV show \
        -Xdemangler=rustfilt \
        -format=html \
        -show-directory-coverage \
        -show-mcdc \
        -show-line-counts-or-regions \
        -instr-profile="$PROFS_DIR/coverage.profdata" \
        -output-dir="$COVERAGE_OUT_DIR/$1/lcov-show" \
        -sources "$src" "${obj_args[@]}"

    # Better branch coverage information
    if command -v genhtml; then
        mkdir -p "$COVERAGE_OUT_DIR/$1/genhtml"

        arg_diff=()

        if [[ -z "${EXPORT_BASE_COMMIT:-}" && -f "$COVERAGE_OUT_DIR/baseline-commit.txt" ]]; then
            commit=$(cat "$COVERAGE_OUT_DIR/baseline-commit.txt")
            current=$(git rev-parse HEAD)

            if [[ $commit != "$current" ]]; then
                git diff "$commit.." -p --src-prefix= --dst-prefix= >"$COVERAGE_OUT_DIR/patch.diff"

                arg_diff+=(
                    "--baseline-file=$COVERAGE_OUT_DIR/baseline-$commit-$p.info"
                    "--diff-file=$COVERAGE_OUT_DIR/patch.diff"
                )
            fi
        fi

        genhtml \
            --show-details \
            --legend \
            --branch-coverage \
            --dark-mode \
            --missed \
            --demangle-cpp rustfilt \
            --output-directory "$COVERAGE_OUT_DIR/$1/genhtml" \
            "${arg_diff[@]}" \
            "$COVERAGE_OUT_DIR/$1/lcov.info"
    fi
}

if [[ -n "${EXPORT_FOR_CI:-}" ]]; then
    out_path="$PWD/coverage-astarte-message-hub.info"
else
    mkdir -p "$CARGO_TARGET_DIR/lcov"
    out_path="$CARGO_TARGET_DIR/lcov/coverage-astarte-message-hub.info"
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
    cargo llvm-cov report --html
fi

if [[ -n "${EXPORT_BASE_COMMIT:-}" ]]; then
    commit=$(git rev-parse HEAD)
    cp -v "$COVERAGE_OUT_DIR/$p/lcov.info" "$COVERAGE_OUT_DIR/baseline-$commit-$p.info"
    echo "$commit" >"$COVERAGE_OUT_DIR/baseline-commit.txt"
fi

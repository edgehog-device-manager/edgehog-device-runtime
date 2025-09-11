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

set -exEuo pipefail

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
SRC_DIR="$(
    cargo +nightly locate-project |
        jq .root --raw-output |
        xargs dirname
)"
export SRC_DIR
export PROFS_DIR="$CARGO_TARGET_DIR/profs"
export LLVM_PROFILE_FILE="$PROFS_DIR/coverage-%p-%m.profraw"
export COVERAGE_OUT_DIR="$CARGO_TARGET_DIR/debug/coverage"

# This require a nightly compiler
#
# Rustc options:
#
# - `instrument-coverage`: enable coverage for all
# - `coverage-options=branch``: enable block and branch coverage (unstable option)
#
# See: https://doc.rust-lang.org/rustc/instrument-coverage.html
export RUSTFLAGS="-Cinstrument-coverage -Zcoverage-options=branch --cfg=__coverage"
export CARGO_INCREMENTAL=0

main='edgehog-device-runtime'
crates=(
    'edgehog-device-runtime'
    'edgehog-device-runtime-containers'
    'edgehog-device-runtime-forwarder'
)

# Helpful for testing changes in the generation options
if [[ ${1:-} != '--no-gen' ]]; then
    cargo +nightly clean

    mkdir -p "$COVERAGE_OUT_DIR"
    mkdir -p "$PROFS_DIR"

    for crate in "${crates[@]}"; do
        cargo +nightly test --locked --all-features --tests --no-fail-fast -p "$crate"
    done
fi

find_target_tool() {
    local libdir
    local tool_path

    libdir=$(rustup run nightly rustc --print target-libdir)
    tool_path=$(realpath "$libdir/../bin/$1")

    echo "$tool_path"
}

rustup_llvm_profdata=$(find_target_tool llvm-profdata)
rustup_llvm_cov=$(find_target_tool llvm-cov)

LLVM_PROFDATA=${LLVM_PROFDATA:-$rustup_llvm_profdata}
LLVM_COV=${LLVM_COV:-$rustup_llvm_cov}

$LLVM_PROFDATA merge -sparse "$PROFS_DIR/"*.profraw -o "$PROFS_DIR/coverage.profdata"

object_files() {
    tests=$(
        cargo +nightly test --tests --all-features --no-run --message-format=json "$@" |
            jq -r "select(.profile.test == true) | .filenames[]" |
            grep -v dSYM -
    )

    for file in $tests; do
        printf "%s %s " -object "$file"
    done
}

export_lcov() {
    local src
    if [[ $1 == "$main" ]]; then
        src="$SRC_DIR/src"
    else
        src="$SRC_DIR/$1/src"
    fi

    # shellcheck disable=2086,2046
    $LLVM_COV export \
        -format=lcov \
        -ignore-filename-regex='/.cargo/registry' \
        -instr-profile="$PROFS_DIR/coverage.profdata" \
        -ignore-filename-regex='.*test\.rs' \
        -ignore-filename-regex='.*mock\.rs' \
        -sources "$src" $(object_files -p "$1") \
        >"$COVERAGE_OUT_DIR/$1/lcov.info"
}

filter_lcov() {
    local src
    if [[ $1 == "$main" ]]; then
        src="$SRC_DIR"
    else
        src="$SRC_DIR/$1"
    fi

    grcov \
        "$COVERAGE_OUT_DIR/$1/lcov.info" \
        --binary-path "$CARGO_TARGET_DIR/debug" \
        --output-path "$COVERAGE_OUT_DIR/$1/" \
        --source-dir "$src" \
        --branch \
        --llvm \
        --excl-start 'mod test(s)?' \
        --output-type lcov,html

    # Better branch coverage information
    if command -v genhtml; then
        mkdir -p "$COVERAGE_OUT_DIR/$1/genhtml"
        genhtml \
            --show-details \
            --legend \
            --branch-coverage \
            --dark-mode \
            --missed \
            --ignore-errors category \
            --ignore-errors inconsistent \
            --output-directory "$COVERAGE_OUT_DIR/$1/genhtml" \
            "$COVERAGE_OUT_DIR/$1/lcov.info"

    fi
}

for p in "${crates[@]}"; do
    mkdir -p "$COVERAGE_OUT_DIR/$p/"

    export_lcov "$p"

    filter_lcov "$p"

    cp -v "$COVERAGE_OUT_DIR/$p/lcov" "$COVERAGE_OUT_DIR/coverage-$p.info"

    if [[ -n "${EXPORT_FOR_CI:-}" ]]; then
        cp -v "$COVERAGE_OUT_DIR/$p/lcov" "$PWD/coverage-$p.info"
    fi
done

# Fixes the profraw being detected by codecov
if [[ -n "${EXPORT_FOR_CI:-}" ]]; then
    rm -rf "$CARGO_TARGET_DIR"
fi

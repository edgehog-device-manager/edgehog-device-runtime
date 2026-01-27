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

src_crate='@MAIN_CRATE@'
crates=(
    '@MAIN_CRATE@'
    '@CRATE@'
)

# Helpful for testing changes in the generation options
if [[ ${1:-} != '--no-gen' ]]; then
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
    objects=$(
        cargo +nightly test --tests --all-features --no-run --message-format=json "$@" |
            jq -r "select(.profile.test == true) | .filenames[]" |
            grep -v dSYM -
    )

    for obj in "${objects[@]}"; do
        echo "-object=$obj"
    done
}

export_lcov() {
    local src
    if [[ $1 == "$src_crate" ]]; then
        src="$SRC_DIR/src"
    else
        src="$SRC_DIR/$1/src"
    fi

    obj_args=$(object_files -p "$p")

    $LLVM_COV export \
        -Xdemangler=rustfilt \
        -format=lcov \
        -ignore-filename-regex='/.cargo/registry' \
        -instr-profile="$PROFS_DIR/coverage.profdata" \
        -ignore-filename-regex='.*test\.rs' \
        -ignore-filename-regex='.*mock\.rs' \
        -sources "$src" "${obj_args[@]}" \
        >"$COVERAGE_OUT_DIR/$1/lcov.info"
}

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

for p in "${crates[@]}"; do
    mkdir -p "$COVERAGE_OUT_DIR/$p/"

    export_lcov "$p"

    filter_lcov "$p"

    cp -v "$COVERAGE_OUT_DIR/$p/lcov.info" "$COVERAGE_OUT_DIR/coverage-$p.info"

    if [[ -n "${EXPORT_FOR_CI:-}" ]]; then
        cp -v "$COVERAGE_OUT_DIR/$p/lcov" "$PWD/coverage-$p.info"
    fi

    if [[ -n "${EXPORT_BASE_COMMIT:-}" ]]; then
        commit=$(git rev-parse HEAD)
        cp -v "$COVERAGE_OUT_DIR/$p/lcov.info" "$COVERAGE_OUT_DIR/baseline-$commit-$p.info"
        echo "$commit" >"$COVERAGE_OUT_DIR/baseline-commit.txt"
    fi
done

# Fixes the profraw being detected by codecov
if [[ -n "${EXPORT_FOR_CI:-}" ]]; then
    rm -rf "$CARGO_TARGET_DIR"
fi

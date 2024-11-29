#!/usr/bin/env bash

set -exEuo pipefail

./scripts/create-db.sh
diesel migration run
diesel migration redo

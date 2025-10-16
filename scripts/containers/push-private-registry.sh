#!/usr/bin/env bash

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

$command login localhost:5000 --username testusername --passowrd testpassword

$command push localhost:5000/library/nginx:stable-alpine-slim

$command logout localhost:5000

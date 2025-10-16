#!/usr/bin/env bash

set -exEuo pipefail

# Trap -e errors
trap 'echo "Exit status $? at line $LINENO from: $BASH_COMMAND"' ERR

authdir=./.tmp/private-registry/auth

if command -v docker; then
    command="docker"
elif command -v podman; then
    command="podman"
else
    echo specify container runtime >&2
    exit 1
fi

mkdir -p $authdir

# Create htpasswd
$command run \
    --entrypoint htpasswd \
    httpd:2 -Bbn testuser testpassword >$authdir/htpasswd

$command run -d \
    -p 5000:5000 \
    --restart=always \
    --name registry \
    -v $authdir:/auth \
    -e "REGISTRY_AUTH=htpasswd" \
    -e "REGISTRY_AUTH_HTPASSWD_REALM=Registry Realm" \
    -e REGISTRY_AUTH_HTPASSWD_PATH=/auth/htpasswd \
    --replace \
    registry:3

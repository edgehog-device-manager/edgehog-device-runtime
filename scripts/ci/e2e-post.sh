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

NAMESPACE="$1"

kubectl describe astarte astarte -n "$NAMESPACE"

kubectl describe deployments/astarte-operator-controller-manager -n astarte-operator
kubectl logs deployments/astarte-operator-controller-manager -n astarte-operator

kubectl get pods -n "$NAMESPACE"

for pod in $(kubectl get pods -n "$NAMESPACE" --no-headers -o custom-columns=":metadata.name"); do
    echo "==== POD($pod) ===="

    kubectl describe pod -n "$NAMESPACE" "$pod"

    echo "==== LOGS($pod) ===="

    kubectl logs -n "$NAMESPACE" "$pod"
done

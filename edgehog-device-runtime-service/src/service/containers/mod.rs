// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

use edgehog_containers::local::ContainerHandle;
use edgehog_proto::tonic::Status;
use tracing::error;

use super::EdgehogService;

mod v1;

impl EdgehogService {
    fn container_handle(&self) -> Result<&ContainerHandle, Status> {
        self.containers.get().ok_or_else(|| {
            error!("container service is not available");

            Status::unavailable("container service not available")
        })
    }
}

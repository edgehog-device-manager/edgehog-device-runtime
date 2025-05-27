// This file is part of Edgehog.
//
// Copyright 2023-2024 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Client to use to communicate with the Docker daemon.

// Mock the client, this is behind a feature flag so we can test with both the real docker daemon
// and the mocked one.

cfg_if::cfg_if! {
    if #[cfg(feature = "__mock")] {
        pub(crate) use crate::mock::{DockerTrait, MockDocker as Client};
    } else {
        pub(crate) use bollard::Docker as Client;
    }
}

// This file is part of Edgehog.
//
// Copyright 2023 SECO Mind Srl
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

//! Wrapper around the Docker client

// Mock the client, this is behind a feature flag so we can test with both the real docker daemon
// and the mocked one.
#[cfg(feature = "mock")]
pub(crate) use crate::mock::{DockerTrait, MockDocker as Client};
#[cfg(not(feature = "mock"))]
pub(crate) use bollard::Docker as Client;

use crate::error::DockerError;

/// Docker container manager
#[derive(Debug, Clone)]
pub struct Docker {
    pub(crate) client: Client,
}

impl Docker {
    /// Create a new Docker container manager
    #[cfg(not(feature = "mock"))]
    pub fn connect() -> Result<Self, DockerError> {
        let client = Client::connect_with_local_defaults().map_err(DockerError::Connection)?;

        Ok(Self { client })
    }

    /// Create a new Docker container manager
    #[cfg(feature = "mock")]
    pub fn connect() -> Result<Self, DockerError> {
        let client = Client::new();

        Ok(Self { client })
    }

    /// Ping the Docker daemon
    pub async fn ping(&self) -> Result<(), DockerError> {
        // Discard the result since it returns the string `OK`
        self.client.ping().await.map_err(DockerError::Ping)?;

        Ok(())
    }
}

impl From<Client> for Docker {
    fn from(client: Client) -> Self {
        Self { client }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns a [Docker] instance, or a mocked version with the expect statements if the mock
    /// feature is enabled.
    #[macro_export]
    macro_rules! docker_mock {
        ($mock:expr) => {{
            #[cfg(feature = "mock")]
            let docker: $crate::Docker = {
                let mock: $crate::Client = $mock;

                Docker::from(mock)
            };

            #[cfg(not(feature = "mock"))]
            let docker: $crate::docker::Docker = $crate::docker::Docker::connect().unwrap();

            docker
        }};
        ($default:expr, $mock:expr) => {{
            #[cfg(feature = "mock")]
            let client: $crate::docker::Client = $mock;

            #[cfg(not(feature = "mock"))]
            let client: $crate::docker::Client = $default;

            $crate::Docker::from(client)
        }};
    }

    #[tokio::test]
    async fn test_ping_and_macro() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            mock.expect_ping().returning(|| Ok(Default::default()));

            mock
        });

        let res = docker.ping().await;

        assert!(res.is_ok(), "Ping failed: {:?}", res);
    }
}

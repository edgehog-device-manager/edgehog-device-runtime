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

//! Wrapper around the Docker client

use std::{
    borrow::{Borrow, BorrowMut},
    ops::{Deref, DerefMut},
};

use bollard::{secret::EventMessage, system::EventsOptions};
use futures::{Stream, TryStreamExt};

pub(crate) use crate::client::*;
use crate::error::DockerError;

pub(crate) mod container;
pub(crate) mod image;
pub(crate) mod network;
pub(crate) mod volume;

/// Docker container manager
#[derive(Debug, Clone)]
pub struct Docker {
    pub(crate) client: Client,
}

impl Docker {
    /// Create a new Docker container manager
    #[cfg(not(feature = "mock"))]
    pub async fn connect() -> Result<Self, DockerError> {
        let client = Client::connect_with_local_defaults()
            .map_err(DockerError::Connection)?
            .negotiate_version()
            .await
            .map_err(DockerError::Version)?;

        Ok(Self { client })
    }

    /// Create a new Docker container manager
    #[cfg(feature = "mock")]
    pub async fn connect() -> Result<Self, DockerError> {
        let client = Client::new();

        Ok(Self { client })
    }

    /// Ping the Docker daemon
    pub async fn ping(&self) -> Result<(), DockerError> {
        // Discard the result since it returns the string `OK`
        self.client.ping().await.map_err(DockerError::Ping)?;

        Ok(())
    }

    /// Ping the Docker daemon
    pub fn events(&self) -> impl Stream<Item = Result<EventMessage, DockerError>> {
        let types = vec!["container", "image", "volume", "network"];

        let filters = [("type", types)].into_iter().collect();

        let options = EventsOptions {
            since: None,
            until: None,
            filters,
        };

        // Discard the result since it returns the string `OK`
        self.client
            .events(Some(options))
            .map_err(DockerError::Envents)
    }
}

impl From<Client> for Docker {
    fn from(client: Client) -> Self {
        Self { client }
    }
}

impl Borrow<Client> for Docker {
    fn borrow(&self) -> &Client {
        &self.client
    }
}

impl BorrowMut<Client> for Docker {
    fn borrow_mut(&mut self) -> &mut Client {
        &mut self.client
    }
}

impl Deref for Docker {
    type Target = Client;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl DerefMut for Docker {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.client
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Returns a [Docker] instance, or a mocked version with the expect statements if the mock
    /// feature is enabled.
    #[macro_export]
    macro_rules! docker_mock {
        ($mock:expr) => {{
            #[cfg(feature = "mock")]
            let docker: $crate::Docker = {
                let mock: $crate::client::Client = $mock;

                Docker::from(mock)
            };

            #[cfg(not(feature = "mock"))]
            let docker: $crate::docker::Docker = $crate::docker::Docker::connect().unwrap();

            docker
        }};
        ($default:expr, $mock:expr) => {{
            #[cfg(feature = "mock")]
            let client: $crate::client::Client = $mock;

            #[cfg(not(feature = "mock"))]
            let client: $crate::client::Client = $default;

            $crate::Docker::from(client)
        }};
    }

    #[cfg(feature = "mock")]
    pub(crate) fn not_found_response() -> bollard::errors::Error {
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            message: "not found".to_string(),
        }
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

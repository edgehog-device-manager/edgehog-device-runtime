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

//! Handles the container networks.

use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    ops::{Deref, DerefMut},
};

use bollard::{
    errors::Error as BollardError,
    models::Network as DockerNetwork,
    network::{CreateNetworkOptions, InspectNetworkOptions},
};
use tracing::{debug, instrument, trace, warn};

use crate::client::*;

/// Error for the network operations.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum NetworkError {
    /// couldn't create the network
    Create(#[source] BollardError),
    /// couldn't inspect the network
    Inspect(#[source] BollardError),
    /// couldn't remove the network
    Remove(#[source] BollardError),
    /// couldn't convert [`DockerNetwork`] into [`Network`]
    Conversion(#[from] ConversionError),
}

/// Error converting [`DockerNetwork`] into [`Network`]
#[non_exhaustive]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum ConversionError {
    /// missing network name or id
    MissingName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkId {
    /// Id of the container network.
    pub id: Option<String>,
    /// The network's name.
    pub name: String,
}

impl NetworkId {
    /// Get the network id or name if it's missing.
    #[instrument(skip_all)]
    pub fn network(&self) -> &str {
        match &self.id {
            Some(id) => {
                trace!("returning id");

                id.as_str()
            }
            None => {
                trace!("id missing, returning name");

                self.name.as_ref()
            }
        }
    }

    /// Set the id from docker.
    fn update(&mut self, id: String) {
        debug!("using id {id} for network {}", self.name);

        let old_id = self.id.replace(id);

        trace!(?old_id);
    }
}

impl Display for NetworkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(id) = &self.id {
            write!(f, "id: {id}, ")?;
        }

        write!(f, "name: {}", self.name)
    }
}

/// Container network struct.
///
/// Networks are user-defined networks that containers can be attached to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub(crate) id: NetworkId,
    /// Network driver plugin to use.
    ///
    /// Defaults to "bridge"
    pub driver: String,
    /// Restrict external access to the network.
    pub internal: bool,
    /// Enable IPv6 on the network.
    pub enable_ipv6: bool,
    /// Network specific options to be used by the drivers.
    pub driver_opts: HashMap<String, String>,
}

impl Network {
    pub fn new(
        id: Option<String>,
        name: String,
        driver: String,
        internal: bool,
        enable_ipv6: bool,
        driver_options: HashMap<String, String>,
    ) -> Self {
        Self {
            id: NetworkId { id, name },
            driver,
            internal,
            enable_ipv6,
            driver_opts: driver_options,
        }
    }

    /// Create a new docker network.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Network/operation/NetworkCreate)
    #[instrument(skip_all, fields(name = %self.name))]
    pub async fn create(&mut self, client: &Client) -> Result<(), NetworkError> {
        debug!("Create the {}", self);

        let options = CreateNetworkOptions::<&str>::from(&*self);

        let res = client
            .create_network(options)
            .await
            .map_err(NetworkError::Create)?;

        if let Some(id) = res.id {
            self.update(id);
        }

        if let Some(warning) = res.warning {
            if !warning.is_empty() {
                warn!("network created with warning: {warning}");
            }
        }

        Ok(())
    }

    /// Inspect a docker network.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Network/operation/NetworkInspect)
    #[instrument(skip_all, fields(name = %self.name))]
    pub async fn inspect(
        &mut self,
        client: &Client,
    ) -> Result<Option<DockerNetwork>, NetworkError> {
        debug!("Inspecting the {}", self);

        let res = client
            .inspect_network(self.network(), None::<InspectNetworkOptions<String>>)
            .await;

        let network = match res {
            Ok(network) => network,
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("network not found: {message}");

                return Ok(None);
            }
            Err(err) => return Err(NetworkError::Inspect(err)),
        };

        trace!("network info: {network:?}");

        if let Some(id) = &network.id {
            self.update(id.clone());
        }

        Ok(Some(network))
    }

    /// Remove a docker network.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Network/operation/NetworkDelete)
    #[instrument(skip_all)]
    pub async fn remove(&self, client: &Client) -> Result<Option<()>, NetworkError> {
        debug!("deleting {}", self);

        let res = client.remove_network(self.network()).await;

        match res {
            Ok(()) => Ok(Some(())),
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("network not found: {message}");

                Ok(None)
            }
            Err(err) => return Err(NetworkError::Remove(err)),
        }
    }
}

impl Deref for Network {
    type Target = NetworkId;

    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

impl DerefMut for Network {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.id
    }
}

impl Display for Network {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Network{{{}}}", self.id)
    }
}

impl<'a> From<&'a Network> for CreateNetworkOptions<&'a str> {
    fn from(value: &'a Network) -> Self {
        CreateNetworkOptions {
            name: value.name.as_ref(),
            driver: value.driver.as_ref(),
            internal: value.internal,
            enable_ipv6: value.enable_ipv6,
            options: value
                .driver_opts
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_ref()))
                .collect(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{docker_mock, tests::random_name};

    use super::*;

    fn new_network(name: &str) -> Network {
        Network::new(
            None,
            name.to_string(),
            "bridge".to_string(),
            false,
            false,
            HashMap::new(),
        )
    }

    #[tokio::test]
    async fn should_create_network() {
        let name = random_name("create");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let resp = bollard::models::NetworkCreateResponse {
                id: Some("id".to_string()),
                warning: None,
            };

            let name_cl = name.clone();
            mock.expect_create_network()
                .withf(move |option| option.name == name_cl && option.driver == "bridge")
                .once()
                .returning(move |_| Ok(resp.clone()));

            mock
        });

        let mut network = new_network(name.as_str());
        network.create(&docker).await.unwrap();
    }

    #[tokio::test]
    async fn should_inspect_network() {
        let name = random_name("inspect");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            let network = bollard::models::Network {
                name: Some(name.clone()),
                id: Some("id".to_string()),
                driver: Some("bridge".to_string()),
                ..Default::default()
            };

            let resp = bollard::models::NetworkCreateResponse {
                id: Some("id".to_string()),
                warning: None,
            };

            let name_cl = name.clone();
            mock.expect_create_network()
                .withf(move |option| option.name == name_cl && option.driver == "bridge")
                .once()
                .in_sequence(&mut seq)
                .returning(move |_| Ok(resp.clone()));

            mock.expect_inspect_network()
                .withf(|name, _| name == "id")
                .once()
                .in_sequence(&mut seq)
                .returning(move |_, _| Ok(network.clone()));

            mock
        });

        let mut network = new_network(name.as_str());

        network.create(&docker).await.unwrap();
        let net = network.inspect(&docker).await.unwrap().unwrap();

        assert_eq!(net.name, Some(name));
        assert_eq!(net.driver, Some("bridge".to_string()));
    }

    #[tokio::test]
    async fn should_inspect_not_found() {
        // Random image name
        let name = random_name("not-found");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let name_cl = name.clone();
            mock.expect_inspect_network()
                .withf(move |name, _| name == name_cl)
                .once()
                .returning(|_, _| Err(crate::tests::not_found_response()));

            mock
        });

        let mut network = new_network(&name);

        let inspect = network
            .inspect(&docker)
            .await
            .expect("failed to inspect image");

        assert_eq!(inspect, None);
    }

    #[tokio::test]
    async fn remove_network() {
        let name = random_name("remove");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let resp = bollard::models::NetworkCreateResponse {
                id: Some("id".to_string()),
                warning: None,
            };

            let name_cl = name.clone();
            mock.expect_create_network()
                .withf(move |option| option.name == name_cl && option.driver == "bridge")
                .once()
                .returning(move |_| Ok(resp.clone()));

            mock.expect_remove_network()
                .withf(move |name| name == "id")
                .once()
                .returning(|_| Ok(()));

            mock
        });

        let mut network = new_network(name.as_str());

        network.create(&docker).await.expect("failed to create");

        network
            .remove(&docker)
            .await
            .expect("error removing")
            .expect("none response");
    }
}

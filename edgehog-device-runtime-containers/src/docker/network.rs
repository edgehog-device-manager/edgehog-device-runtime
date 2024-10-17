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

use std::fmt::{Debug, Display};

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

/// Container network struct.
///
/// Networks are user-defined networks that containers can be attached to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network<S> {
    /// Id of the container network.
    pub id: Option<String>,
    /// The network's name.
    pub name: S,
    /// The network's name.
    /// Name of the network driver plugin to use.
    ///
    /// Defaults to "bridge"
    pub driver: S,
    /// Check for networks with duplicate names.
    pub check_duplicate: bool,
    /// Restrict external access to the network.
    pub internal: bool,
    /// Enable IPv6 on the network.
    pub enable_ipv6: bool,
}

impl<S> Network<S> {
    pub fn new(
        id: Option<String>,
        name: S,
        driver: S,
        check_duplicate: bool,
        internal: bool,
        enable_ipv6: bool,
    ) -> Self {
        Self {
            id,
            name,
            driver,
            check_duplicate,
            internal,
            enable_ipv6,
        }
    }

    /// Get the network id or name if it's missing.
    #[instrument]
    pub fn network(&self) -> &str
    where
        S: AsRef<str> + Debug,
    {
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
    fn update_id(&mut self, id: Option<String>)
    where
        S: Display,
    {
        if let Some(id) = id {
            debug!("using id {id} for network {}", self.name);

            let old_id = self.id.replace(id);

            trace!(?old_id);
        }
    }

    /// Check if the network exists if the id is set, otherwise it will try creating it.
    ///
    /// Returns true if the network was created.
    pub async fn inspect_or_create(&mut self, client: &Client) -> Result<bool, NetworkError>
    where
        S: AsRef<str> + Display + Debug,
    {
        if self.id.is_some() {
            match self.inspect(client).await? {
                Some(net) => {
                    trace!("found network {net:?}");

                    return Ok(false);
                }
                None => {
                    debug!("network not found, creating it");
                }
            }
        }

        self.create(client).await?;

        Ok(true)
    }

    /// Create a new docker network.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Network/operation/NetworkCreate)
    #[instrument]
    pub async fn create(&mut self, client: &Client) -> Result<(), NetworkError>
    where
        S: Debug + Display + AsRef<str>,
    {
        debug!("Create the {}", self);

        let res = client
            .create_network((&*self).into())
            .await
            .map_err(NetworkError::Create)?;

        self.update_id(res.id);

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
    #[instrument]
    pub async fn inspect(&mut self, client: &Client) -> Result<Option<DockerNetwork>, NetworkError>
    where
        S: Debug + Display + AsRef<str>,
    {
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

        self.update_id(network.id.clone());

        Ok(Some(network))
    }

    /// Remove a docker network.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Network/operation/NetworkDelete)
    #[instrument]
    pub async fn remove(&self, client: &Client) -> Result<Option<()>, NetworkError>
    where
        S: Debug + Display + AsRef<str>,
    {
        debug!("deleting {}", self.name);

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

impl<S> Display for Network<S>
where
    S: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Network")?;

        if let Some(id) = &self.id {
            write!(f, " ({id})")?;
        }

        write!(f, " {}/{}", self.name, self.driver)
    }
}

impl<'a, S> From<&'a Network<S>> for CreateNetworkOptions<&'a str>
where
    S: AsRef<str>,
{
    fn from(value: &'a Network<S>) -> Self {
        CreateNetworkOptions {
            name: value.name.as_ref(),
            driver: value.driver.as_ref(),
            check_duplicate: value.check_duplicate,
            internal: value.internal,
            enable_ipv6: value.enable_ipv6,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{docker_mock, tests::random_name};

    use super::*;

    fn new_network(name: &str) -> Network<&str> {
        Network::new(None, name, "bridge", false, false, false)
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
                .returning(move |_| Ok(resp.clone()));

            mock.expect_inspect_network()
                .withf(|name, _| name == "id")
                .once()
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

    #[tokio::test]
    async fn should_inspect_or_create_network() {
        let name = random_name("inspect_or_create");

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

        let created = network.inspect_or_create(&docker).await.unwrap();

        assert!(created);

        let created = network.inspect_or_create(&docker).await.unwrap();

        assert!(!created);
    }
}

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

//! Docker struct to manage containers.

use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    hash::Hash,
};

use bollard::{
    container::{
        Config, CreateContainerOptions, InspectContainerOptions, NetworkingConfig,
        RemoveContainerOptions,
    },
    errors::Error as BollardError,
    models::{
        ContainerInspectResponse, EndpointSettings, HostConfig, PortBinding, RestartPolicy,
        RestartPolicyNameEnum,
    },
};
use tracing::{debug, instrument, trace, warn};

use crate::client::*;

/// Error for the container operations.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum ContainerError {
    /// couldn't create the container
    Create(#[source] BollardError),
    /// couldn't inspect the container
    Inspect(#[source] BollardError),
    /// couldn't remove the container
    Remove(#[source] BollardError),
    /// couldn't list containers
    List(#[source] BollardError),
}

/// Docker container struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container<S>
where
    S: Hash + Eq,
{
    /// Assign the specified name to the container.
    ///
    /// Must match /?[a-zA-Z0-9][a-zA-Z0-9_.-]+.
    name: S,
    /// The name (or reference) of the image to use.
    image: S,
    /// Network to link the container with.
    network_ids: Vec<S>,
    /// The hostname to use for the container.
    ///
    /// Defaults to the container name.
    hostname: Option<S>,
    /// The behaviour to apply when the container exits.
    ///
    /// See the [create container
    /// API](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerCreate) for
    /// possible values.
    restart_policy: Option<RestartPolicyNameEnum>,
    /// A list of environment variables to set inside the container.
    ///
    /// In the form of `NAME=VALUE`.
    env: Vec<S>,
    /// A list of volume bindings for this container.
    binds: Vec<S>,
    /// Describes the mapping of container ports to host ports.
    ///
    /// It uses the container's port-number and protocol as key in the format `<port>/<protocol>`, for
    /// example, 80/udp.
    port_bindings: HashMap<S, Option<Vec<Binding<S>>>>,
    /// Gives the container full access to the host.
    ///
    /// Defaults to false.
    privileged: bool,
}

impl<S> Container<S>
where
    S: Hash + Eq,
{
    /// Create a new container.
    pub fn new(name: S, image: S) -> Self {
        Self {
            name,
            image,
            hostname: None,
            restart_policy: None,
            env: Vec::new(),
            binds: Vec::new(),
            network_ids: Vec::new(),
            port_bindings: HashMap::new(),
            privileged: false,
        }
    }

    /// Convert the port bindings to be used in [`HostConfig`].
    fn as_port_bindings(&self) -> HashMap<String, Option<Vec<PortBinding>>>
    where
        S: AsRef<str>,
    {
        self.port_bindings
            .iter()
            .map(|(port_proto, binds)| {
                let bindings = binds.as_ref().map(|b| b.iter().map(|b| b.into()).collect());

                (port_proto.as_ref().to_string(), bindings)
            })
            .collect()
    }

    /// Convert the networks into [`NetworkingConfig`]
    // TODO: should test the network link from two containers
    fn as_network_config(&self) -> HashMap<&str, EndpointSettings>
    where
        S: AsRef<str>,
    {
        self.network_ids
            .iter()
            .map(|net_id| {
                (
                    net_id.as_ref(),
                    EndpointSettings {
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    /// Create a new docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerCreate)
    #[instrument]
    pub async fn create(&self, client: &Client) -> Result<(), ContainerError>
    where
        S: Debug + Display + AsRef<str>,
    {
        debug!("creating the {}", self);

        let res = client
            .create_container(Some(self.into()), self.into())
            .await
            .map_err(ContainerError::Create)?;

        for warning in res.warnings {
            warn!("container created with working: {warning}");
        }

        Ok(())
    }

    /// Inspect a docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerInspect)
    #[instrument]
    pub async fn inspect(
        &self,
        client: &Client,
    ) -> Result<Option<ContainerInspectResponse>, ContainerError>
    where
        S: Debug + Display + AsRef<str>,
    {
        debug!("Inspecting the {}", self);

        let res = client
            .inspect_container(self.name.as_ref(), None::<InspectContainerOptions>)
            .await;

        let container = match res {
            Ok(container) => container,
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("container not found: {message}");

                return Ok(None);
            }
            Err(err) => return Err(ContainerError::Inspect(err)),
        };

        trace!("container info: {container:?}");

        Ok(Some(container))
    }

    /// Remove a docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerDelete)
    #[instrument]
    pub async fn remove(&self, client: &Client) -> Result<Option<()>, ContainerError>
    where
        S: Debug + Display + AsRef<str>,
    {
        debug!("deleting {}", self);

        let opts = RemoveContainerOptions {
            v: false,
            force: true,
            link: false,
        };

        let res = client
            .remove_container(self.name.as_ref(), Some(opts))
            .await;

        match res {
            Ok(()) => Ok(Some(())),
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("container not found: {message}");

                Ok(None)
            }
            Err(err) => return Err(ContainerError::Remove(err)),
        }
    }
}

impl<S> Display for Container<S>
where
    S: Display + Hash + Eq,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "container {}/{}", self.name, self.image)
    }
}

impl<'a, S> From<&'a Container<S>> for CreateContainerOptions<&'a str>
where
    S: AsRef<str> + Hash + Eq,
{
    fn from(value: &'a Container<S>) -> Self {
        CreateContainerOptions {
            name: value.name.as_ref(),
            platform: None,
        }
    }
}

impl<'a, S> From<&'a Container<S>> for Config<&'a str>
where
    S: AsRef<str> + Hash + Eq,
{
    fn from(value: &'a Container<S>) -> Self {
        let hostname = value.hostname.as_ref().map(|s| s.as_ref());
        let env = value.env.iter().map(|s| s.as_ref()).collect();
        let binds = value.binds.iter().map(|s| s.as_ref().to_string()).collect();
        let port_bindings = value.as_port_bindings();
        let networks = value.as_network_config();

        Config {
            hostname,
            image: Some(value.image.as_ref()),
            env: Some(env),
            host_config: Some(HostConfig {
                restart_policy: Some(RestartPolicy {
                    name: value.restart_policy,
                    maximum_retry_count: None,
                }),
                binds: Some(binds),
                port_bindings: Some(port_bindings),
                privileged: Some(value.privileged),
                ..Default::default()
            }),
            networking_config: Some(NetworkingConfig {
                endpoints_config: networks,
            }),
            ..Default::default()
        }
    }
}

/// Represents a binding between a host IP address and a host port.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding<S> {
    host_ip: Option<S>,
    host_port: Option<S>,
}

impl<S> From<&Binding<S>> for PortBinding
where
    S: AsRef<str>,
{
    fn from(value: &Binding<S>) -> Self {
        let host_ip = value.host_ip.as_ref().map(|s| s.as_ref().to_string());
        let host_port = value.host_port.as_ref().map(|s| s.as_ref().to_string());

        PortBinding { host_ip, host_port }
    }
}

#[cfg(test)]
mod tests {
    use crate::{docker_mock, image::Image, tests::random_name};

    use super::*;

    #[tokio::test]
    async fn should_create() {
        let name = random_name("create");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{stream, StreamExt};
            let mut mock = Client::new();

            let create_res = bollard::models::ContainerCreateResponse {
                id: "id".to_string(),
                warnings: Vec::new(),
            };

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option.as_ref().map_or(false, |opt| {
                        opt.from_image == "hello-world:latest" && opt.tag == "latest"
                    })
                })
                .once()
                .returning(|_, _, _| stream::empty().boxed());

            let name_cl = name.clone();
            mock.expect_create_container()
                .withf(move |option, config| {
                    option.as_ref().map_or(false, |opt| opt.name == name_cl)
                        && config.image == Some("hello-world")
                })
                .once()
                .returning(move |_, _| Ok(create_res.clone()));

            mock
        });

        let image = Image::new("hello-world", "latest");
        image.pull(&docker).await.unwrap();

        let container = Container::new(name.as_str(), image.name);

        container.create(&docker).await.unwrap();
    }

    #[tokio::test]
    async fn should_inspect() {
        let name = random_name("inspect");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{stream, StreamExt};
            let mut mock = Client::new();

            let create_res = bollard::models::ContainerCreateResponse {
                id: "id".to_string(),
                warnings: Vec::new(),
            };

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option.as_ref().map_or(false, |opt| {
                        opt.from_image == "hello-world:latest" && opt.tag == "latest"
                    })
                })
                .once()
                .returning(|_, _, _| stream::empty().boxed());

            let name_cl = name.clone();
            mock.expect_create_container()
                .withf(move |option, config| {
                    option.as_ref().map_or(false, |opt| opt.name == name_cl)
                        && config.image == Some("hello-world")
                })
                .once()
                .returning(move |_, _| Ok(create_res.clone()));

            let inspect_res = bollard::models::ContainerInspectResponse {
                id: Some("id".to_string()),
                name: Some(format!("/{name}")),
                image: Some("hello-world".to_string()),
                ..Default::default()
            };

            let name_cl = name.clone();
            mock.expect_inspect_container()
                .withf(move |name, _option| name == name_cl)
                .once()
                .returning(move |_, _| Ok(inspect_res.clone()));

            mock
        });

        let image = Image::new("hello-world", "latest");
        image.pull(&docker).await.unwrap();

        let container = Container::new(name.as_str(), image.name);

        container.create(&docker).await.unwrap();

        let resp = container.inspect(&docker).await.unwrap().unwrap();

        assert_eq!(resp.name, Some(format!("/{name}")));
    }

    #[tokio::test]
    async fn should_inspect_not_found() {
        let name = random_name("inspect-not-found");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let name_cl = name.clone();
            mock.expect_inspect_container()
                .withf(move |name, _option| name == name_cl)
                .once()
                .returning(move |_, _| Err(crate::tests::not_found_response()));

            mock
        });

        let container = Container::new(name.as_str(), "hello-world");

        let resp = container.inspect(&docker).await.unwrap();

        assert!(resp.is_none());
    }

    #[tokio::test]
    async fn should_remove() {
        let name = random_name("remove");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{stream, StreamExt};
            let mut mock = Client::new();

            let create_res = bollard::models::ContainerCreateResponse {
                id: "id".to_string(),
                warnings: Vec::new(),
            };

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option.as_ref().map_or(false, |opt| {
                        opt.from_image == "hello-world:latest" && opt.tag == "latest"
                    })
                })
                .once()
                .returning(|_, _, _| stream::empty().boxed());

            let name_cl = name.clone();
            mock.expect_create_container()
                .withf(move |option, config| {
                    option.as_ref().map_or(false, |opt| opt.name == name_cl)
                        && config.image == Some("hello-world")
                })
                .once()
                .returning(move |_, _| Ok(create_res.clone()));

            let name_cl = name.clone();
            mock.expect_remove_container()
                .withf(move |name, _options| name == name_cl)
                .once()
                .returning(move |_, _| Ok(()));

            mock
        });

        let image = Image::new("hello-world", "latest");
        image.pull(&docker).await.unwrap();

        let container = Container::new(name.as_str(), image.name);

        container.create(&docker).await.unwrap();

        container.remove(&docker).await.unwrap();
    }

    #[tokio::test]
    async fn should_remove_not_found() {
        let name = random_name("remove-not-found");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let name_cl = name.clone();
            mock.expect_remove_container()
                .withf(move |name, _options| name == name_cl)
                .once()
                .returning(move |_, _| Err(crate::tests::not_found_response()));

            mock
        });

        let container = Container::new(name.as_str(), "hello-world");

        container.remove(&docker).await.unwrap();
    }
}

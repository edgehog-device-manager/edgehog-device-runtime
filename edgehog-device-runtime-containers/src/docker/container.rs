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
    ops::{Deref, DerefMut},
};

use bollard::{
    container::{
        Config, CreateContainerOptions, InspectContainerOptions, NetworkingConfig,
        RemoveContainerOptions, StartContainerOptions,
    },
    errors::Error as BollardError,
    models::{
        ContainerInspectResponse, EndpointSettings, HostConfig, PortBinding, RestartPolicy,
        RestartPolicyNameEnum,
    },
};
use tracing::{debug, info, instrument, trace, warn};

use crate::{
    client::*,
    requests::{container::parse_port_binding, BindingError},
};

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
    /// couldn't start container.
    Start(#[source] BollardError),
    /// couldn't stop container
    Stop(#[source] BollardError),
    /// missing image reference in container definition
    Image,
}

/// Docker container struct.
#[derive(Debug, Clone, Eq)]
pub struct Container<S>
where
    S: Hash + Eq,
{
    /// Id of the docker container.
    ///
    /// The id of the image is optional since it will be available only when the image is created.
    pub id: Option<String>,
    /// Assign the specified name to the container.
    ///
    /// Must match /?[a-zA-Z0-9][a-zA-Z0-9_.-]+.
    pub name: S,
    /// The name (or reference) of the image to use.
    pub image: S,
    /// Network to link the container with.
    ///
    /// This should be in the form `[https://docker.io/][library/]postgres[:14]` with the fields in
    /// square brackets optional.
    pub networks: Vec<S>,
    /// The hostname to use for the container.
    ///
    /// Defaults to the container name.
    pub hostname: Option<S>,
    /// The behaviour to apply when the container exits.
    ///
    /// See the [create container
    /// API](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerCreate) for
    /// possible values.
    pub restart_policy: RestartPolicyNameEnum,
    /// A list of environment variables to set inside the container.
    ///
    /// In the form of `NAME=VALUE`.
    pub env: Vec<S>,
    /// A list of volume bindings for this container.
    pub binds: Vec<S>,
    /// Describes the mapping of container ports to host ports.
    ///
    /// It uses the container's port-number and protocol as key in the format `<port>/<protocol>`, for
    /// example, 80/udp.
    pub port_bindings: PortBindingMap<S>,
    /// Gives the container full access to the host.
    ///
    /// Defaults to false.
    pub privileged: bool,
}

impl<S> Container<S>
where
    S: Hash + Eq,
{
    /// Create a new container.
    pub fn new(name: S, image: S) -> Self {
        Self {
            id: None,
            name,
            image,
            hostname: None,
            restart_policy: RestartPolicyNameEnum::EMPTY,
            env: Vec::new(),
            binds: Vec::new(),
            networks: Vec::new(),
            port_bindings: PortBindingMap::new(),
            privileged: false,
        }
    }

    /// Get the container id or name if it's missing.
    #[instrument]
    pub fn container(&self) -> &str
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
    #[instrument]
    fn update_id(&mut self, id: String)
    where
        S: Display + Debug,
    {
        info!("using id {id} for container {}", self.name);

        let old_id = self.id.replace(id);

        trace!(?old_id);
    }

    /// Convert the port bindings to be used in [`HostConfig`].
    fn as_port_bindings(&self) -> HashMap<String, Option<Vec<PortBinding>>>
    where
        S: AsRef<str>,
    {
        self.port_bindings
            .iter()
            .map(|(port_proto, binds)| {
                let bindings = if binds.is_empty() {
                    None
                } else {
                    Some(binds.iter().map(PortBinding::from).collect())
                };

                (port_proto.to_string(), bindings)
            })
            .collect()
    }

    /// Convert the networks into [`NetworkingConfig`]
    // TODO: should test the network link from two containers
    fn as_network_config(&self) -> HashMap<&str, EndpointSettings>
    where
        S: AsRef<str>,
    {
        self.networks
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

    /// Check if the container exists, if the id is set, otherwise it will try creating it.
    pub async fn inspect_or_create(&mut self, client: &Client) -> Result<bool, ContainerError>
    where
        S: AsRef<str> + Display + Debug,
    {
        if self.id.is_some() {
            match self.inspect(client).await? {
                Some(net) => {
                    trace!("found container {net:?}");

                    return Ok(false);
                }
                None => {
                    debug!("container not found, creating it");
                }
            }
        }

        self.create(client).await?;

        Ok(true)
    }

    /// Create a new docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerCreate)
    #[instrument]
    pub async fn create(&mut self, client: &Client) -> Result<(), ContainerError>
    where
        S: Debug + Display + AsRef<str>,
    {
        debug!("creating the {}", self);

        let res = client
            .create_container(Some((&*self).into()), (&*self).into())
            .await
            .map_err(ContainerError::Create)?;

        self.update_id(res.id);

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
        &mut self,
        client: &Client,
    ) -> Result<Option<ContainerInspectResponse>, ContainerError>
    where
        S: Debug + Display + AsRef<str>,
    {
        debug!("Inspecting the {}", self);

        let res = client
            .inspect_container(self.container(), None::<InspectContainerOptions>)
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

        if let Some(id) = &container.id {
            self.update_id(id.clone());
        }

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

        let res = client.remove_container(self.container(), Some(opts)).await;

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

    /// Start a docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerStart)
    #[instrument]
    pub async fn start(&self, client: &Client) -> Result<Option<()>, ContainerError>
    where
        S: AsRef<str> + Display + Debug,
    {
        debug!("starting {self}");

        let res = client
            .start_container(self.container(), None::<StartContainerOptions<&str>>)
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
            Err(err) => return Err(ContainerError::Start(err)),
        }
    }

    /// Stop a docker container.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Container/operation/ContainerStop)
    #[instrument]
    pub async fn stop(&self, client: &Client) -> Result<Option<()>, ContainerError>
    where
        S: AsRef<str> + Display + Debug,
    {
        debug!("stopping {self}");

        let res = client.stop_container(self.container(), None).await;

        match res {
            Ok(()) => Ok(Some(())),
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("container not found: {message}");

                Ok(None)
            }
            Err(err) => return Err(ContainerError::Start(err)),
        }
    }
}

impl<S> Display for Container<S>
where
    S: Display + Hash + Eq,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Container")?;

        if let Some(id) = &self.id {
            write!(f, " ({id})")?;
        }

        write!(f, " {}", self.name)
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
        let hostname = value.hostname.as_ref().map(S::as_ref);
        let env = value.env.iter().map(S::as_ref).collect();
        let binds = value.binds.iter().map(|s| s.as_ref().to_string()).collect();
        let port_bindings = value.as_port_bindings();
        let networks = value.as_network_config();

        let restart_policy = RestartPolicy {
            name: Some(value.restart_policy),
            maximum_retry_count: None,
        };

        let host_config = HostConfig {
            restart_policy: Some(restart_policy),
            binds: Some(binds),
            port_bindings: Some(port_bindings),
            privileged: Some(value.privileged),
            ..Default::default()
        };

        let networking_config = NetworkingConfig {
            endpoints_config: networks,
        };

        Config {
            hostname,
            image: Some(value.image.as_ref()),
            env: Some(env),
            host_config: Some(host_config),
            networking_config: Some(networking_config),
            ..Default::default()
        }
    }
}

fn opt_eq<T, U>(opt1: &Option<T>, opt2: &Option<U>) -> bool
where
    T: PartialEq<U>,
{
    match (opt1, opt2) {
        (None, None) => true,
        (None, Some(_)) | (Some(_), None) => false,
        (Some(v1), Some(v2)) => *v1 == *v2,
    }
}

impl<S1, S2> PartialEq<Container<S2>> for Container<S1>
where
    S1: PartialEq<S2> + Eq + Hash,
    S2: Eq + Hash,
{
    fn eq(
        &self,
        Container {
            id,
            name,
            image,
            networks,
            hostname,
            restart_policy,
            env,
            binds,
            port_bindings,
            privileged,
        }: &Container<S2>,
    ) -> bool {
        let eq_port_bindings = self.port_bindings.len() == port_bindings.len()
            && self
                .port_bindings
                .iter()
                .all(|(k, v1)| port_bindings.get(k).map_or(false, |v2| *v1 == *v2));

        self.id.eq(id)
            && self.name.eq(name)
            && self.image.eq(image)
            && self.networks.eq(networks)
            && opt_eq(&self.hostname, hostname)
            && self.restart_policy.eq(restart_policy)
            && self.env.eq(env)
            && self.binds.eq(binds)
            && eq_port_bindings
            && self.privileged.eq(privileged)
    }
}

/// Map of a port/protocol and an array of bindings.
///
/// See [`Container::port_bindings`] for more information.
#[derive(Debug, Clone, Default)]
pub struct PortBindingMap<S>(pub HashMap<String, Vec<Binding<S>>>);

impl<S> PortBindingMap<S> {
    fn new() -> Self {
        Self(HashMap::new())
    }
}

impl TryFrom<Vec<String>> for PortBindingMap<String> {
    type Error = BindingError;

    fn try_from(value: Vec<String>) -> Result<Self, Self::Error> {
        value
            .iter()
            .try_fold(
                HashMap::<String, Vec<Binding<String>>>::new(),
                |mut acc, s| {
                    let bind = parse_port_binding(s)?;

                    let port_binds = acc.entry(bind.id()).or_default();

                    port_binds.push(bind.host.into());

                    Ok(acc)
                },
            )
            .map(PortBindingMap)
    }
}

impl<S> PartialEq for PortBindingMap<S>
where
    S: Eq + Hash,
{
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl<S> Eq for PortBindingMap<S> where S: Eq + Hash {}

impl<S> Deref for PortBindingMap<S>
where
    S: Hash + Eq,
{
    type Target = HashMap<String, Vec<Binding<S>>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> DerefMut for PortBindingMap<S>
where
    S: Hash + Eq,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// Represents a binding between a host IP address and a host port.
#[derive(Debug, Clone, Eq)]
pub struct Binding<S> {
    /// Host IP
    pub host_ip: Option<S>,
    /// Host port
    pub host_port: Option<u16>,
}

impl<S> From<&Binding<S>> for PortBinding
where
    S: AsRef<str>,
{
    fn from(value: &Binding<S>) -> Self {
        let host_ip = value.host_ip.as_ref().map(|s| s.as_ref().to_string());
        let host_port = value.host_port.map(|p| p.to_string());

        PortBinding { host_ip, host_port }
    }
}

impl<'a> From<&'a Binding<String>> for Binding<&'a str> {
    fn from(value: &'a Binding<String>) -> Self {
        Binding {
            host_ip: value.host_ip.as_deref(),
            host_port: value.host_port,
        }
    }
}

impl From<Binding<&str>> for Binding<String> {
    fn from(value: Binding<&str>) -> Self {
        Binding {
            host_ip: value.host_ip.map(ToString::to_string),
            host_port: value.host_port,
        }
    }
}

impl<S> Display for Binding<S>
where
    S: AsRef<str>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.host_ip, self.host_port) {
            (None, None) => Ok(()),
            (Some(ip), None) => write!(f, "{}", ip.as_ref()),
            (None, Some(port)) => write!(f, "{port}"),
            (Some(ip), Some(port)) => write!(f, "{}:{port}", ip.as_ref()),
        }
    }
}

impl<S1, S2> PartialEq<Binding<S2>> for Binding<S1>
where
    S1: PartialEq<S2>,
{
    fn eq(&self, Binding { host_ip, host_port }: &Binding<S2>) -> bool {
        opt_eq(&self.host_ip, host_ip) && self.host_port.eq(host_port)
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
            let mut seq = mockall::Sequence::new();

            let create_res = bollard::models::ContainerCreateResponse {
                id: "id".to_string(),
                warnings: Vec::new(),
            };

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option
                        .as_ref()
                        .map_or(false, |opt| opt.from_image == "hello-world:latest")
                })
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(|name| name == "hello-world:latest")
                .once()
                .in_sequence(&mut seq)
                .returning(|_| {
                    Ok(bollard::secret::ImageInspect {
                        id: Some(
                            "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string(),
                        ),
                        ..Default::default()
                    })
                });

            let name_cl = name.clone();
            mock.expect_create_container()
                .withf(move |option, config| {
                    option.as_ref().map_or(false, |opt| opt.name == name_cl)
                        && config.image == Some("hello-world:latest")
                })
                .once()
                .in_sequence(&mut seq)
                .returning(move |_, _| Ok(create_res.clone()));

            mock
        });

        let mut image = Image::new("hello-world:latest", None);
        image.create(&docker).await.unwrap();

        let mut container = Container::new(name.as_str(), image.reference);

        container.create(&docker).await.unwrap();
    }

    #[tokio::test]
    async fn should_inspect() {
        let name = random_name("inspect");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{stream, StreamExt};
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            let create_res = bollard::models::ContainerCreateResponse {
                id: "id".to_string(),
                warnings: Vec::new(),
            };

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option
                        .as_ref()
                        .map_or(false, |opt| opt.from_image == "hello-world:latest")
                })
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(|name| name == "hello-world:latest")
                .once()
                .in_sequence(&mut seq)
                .returning(|_| {
                    Ok(bollard::secret::ImageInspect {
                        id: Some(
                            "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string(),
                        ),
                        ..Default::default()
                    })
                });

            let name_cl = name.clone();
            mock.expect_create_container()
                .withf(move |option, config| {
                    option.as_ref().map_or(false, |opt| opt.name == name_cl)
                        && config.image == Some("hello-world:latest")
                })
                .once()
                .in_sequence(&mut seq)
                .returning(move |_, _| Ok(create_res.clone()));

            let inspect_res = bollard::models::ContainerInspectResponse {
                id: Some("id".to_string()),
                name: Some(format!("/{name}")),
                image: Some("hello-world".to_string()),
                ..Default::default()
            };

            mock.expect_inspect_container()
                .withf(move |id, _option| id == "id")
                .once()
                .returning(move |_, _| Ok(inspect_res.clone()));

            mock
        });

        let mut image = Image::new("hello-world:latest", None);
        image.create(&docker).await.unwrap();

        let mut container = Container::new(name.as_str(), image.reference);

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

        let mut container = Container::new(name.as_str(), "hello-world");

        let resp = container.inspect(&docker).await.unwrap();

        assert!(resp.is_none());
    }

    #[tokio::test]
    async fn should_remove() {
        let name = random_name("remove");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{stream, StreamExt};
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            let create_res = bollard::models::ContainerCreateResponse {
                id: "id".to_string(),
                warnings: Vec::new(),
            };

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option
                        .as_ref()
                        .map_or(false, |opt| opt.from_image == "hello-world:latest")
                })
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(|name| name == "hello-world:latest")
                .once()
                .in_sequence(&mut seq)
                .returning(|_| {
                    Ok(bollard::secret::ImageInspect {
                        id: Some(
                            "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string(),
                        ),
                        ..Default::default()
                    })
                });

            let name_cl = name.clone();
            mock.expect_create_container()
                .withf(move |option, config| {
                    option.as_ref().map_or(false, |opt| opt.name == name_cl)
                        && config.image == Some("hello-world:latest")
                })
                .once()
                .in_sequence(&mut seq)
                .returning(move |_, _| Ok(create_res.clone()));

            mock.expect_remove_container()
                .withf(move |id, _options| id == "id")
                .once()
                .in_sequence(&mut seq)
                .returning(move |_, _| Ok(()));

            mock
        });

        let mut image = Image::new("hello-world:latest", None);
        image.create(&docker).await.unwrap();

        let mut container = Container::new(name.as_str(), image.reference);

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

    #[test]
    fn to_string_bind() {
        let cases = [
            (
                Binding {
                    host_ip: Some("127.0.0.1"),
                    host_port: Some(80),
                },
                "127.0.0.1:80",
            ),
            (
                Binding {
                    host_ip: Some("127.0.0.1"),
                    host_port: None,
                },
                "127.0.0.1",
            ),
            (
                Binding {
                    host_ip: None,
                    host_port: Some(80),
                },
                "80",
            ),
        ];

        for (case, expect) in cases {
            assert_eq!(case.to_string(), expect)
        }
    }
}

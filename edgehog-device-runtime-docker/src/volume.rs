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

//! Docker Volume struct to pull it from a registry.

use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    hash::Hash,
};

use bollard::{
    errors::Error as BollardError,
    models::Volume as DockerVolume,
    volume::{CreateVolumeOptions, ListVolumesOptions},
};
use serde::Serialize;
use tracing::{debug, error, instrument, warn};

use crate::client::*;

/// Error for the image operations.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum VolumeError {
    /// couldn't create the volume
    Create(#[source] BollardError),
    /// couldn't inspect volume
    Inspect(#[source] BollardError),
    /// couldn't complete volume operation, volume in use
    InUse(#[source] BollardError),
    /// couldn't remove volume
    Remove(#[source] BollardError),
    /// couldn't list volumes
    List(#[source] BollardError),
}

/// Docker volume struct.
///
/// Persistent storage that can be attached to containers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume<S> {
    /// The volume's name. If not specified (empty), Docker generates a name.
    pub name: S,
    /// Name of the volume driver to use.
    ///
    /// Defaults to "local".
    pub driver: S,
    /// A mapping of driver options and values.
    ///
    /// These options are passed directly to the driver and are driver specific.
    pub driver_opts: HashMap<String, S>,
}

impl<S> Display for Volume<S>
where
    S: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "volume {}/{}", self.name, self.driver)
    }
}

impl<S> Volume<S> {
    /// Create a new volume.
    pub fn new(name: S, driver: S) -> Self {
        Self {
            name,
            driver,
            driver_opts: HashMap::new(),
        }
    }

    /// Create a new volume with the options for the driver.
    pub fn with_options(name: S, driver: S, driver_opts: HashMap<String, S>) -> Self {
        Self {
            name,
            driver,
            driver_opts,
        }
    }

    /// Create a new docker volume.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Volume/operation/VolumeCreate)
    #[instrument]
    pub async fn create(&self, client: &Client) -> Result<(), VolumeError>
    where
        S: Debug + Display + AsRef<str>,
    {
        debug!("Create the volume {}", self);

        client
            .create_volume(self.into())
            .await
            .map_err(VolumeError::Create)?;

        Ok(())
    }

    /// Inspect a docker volume.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Volume/operation/VolumeInspect)
    #[instrument]
    pub async fn inspect(&self, client: &Client) -> Result<Option<DockerVolume>, VolumeError>
    where
        S: Debug + Display + AsRef<str>,
    {
        debug!("inspecting volume {}", self.name);

        let res = client.inspect_volume(self.name.as_ref()).await;

        let volume = match res {
            Ok(volume) => volume,
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("volume not found: {message}");

                return Ok(None);
            }
            Err(err) => return Err(VolumeError::Inspect(err)),
        };

        debug!("volume info: {volume:?}");

        Ok(Some(volume))
    }

    /// Remove a docker volume.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Volume/operation/VolumeDelete)
    #[instrument]
    pub async fn remove(&self, client: &Client) -> Result<Option<()>, VolumeError>
    where
        S: Debug + Display + AsRef<str>,
    {
        debug!("deleting volume {}", self.name);

        let res = client.remove_volume(self.name.as_ref(), None).await;

        match res {
            Ok(()) => Ok(Some(())),
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("volume not found: {message}");

                Ok(None)
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 409,
                message,
            }) => {
                error!("cannot remove volume in use: {message}");

                Err(VolumeError::InUse(
                    bollard::errors::Error::DockerResponseServerError {
                        status_code: 409,
                        message,
                    },
                ))
            }
            Err(err) => Err(VolumeError::Remove(err)),
        }
    }
}

impl Volume<String> {
    /// List all docker volumes.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Volume/operation/VolumeDelete)
    #[instrument]
    pub async fn list<T>(
        client: &Client,
        options: Option<ListVolumesOptions<T>>,
    ) -> Result<Vec<Self>, VolumeError>
    where
        T: Into<String> + Serialize + Hash + Eq + Debug,
    {
        debug!("listing volumes");

        let volumes = client
            .list_volumes(Self::convert_option(options))
            .await
            .map_err(VolumeError::List)?;

        if let Some(warnings) = volumes.warnings {
            if !warnings.is_empty() {
                warn!("warnings when listing images: {warnings:?}");
            }
        }

        Ok(volumes
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(Volume::from)
            .collect())
    }

    /// Identity
    #[cfg(not(feature = "mock"))]
    #[inline]
    fn convert_option<T>(options: Option<ListVolumesOptions<T>>) -> Option<ListVolumesOptions<T>>
    where
        T: Debug + Serialize + Into<String> + Hash + Eq,
    {
        options
    }

    /// This is done to be able to mock the function and being able to call the list with a
    /// String.
    #[cfg(feature = "mock")]
    #[inline]
    fn convert_option<T>(
        options: Option<ListVolumesOptions<T>>,
    ) -> Option<ListVolumesOptions<String>>
    where
        T: Debug + Serialize + Into<String> + Hash + Eq,
    {
        options.map(|ListVolumesOptions { filters }| {
            let filters = filters
                .into_iter()
                .map(|(k, v)| (k.into(), v.into_iter().map(T::into).collect()))
                .collect();
            ListVolumesOptions::<String> { filters }
        })
    }
}

impl<'a, S> From<&'a Volume<S>> for CreateVolumeOptions<&'a str>
where
    S: AsRef<str>,
{
    fn from(value: &'a Volume<S>) -> Self {
        CreateVolumeOptions {
            name: value.name.as_ref(),
            driver: value.driver.as_ref(),
            driver_opts: value
                .driver_opts
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_ref()))
                .collect(),
            ..Default::default()
        }
    }
}

impl From<DockerVolume> for Volume<String> {
    fn from(value: DockerVolume) -> Self {
        Volume::with_options(value.name, value.driver, value.options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{docker_mock, tests::random_name};

    #[tokio::test]
    async fn should_create_volume() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            mock.expect_create_volume()
                .withf(|option| option.name == "volume-name" && option.driver == "local")
                .once()
                .returning(|_| Ok(Default::default()));

            mock
        });

        let volume = Volume::new("volume-name", "local");
        volume.create(&docker).await.unwrap();
    }

    #[tokio::test]
    async fn should_inspect_volume() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let volume = bollard::models::Volume {
                name: "volume-name".to_string(),
                driver: "local".to_string(),
                ..Default::default()
            };

            let v_cl = volume.clone();
            mock.expect_create_volume()
                .withf(|option| option.name == "volume-name" && option.driver == "local")
                .once()
                .returning(move |_| Ok(v_cl.clone()));

            mock.expect_inspect_volume()
                .withf(|name| name == "volume-name")
                .once()
                .returning(move |_| Ok(volume.clone()));

            mock
        });

        let volume = Volume::new("volume-name", "local");

        volume.create(&docker).await.unwrap();
        let v = volume.inspect(&docker).await.unwrap().unwrap();

        assert_eq!(v.name, "volume-name")
    }

    #[tokio::test]
    async fn should_inspect_volume_not_found() {
        // Random volume name
        let name = random_name();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let name = name.clone();
            mock.expect_inspect_volume()
                .withf(move |v_name| v_name == name)
                .once()
                .returning(move |_| Err(crate::tests::not_found_response()));

            mock
        });

        let volume = Volume::new(name.as_str(), "local");

        let res = volume.inspect(&docker).await.unwrap();

        assert_eq!(res, None)
    }

    #[tokio::test]
    async fn should_remove_volume() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            mock.expect_create_volume()
                .withf(|option| option.name == "volume-name" && option.driver == "local")
                .once()
                .returning(|_| Ok(Default::default()));

            mock.expect_remove_volume()
                .withf(|name, _| name == "volume-name")
                .once()
                .returning(|_, _| Ok(()));

            mock.expect_inspect_volume()
                .withf(move |v_name| v_name == "volume-name")
                .once()
                .returning(move |_| Err(crate::tests::not_found_response()));

            mock
        });

        let volume = Volume::new("volume-name", "local");
        volume.create(&docker).await.unwrap();
        volume.remove(&docker).await.unwrap();

        let v = volume.inspect(&docker).await.unwrap();

        assert_eq!(v, None);
    }

    #[tokio::test]
    async fn should_list_volumes() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use bollard::service::VolumeListResponse;

            let mut mock = Client::new();

            mock.expect_create_volume()
                .withf(|option| option.name == "volume-name" && option.driver == "local")
                .once()
                .returning(|_| Ok(Default::default()));

            mock.expect_list_volumes()
                .withf(|_| true)
                .once()
                .returning(|_| {
                    Ok(VolumeListResponse {
                        volumes: Some(vec![DockerVolume {
                            name: "volume-name".to_string(),
                            driver: "local".to_string(),
                            ..Default::default()
                        }]),
                        warnings: None,
                    })
                });

            mock
        });

        let volume = Volume::new("volume-name", "local");
        volume.create(&docker).await.unwrap();

        let filters = HashMap::from_iter([("name", vec!["volume-name"])]);
        let options = ListVolumesOptions { filters };
        let volumes = Volume::list(&docker, Some(options)).await.unwrap();

        let found = volumes.into_iter().any(|v| v == v);

        assert!(found)
    }
}

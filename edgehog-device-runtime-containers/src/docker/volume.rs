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

//! Handles the container Volumes.

use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    ops::{Deref, DerefMut},
    sync::OnceLock,
};

use bollard::{
    errors::Error as BollardError, models::Volume as DockerVolume, models::VolumeCreateOptions,
    query_parameters::RemoveVolumeOptions,
};
use tracing::{debug, error, instrument, trace, warn};
use uuid::Uuid;

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

/// Unique identifier for the Volume
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VolumeId {
    /// The volume's name. If not specified (empty), Docker generates a name.
    pub(crate) name: Uuid,
    /// Cache the name with a single allocation.
    ///
    /// Usually multiple functions are called in sequence.
    name_cache: OnceLock<String>,
}

impl VolumeId {
    pub(crate) fn new(name: Uuid) -> Self {
        Self {
            name,
            name_cache: OnceLock::new(),
        }
    }

    pub(crate) fn name_as_str(&self) -> &str {
        self.name_cache
            .get_or_init(|| self.name.to_string())
            .as_str()
    }

    /// Inspect a docker volume.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Volume/operation/VolumeInspect)
    #[instrument(skip_all)]
    pub async fn inspect(&self, client: &Client) -> Result<Option<DockerVolume>, VolumeError> {
        debug!("inspecting {}", self);

        let res = client.inspect_volume(self.name_as_str()).await;

        match res {
            Ok(volume) => {
                trace!("volume info: {volume:?}");

                Ok(Some(volume))
            }
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("volume not found: {message}");

                Ok(None)
            }
            Err(err) => Err(VolumeError::Inspect(err)),
        }
    }

    /// Remove a docker volume.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Volume/operation/VolumeDelete)
    #[instrument(skip_all)]
    pub async fn remove(&self, client: &Client) -> Result<Option<()>, VolumeError> {
        debug!("deleting {}", self);

        let res = client
            .remove_volume(self.name_as_str(), None::<RemoveVolumeOptions>)
            .await;

        match res {
            Ok(()) => Ok(Some(())),
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("volume not found: {message}");

                Ok(None)
            }
            Err(BollardError::DockerResponseServerError {
                status_code: 409,
                message,
            }) => {
                error!("cannot remove volume in use: {message}");

                Err(VolumeError::InUse(
                    BollardError::DockerResponseServerError {
                        status_code: 409,
                        message,
                    },
                ))
            }
            Err(err) => Err(VolumeError::Remove(err)),
        }
    }
}

impl Display for VolumeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "name: {}", self.name)
    }
}

/// Docker volume struct.
///
/// Persistent storage that can be attached to containers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Volume {
    pub(crate) id: VolumeId,
    /// Name of the volume driver to use.
    ///
    /// Defaults to "local".
    pub(crate) driver: String,
    /// A mapping of driver options and values.
    ///
    /// These options are passed directly to the driver and are driver specific.
    pub(crate) driver_opts: HashMap<String, String>,
}

impl Volume {
    /// Create a new volume.
    pub fn new(name: Uuid, driver: String, driver_opts: HashMap<String, String>) -> Self {
        Self {
            id: VolumeId::new(name),
            driver,
            driver_opts,
        }
    }

    /// Create a new docker volume.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Volume/operation/VolumeCreate)
    #[instrument(skip_all)]
    pub async fn create(&self, client: &Client) -> Result<(), VolumeError> {
        debug!("createing {}", self);

        client
            .create_volume(VolumeCreateOptions::from(self))
            .await
            .map_err(VolumeError::Create)?;

        Ok(())
    }
}

impl Display for Volume {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Volume{{{}}}", self.id)
    }
}

impl Deref for Volume {
    type Target = VolumeId;

    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

impl DerefMut for Volume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.id
    }
}

impl From<&Volume> for VolumeCreateOptions {
    fn from(value: &Volume) -> Self {
        VolumeCreateOptions {
            name: Some(value.name_as_str().to_string()),
            driver: Some(value.driver.clone()),
            driver_opts: Some(value.driver_opts.clone()),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use mockall::predicate;
    use uuid::Uuid;

    use super::*;

    use crate::docker_mock;

    #[tokio::test]
    async fn should_create_volume() {
        let name = Uuid::now_v7();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let name_str = name.to_string();
            mock.expect_create_volume()
                .withf(move |option| {
                    option.name.as_ref().is_some_and(|opt| *opt == name_str)
                        && option
                            .driver
                            .as_ref()
                            .is_some_and(|driver| driver == "local")
                })
                .once()
                .returning(|_| Ok(Default::default()));

            mock
        });

        let volume = Volume::new(name, "local".to_string(), HashMap::new());
        volume.create(&docker).await.unwrap();
    }

    #[tokio::test]
    async fn should_inspect_volume() {
        let name = Uuid::now_v7();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let volume = bollard::models::Volume {
                name: name.to_string(),
                driver: "local".to_string(),
                ..Default::default()
            };

            let v_cl = volume.clone();
            let name_exp = name.to_string();
            mock.expect_create_volume()
                .with(predicate::eq(VolumeCreateOptions {
                    name: Some(name_exp),
                    driver: Some("local".to_string()),
                    driver_opts: Some(HashMap::new()),
                    ..Default::default()
                }))
                .once()
                .returning(move |_| Ok(v_cl.clone()));

            let name_str = name.to_string();
            mock.expect_inspect_volume()
                .withf(move |name| name == name_str)
                .once()
                .returning(move |_| Ok(volume.clone()));

            mock
        });

        let volume = Volume::new(name, "local".to_string(), HashMap::new());

        volume.create(&docker).await.unwrap();
        let v = volume.inspect(&docker).await.unwrap().unwrap();

        assert_eq!(v.name, name.to_string())
    }

    #[tokio::test]
    async fn should_inspect_volume_not_found() {
        // Random volume name
        let name = Uuid::now_v7();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let name = name.to_string();
            mock.expect_inspect_volume()
                .withf(move |v_name| v_name == name)
                .once()
                .returning(move |_| Err(crate::tests::not_found_response()));

            mock
        });

        let volume = Volume::new(name, "local".to_string(), HashMap::new());

        let res = volume.inspect(&docker).await.unwrap();

        assert_eq!(res, None)
    }

    #[tokio::test]
    async fn should_remove_volume() {
        let name = Uuid::now_v7();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let name_exp = name.to_string();
            mock.expect_create_volume()
                .withf(move |option| {
                    option.name.as_ref().is_some_and(|name| *name == name_exp)
                        && option
                            .driver
                            .as_ref()
                            .is_some_and(|driver| driver == "local")
                })
                .once()
                .returning(|_| Ok(Default::default()));

            mock.expect_remove_volume()
                .with(predicate::eq(name.to_string()), predicate::eq(None))
                .once()
                .returning(|_, _| Ok(()));

            let name_cl = name.to_string();
            mock.expect_inspect_volume()
                .withf(move |v_name| v_name == name_cl)
                .once()
                .returning(move |_| Err(crate::tests::not_found_response()));

            mock
        });

        let volume = Volume::new(name, "local".to_string(), HashMap::new());
        volume.create(&docker).await.unwrap();
        volume.remove(&docker).await.unwrap();

        let v = volume.inspect(&docker).await.unwrap();

        assert_eq!(v, None);
    }
}

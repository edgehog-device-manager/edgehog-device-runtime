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
};

use bollard::{
    errors::Error as BollardError, models::Volume as DockerVolume, volume::CreateVolumeOptions,
};
use tracing::{debug, error, instrument, trace, warn};

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
#[derive(Debug, Clone, Eq)]
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

impl<S> Volume<S> {
    /// Create a new volume.
    pub fn new(name: S, driver: S, driver_opts: HashMap<String, S>) -> Self {
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
        debug!("Create the Volume {}", self);

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

impl<S> Display for Volume<S>
where
    S: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Volume {}/{}", self.name, self.driver)
    }
}

impl<S1, S2> PartialEq<Volume<S2>> for Volume<S1>
where
    S1: PartialEq<S2>,
{
    fn eq(
        &self,
        Volume {
            name,
            driver,
            driver_opts,
        }: &Volume<S2>,
    ) -> bool {
        let eq_driver_opts = self.driver_opts.len() == driver_opts.len()
            && self
                .driver_opts
                .iter()
                .all(|(k, v1)| driver_opts.get(k).map_or(false, |v2| *v1 == *v2));

        self.name.eq(name) && self.driver.eq(driver) && eq_driver_opts
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{docker_mock, tests::random_name};

    #[tokio::test]
    async fn should_create_volume() {
        let name = random_name("create");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let name_cl = name.clone();
            mock.expect_create_volume()
                .withf(move |option| option.name == name_cl && option.driver == "local")
                .once()
                .returning(|_| Ok(Default::default()));

            mock
        });

        let volume = Volume::new(name.as_str(), "local", HashMap::new());
        volume.create(&docker).await.unwrap();
    }

    #[tokio::test]
    async fn should_inspect_volume() {
        let name = random_name("inspect");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let volume = bollard::models::Volume {
                name: name.clone(),
                driver: "local".to_string(),
                ..Default::default()
            };

            let v_cl = volume.clone();
            let name_cl = name.clone();
            mock.expect_create_volume()
                .withf(move |option| option.name == name_cl && option.driver == "local")
                .once()
                .returning(move |_| Ok(v_cl.clone()));

            let name_cl = name.clone();
            mock.expect_inspect_volume()
                .withf(move |name| name == name_cl)
                .once()
                .returning(move |_| Ok(volume.clone()));

            mock
        });

        let volume = Volume::new(name.as_str(), "local", HashMap::new());

        volume.create(&docker).await.unwrap();
        let v = volume.inspect(&docker).await.unwrap().unwrap();

        assert_eq!(v.name, name)
    }

    #[tokio::test]
    async fn should_inspect_volume_not_found() {
        // Random volume name
        let name = random_name("not-found");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let name = name.clone();
            mock.expect_inspect_volume()
                .withf(move |v_name| v_name == name)
                .once()
                .returning(move |_| Err(crate::tests::not_found_response()));

            mock
        });

        let volume = Volume::new(name.as_str(), "local", HashMap::new());

        let res = volume.inspect(&docker).await.unwrap();

        assert_eq!(res, None)
    }

    #[tokio::test]
    async fn should_remove_volume() {
        let name = random_name("remove");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let name_cl = name.clone();
            mock.expect_create_volume()
                .withf(move |option| option.name == name_cl && option.driver == "local")
                .once()
                .returning(|_| Ok(Default::default()));

            let name_cl = name.clone();
            mock.expect_remove_volume()
                .withf(move |name, _| name == name_cl)
                .once()
                .returning(|_, _| Ok(()));

            let name_cl = name.clone();
            mock.expect_inspect_volume()
                .withf(move |v_name| v_name == name_cl)
                .once()
                .returning(move |_| Err(crate::tests::not_found_response()));

            mock
        });

        let volume = Volume::new(name.as_str(), "local", HashMap::new());
        volume.create(&docker).await.unwrap();
        volume.remove(&docker).await.unwrap();

        let v = volume.inspect(&docker).await.unwrap();

        assert_eq!(v, None);
    }
}

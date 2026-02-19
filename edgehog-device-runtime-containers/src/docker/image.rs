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

//! Handle the calls to manage an Image.

use std::{
    fmt::{Debug, Display},
    hash::Hash,
    ops::{Deref, DerefMut},
};

use base64::Engine;
use bollard::{
    auth::DockerCredentials,
    errors::Error as BollardError,
    models::{ImageDeleteResponseItem, ImageInspect},
    query_parameters::{CreateImageOptions, RemoveImageOptions},
};
use futures::{TryStreamExt, future};
use tracing::{debug, error, info, instrument, trace, warn};

use crate::client::*;

/// Error for the image operations.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum ImageError {
    /// couldn't pull the image
    Pull(#[source] BollardError),
    /// couldn't inspect the image
    Inspect(#[source] BollardError),
    /// couldn't remove image
    Remove(#[source] BollardError),
    /// couldn't remove image in use by container
    ImageInUse(#[source] BollardError),
    /// couldn't list the images
    List(#[source] BollardError),
    /// couldn't convert container summary
    Convert(#[from] ConversionError),
    /// couldn't decode the registry auth
    RegistryAuth(#[from] RegistryAuthError),
}

/// Error while decoding the Registry Authentication.
///
/// It should be a url safe base64 encoded JSON with padding
#[non_exhaustive]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum RegistryAuthError {
    /// couldn't base64 decode the registry auth
    Base64(#[from] base64::DecodeError),
    /// couldn't decode the registry auth JSON
    Json(#[from] serde_json::Error),
}

/// Error for the converting a value to an [`Image`]
#[non_exhaustive]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum ConversionError {
    /// couldn't get tag from untagged image
    UntaggedImage,
    /// couldn't extract the tag for the image
    MissingTag,
}

/// Unique identifier for an image
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ImageId {
    /// Id of the crated image.
    pub id: Option<String>,
    /// Reference to the image that we need to pull.
    pub reference: String,
}

impl ImageId {
    fn id_or_ref(&self) -> &str {
        match &self.id {
            Some(id) => {
                debug!("using image id {id}");

                id.as_str()
            }
            None => {
                debug!("using image reference {}", self.reference);

                self.reference.as_ref()
            }
        }
    }

    fn update(&mut self, id: String) {
        let old = self.id.replace(id);

        debug!(?old, "replaced");
    }

    pub(crate) async fn inspect(
        &mut self,
        client: &Client,
    ) -> Result<Option<ImageInspect>, ImageError> {
        // We need to account to the case that we have an incorrect id, but it exists another
        // image with the correct reference
        if let Some(id) = self.id.clone() {
            debug!("inspecting by id");

            let response = self.inspect_with(client, &id).await?;

            if response.is_some() {
                return Ok(response);
            }
        }

        self.inspect_with(client, &self.reference.to_string()).await
    }

    /// Inspect a docker image.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Image/operation/ImageInspect)
    #[instrument(skip(self, client))]
    async fn inspect_with(
        &mut self,
        client: &Client,
        name: &str,
    ) -> Result<Option<ImageInspect>, ImageError> {
        let res = client.inspect_image(name).await;

        trace!("received response {res:?}");

        // Check if the image was not found
        let inspect = match res {
            Ok(inspect) => inspect,
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                trace!("image not found: {message}");

                return Ok(None);
            }
            Err(err) => return Err(ImageError::Inspect(err)),
        };

        if let Some(id) = &inspect.id {
            self.update(id.clone());
        }

        trace!("inspected image: {inspect:?}");

        Ok(Some(inspect))
    }
}

impl Display for ImageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(id) = &self.id {
            write!(f, "id: {id}, ")?;
        }

        write!(f, "reference: {}", self.reference)
    }
}

/// Docker image struct.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Image {
    image_id: ImageId,
    /// Authentication to the registry.
    ///
    /// See <https://docs.docker.com/reference/api/engine/version/v1.43/#section/Authentication>
    /// for more information.
    pub(crate) registry_auth: Option<String>,
}

impl Image {
    /// Create an image with tag and repo.
    pub fn new(
        id: Option<String>,
        reference: impl Into<String>,
        registry_auth: Option<String>,
    ) -> Self {
        Self {
            image_id: ImageId {
                id,
                reference: reference.into(),
            },
            registry_auth,
        }
    }

    fn docker_credentials(&self) -> Result<Option<DockerCredentials>, RegistryAuthError> {
        self.registry_auth
            .as_ref()
            .map(|auth| {
                trace!("decoding registry credentials");

                base64::engine::general_purpose::URL_SAFE
                    .decode(auth)
                    .map_err(RegistryAuthError::Base64)
                    .and_then(|json| {
                        trace!("deserializing registry credentials");

                        serde_json::from_slice::<DockerCredentials>(&json)
                            .map_err(RegistryAuthError::Json)
                    })
            })
            .transpose()
    }

    /// Pull the docker image struct.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Image/operation/ImageCreate)
    #[instrument(skip_all, fields(image = self.id_or_ref()))]
    pub async fn pull(&mut self, client: &Client) -> Result<(), ImageError> {
        let options = CreateImageOptions::from(&*self);

        debug!("Creating the {}", self);

        let auth = self.docker_credentials()?;

        client
            .create_image(Some(options), None, auth)
            .try_for_each(|create_info| {
                trace!("creating image: {:?}", create_info);

                if let Some(err) = create_info.error_detail {
                    error!(
                        "create {self} error details with code {:?} and message: {:?}",
                        err.code, err.message
                    );
                }

                future::ready(Ok(()))
            })
            .await
            .map_err(ImageError::Pull)?;

        // Set the id and check the errors
        self.inspect(client).await?;

        info!("{} created", self);

        Ok(())
    }

    /// Remove an image and it's parents.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Image/operation/ImageDelete)
    #[instrument(skip_all, fields(image = self.id_or_ref()))]
    pub async fn remove(
        &mut self,
        client: &Client,
    ) -> Result<Option<Vec<ImageDeleteResponseItem>>, ImageError> {
        let id = self.id_or_ref();

        debug!("removing {self}");

        let res = client
            .remove_image(id, None::<RemoveImageOptions>, self.docker_credentials()?)
            .await;

        match res {
            Ok(delete_res) => {
                info!("removed {self}");

                self.image_id.id = None;

                Ok(Some(delete_res))
            }
            Err(BollardError::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("image not found: {message}");

                Ok(None)
            }
            Err(BollardError::DockerResponseServerError {
                status_code: 409,
                message,
            }) => {
                error!("cannot remove image in use: {}", &message);

                // We need to unpack and repack the struct to access the fields
                Err(ImageError::ImageInUse(
                    BollardError::DockerResponseServerError {
                        status_code: 409,
                        message,
                    },
                ))
            }
            Err(err) => Err(ImageError::Remove(err)),
        }
    }
}

impl Deref for Image {
    type Target = ImageId;

    fn deref(&self) -> &Self::Target {
        &self.image_id
    }
}

impl DerefMut for Image {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.image_id
    }
}

impl Display for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image{{{}}}", self.image_id)
    }
}

impl From<&Image> for CreateImageOptions {
    fn from(value: &Image) -> Self {
        CreateImageOptions {
            from_image: Some(value.reference.clone()),
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
    async fn pull_hello_world() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{StreamExt, stream};
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option
                        .as_ref()
                        .and_then(|opt| opt.from_image.as_ref())
                        .is_some_and(|img| img == "hello-world:latest")
                })
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(|name| name == "hello-world:latest")
                .once()
                .in_sequence(&mut seq)
                .returning(|_| {
                    Ok(bollard::models::ImageInspect {
                    id: Some(
                        "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string(),
                    ),
                    ..Default::default()
                })
                });

            mock
        });

        let mut image = Image::new(None, "hello-world:latest", None);

        image
            .pull(&docker)
            .await
            .expect("error while pulling the image");
    }

    #[tokio::test]
    async fn inspect_hello_world() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{StreamExt, stream};
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            mock.expect_create_image()
                .withf(|options, _, _| {
                    options
                        .as_ref()
                        .and_then(|opt| opt.from_image.as_ref())
                        .is_some_and(|img| img == "hello-world:latest")
                })
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(|name| name == "hello-world:latest")
                .once()
                .in_sequence(&mut seq)
                .returning(|_| {
                    Ok(ImageInspect {
                        id: Some(
                            "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string(),
                        ),
                        ..Default::default()
                    })
                });

            mock.expect_inspect_image()
                .withf(|name| name == "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a")
                .once()
                .in_sequence(&mut seq)
                .returning(|_| {
                    Ok(ImageInspect {
                        id: Some(
                            "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string(),
                        ),
                        ..Default::default()
                    })
                });

            mock
        });

        let mut image = Image::new(None, "hello-world:latest", None);

        image.pull(&docker).await.expect("failed to poll image");

        let inspect = image
            .inspect(&docker)
            .await
            .expect("failed to inspect image")
            .expect("image not found");

        assert_eq!(inspect.id, image.image_id.id);
    }

    #[tokio::test]
    async fn inspect_not_found() {
        // Random image name
        let name = Uuid::now_v7();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            mock.expect_inspect_image()
                .with(predicate::eq(name.to_string()))
                .once()
                .in_sequence(&mut seq)
                .returning(|_| Err(crate::tests::not_found_response()));

            mock
        });

        let mut image = Image::new(None, name, None);

        let inspect = image
            .inspect(&docker)
            .await
            .expect("failed to inspect image");

        assert_eq!(inspect, None);
    }

    #[tokio::test]
    async fn remove_image() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{StreamExt, stream};

            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option
                        .as_ref()
                        .and_then(|opt| opt.from_image.as_ref())
                        .is_some_and(|from_image| from_image == "alpine:edge")
                })
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(|name| name == "alpine:edge")
                .once()
                .in_sequence(&mut seq)
                .returning(|_| {
                    Ok(ImageInspect {
                        id: Some(
                            "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string(),
                        ),
                        repo_tags: Some(vec![
                            "hello-world:latest".to_string(),
                            "hello-world:linux".to_string(),
                        ]),
                        ..Default::default()
                    })
                });

            mock.expect_remove_image()
                .withf(|name, _, _| {
                    name
                        == "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a"
                })
                .once()
                .returning(|_, _, _| {
                    Ok(vec![
                        ImageDeleteResponseItem {
                            untagged: Some("alpine:edge".to_string()),
                            deleted: None,
                        },
                        ImageDeleteResponseItem {
                            untagged: None,
                            deleted: Some("sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string()),
                        },
                    ])
                });

            mock
        });

        let mut image = Image::new(None, "alpine:edge", None);

        image.pull(&docker).await.expect("failed to pull");

        let res = image
            .clone()
            .remove(&docker)
            .await
            .expect("error removing")
            .expect("none response");

        assert!(
            res.iter()
                .filter_map(|i| i.untagged.as_deref())
                .any(|untagged| untagged == "alpine:edge"
                    || untagged == "docker.io/library/alpine:edge"),
            "no untagged in {res:?}"
        );
        let id = image.image_id.id.unwrap();
        assert!(
            res.iter()
                .filter_map(|i| i.deleted.as_deref())
                // This is different between docker and podman
                .any(|deleted| id.ends_with(deleted)),
            "no deleted {id} in {res:?}"
        );
    }

    #[tokio::test]
    async fn remove_image_not_found() {
        // Random image name
        let name = Uuid::now_v7();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            mock.expect_remove_image()
                .with(
                    predicate::eq(name.to_string()),
                    predicate::eq(None),
                    predicate::eq(None),
                )
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| Err(crate::tests::not_found_response()));

            mock
        });

        let mut image = Image::new(None, name, None);

        let res = image.remove(&docker).await.expect("error removing");
        assert_eq!(res, None);
    }

    #[test]
    fn should_decode_auth() {
        let image = Image {
            image_id: ImageId {
                id: None,
                reference: "alpine".to_string(),
            },
            registry_auth: Some(
                "eyJ1c2VybmFtZSI6InVzZXIiLCJwYXNzd29yZCI6InBhc3N3ZCJ9Cg==".to_string(),
            ),
        };

        let exp = DockerCredentials {
            username: Some("user".to_string()),
            password: Some("passwd".to_string()),
            ..Default::default()
        };
        let cred = image.docker_credentials().unwrap().unwrap();
        assert_eq!(cred, exp)
    }
}

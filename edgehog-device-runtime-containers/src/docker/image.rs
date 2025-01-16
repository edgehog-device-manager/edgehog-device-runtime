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
};

use base64::Engine;
use bollard::{
    auth::DockerCredentials,
    errors::Error as BollardError,
    image::CreateImageOptions,
    models::{ImageDeleteResponseItem, ImageInspect},
};
use futures::{future, TryStreamExt};
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

/// Docker image struct.
#[derive(Debug, Clone, Eq, Hash)]
pub struct Image<S = String> {
    /// Id of the crated image.
    pub id: Option<String>,
    /// Reference to the image that we need to pull.
    pub reference: S,
    /// Authentication to the registry.
    ///
    /// See <https://docs.docker.com/reference/api/engine/version/v1.43/#section/Authentication>
    /// for more information.
    pub registry_auth: Option<S>,
}

impl<S> Image<S> {
    /// Create a new docker image.
    pub fn new(reference: S, registry_auth: Option<S>) -> Self {
        Self::with_id(None, reference, registry_auth)
    }

    /// Create an image with tag and repo.
    pub fn with_id(id: Option<String>, reference: S, registry_auth: Option<S>) -> Self {
        Self {
            id,
            reference,
            registry_auth,
        }
    }

    fn docker_credentials(&self) -> Result<Option<DockerCredentials>, RegistryAuthError>
    where
        S: AsRef<str>,
    {
        self.registry_auth
            .as_ref()
            .map(|auth| {
                trace!("decoding registry credentials");

                base64::engine::general_purpose::URL_SAFE
                    .decode(auth.as_ref())
                    .map_err(RegistryAuthError::Base64)
                    .and_then(|json| {
                        trace!("deserializing registry credentials");

                        serde_json::from_slice::<DockerCredentials>(&json)
                            .map_err(RegistryAuthError::Json)
                    })
            })
            .transpose()
    }

    fn id_or_ref(&self) -> &str
    where
        S: AsRef<str> + Debug + Display,
    {
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

    /// Create the image only if it doesn't already exists
    pub async fn inspect_or_create(&mut self, client: &Client) -> Result<bool, ImageError>
    where
        S: Debug + Display + AsRef<str>,
    {
        if self.id.is_some() {
            if self.inspect(client).await?.is_some() {
                debug!("{self} already exists, no need to create it");

                return Ok(false);
            }

            warn!("{self} has id, but cannot inspect it");
        }

        self.pull(client).await?;

        Ok(true)
    }

    /// Pull the docker image struct.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Image/operation/ImageCreate)
    #[instrument(skip_all, fields(image = self.id_or_ref()))]
    pub async fn pull(&mut self, client: &Client) -> Result<(), ImageError>
    where
        S: AsRef<str> + Debug + Display,
    {
        let options = CreateImageOptions::from(&*self);

        debug!("Creating the {}", self);

        let auth = self.docker_credentials()?;

        client
            .create_image(Some(options), None, auth)
            .try_for_each(|create_info| {
                trace!("creating image: {:?}", create_info);

                if let Some(err) = create_info.error {
                    error!("create {self} returned an error: {err}");
                }

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

    /// Inspect a docker image.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Image/operation/ImageInspect)
    #[instrument(skip_all, fields(image = self.id_or_ref()))]
    pub async fn inspect(&mut self, client: &Client) -> Result<Option<ImageInspect>, ImageError>
    where
        S: Debug + Display + AsRef<str>,
    {
        let image_name = self.id_or_ref();

        let res = client.inspect_image(image_name).await;

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
            self.id = Some(id.clone());
        }

        trace!("inspected image: {inspect:?}");

        Ok(Some(inspect))
    }

    /// Remove an image and it's parents.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Image/operation/ImageDelete)
    #[instrument(skip_all, fields(image = self.id_or_ref()))]
    pub async fn remove(
        &mut self,
        client: &Client,
    ) -> Result<Option<Vec<ImageDeleteResponseItem>>, ImageError>
    where
        S: AsRef<str> + Debug + Display,
    {
        let id = self.id_or_ref();

        debug!("removing {self}");

        let res = client
            .remove_image(id, None, self.docker_credentials()?)
            .await;

        match res {
            Ok(delete_res) => {
                info!("removed {self}");

                self.id = None;

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

impl<S: Display> Display for Image<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image")?;

        if let Some(id) = &self.id {
            write!(f, " ({id})")?;
        }

        write!(f, " {}", self.reference)
    }
}

impl<S1, S2> PartialEq<Image<S2>> for Image<S1>
where
    S1: PartialEq<S2>,
{
    fn eq(
        &self,
        Image {
            id,
            reference,
            registry_auth,
        }: &Image<S2>,
    ) -> bool {
        let eq_registry_auth = match (&self.registry_auth, registry_auth) {
            (Some(s1), Some(s2)) => *s1 == *s2,
            (Some(_), None) | (None, Some(_)) => false,
            (None, None) => true,
        };

        self.id.eq(id) && self.reference.eq(reference) && eq_registry_auth
    }
}

impl<'a, S> From<&'a Image<S>> for CreateImageOptions<'static, String>
where
    S: Display,
{
    fn from(value: &'a Image<S>) -> Self {
        CreateImageOptions {
            from_image: value.reference.to_string(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{docker_mock, tests::random_name};

    #[tokio::test]
    async fn pull_hello_world() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{stream, StreamExt};
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option
                        .as_ref()
                        .is_some_and(|opt| opt.from_image == "hello-world:latest")
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

        let mut image = Image::new("hello-world:latest", None);

        image
            .pull(&docker)
            .await
            .expect("error while pulling the image");
    }

    #[tokio::test]
    async fn inspect_hello_world() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{stream, StreamExt};
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            mock.expect_create_image()
                .withf(|options, _, _| {
                    options
                        .as_ref()
                        .is_some_and(|opt| opt.from_image == "hello-world:latest")
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

        let mut image = Image::new("hello-world:latest", None);

        image.pull(&docker).await.expect("failed to poll image");

        let inspect = image
            .inspect(&docker)
            .await
            .expect("failed to inspect image")
            .expect("image not found");

        assert_eq!(inspect.id, image.id);
    }

    #[tokio::test]
    async fn inspect_not_found() {
        // Random image name
        let name = random_name("not-found");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            let img_name = name.clone();
            mock.expect_inspect_image()
                .withf(move |img| img == img_name)
                .once()
                .in_sequence(&mut seq)
                .returning(|_| Err(crate::tests::not_found_response()));

            mock
        });

        let mut image = Image::new(name, None);

        let inspect = image
            .inspect(&docker)
            .await
            .expect("failed to inspect image");

        assert_eq!(inspect, None);
    }

    #[tokio::test]
    async fn remove_image() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{stream, StreamExt};

            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option
                        .as_ref()
                        .is_some_and(|opt| opt.from_image == "alpine:edge")
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

        let mut image = Image::new("alpine:edge", None);

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
        let id = image.id.unwrap();
        assert!(
            res.iter()
                .filter_map(|i| i.deleted.as_deref())
                // This is different between docker and podman
                .any(|deleted| id.ends_with(deleted)),
            "no deleted {} in {res:?}",
            id
        );
    }

    #[tokio::test]
    async fn remove_image_not_found() {
        // Random image name
        let name = random_name("remove-not-found");

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            let name_cl = name.clone();
            mock.expect_remove_image()
                .withf(move |name, _, _| name == name_cl)
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| Err(crate::tests::not_found_response()));

            mock
        });

        let mut image = Image::new(name, None);

        let res = image.remove(&docker).await.expect("error removing");
        assert_eq!(res, None);
    }

    #[test]
    fn should_decode_auth() {
        let image = Image {
            id: None,
            reference: "alpine",
            registry_auth: Some("eyJ1c2VybmFtZSI6InVzZXIiLCJwYXNzd29yZCI6InBhc3N3ZCJ9Cg=="),
        };

        let exp = DockerCredentials {
            username: Some("user".to_string()),
            password: Some("passwd".to_string()),
            ..Default::default()
        };
        let cred = image.docker_credentials().unwrap().unwrap();
        assert_eq!(cred, exp)
    }

    #[tokio::test]
    async fn inspect_or_create_hello_world() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{stream, StreamExt};
            let mut mock = Client::new();
            let mut seq = mockall::Sequence::new();

            mock.expect_remove_image()
                .withf(|name, _, _| name == "docker.io/library/nginx:1.27.2-bookworm-perl")
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| Err(crate::tests::not_found_response()));

            mock.expect_create_image()
                .withf(|options, _, _| {
                    options.as_ref().is_some_and(|opt| {
                        opt.from_image == "docker.io/library/nginx:1.27.2-bookworm-perl"
                    })
                })
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(|name| name == "docker.io/library/nginx:1.27.2-bookworm-perl")
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

        let mut image = Image::new("docker.io/library/nginx:1.27.2-bookworm-perl", None);

        image.remove(&docker).await.unwrap();

        let created = image
            .inspect_or_create(&docker)
            .await
            .expect("failed to poll image");

        assert!(created);

        let created = image
            .inspect_or_create(&docker)
            .await
            .expect("failed to poll image");

        assert!(!created);
    }
}

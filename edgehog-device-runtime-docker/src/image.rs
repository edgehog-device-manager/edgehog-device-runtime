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

//! Docker Image struct to pull it from a registry.

use std::{
    fmt::{Debug, Display},
    hash::Hash,
    str::FromStr,
};

use bollard::{
    image::{CreateImageOptions, ListImagesOptions},
    models::{ImageDeleteResponseItem, ImageInspect, ImageSummary},
};
use futures::{future, TryStreamExt};
use serde::Serialize;
use tracing::{debug, error, info, instrument, warn};

use crate::client::*;

/// Error for the image operations.
#[non_exhaustive]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum ImageError {
    /// couldn't pull the image
    Pull(#[source] bollard::errors::Error),
    /// couldn't inspect the image
    Inspect(#[source] bollard::errors::Error),
    /// couldn't remove image
    Remove(#[source] bollard::errors::Error),
    /// couldn't remove image in use by container
    ImageInUse(#[source] bollard::errors::Error),
    /// couldn't list the images
    List(#[source] bollard::errors::Error),
    /// couldn't convert container summary
    Convert(#[from] ConversionError),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Image<S> {
    /// Image name
    pub name: S,
    /// Tag or digest, uses "latest" if `None`
    pub tag: S,
    /// Repository
    pub repo: Option<S>,
}

impl<S: Display> Display for Image<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(repo) = &self.repo {
            write!(f, "{}/", repo)?;
        }

        write!(f, "{}:{}", self.name, self.tag)
    }
}

impl<'a, S> From<&'a Image<S>> for CreateImageOptions<'static, String>
where
    S: Display,
{
    fn from(value: &'a Image<S>) -> Self {
        CreateImageOptions {
            from_image: value.tag(),
            tag: value.tag.to_string(),
            repo: value
                .repo
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            ..Default::default()
        }
    }
}

impl FromStr for Image<String> {
    type Err = ConversionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (rest, tag) = s.rsplit_once(':').ok_or(ConversionError::MissingTag)?;

        let (repo, name) = match rest.split_once('/') {
            Some((repo, name)) => (Some(repo), name),
            None => (None, rest),
        };

        Ok(Image { name, tag, repo }.into())
    }
}

impl TryFrom<ImageSummary> for Image<String> {
    type Error = ConversionError;

    fn try_from(value: ImageSummary) -> Result<Self, Self::Error> {
        let tag = value
            .repo_tags
            .first()
            .ok_or(ConversionError::UntaggedImage)?;

        tag.parse()
    }
}

impl From<Image<&str>> for Image<String> {
    fn from(value: Image<&str>) -> Self {
        Image {
            name: value.name.to_string(),
            tag: value.tag.to_string(),
            repo: value.repo.map(ToString::to_string),
        }
    }
}

impl<S> Image<S> {
    /// Create a new docker image.
    pub fn new(name: S, tag: S) -> Self {
        Self {
            name,
            tag,
            repo: None,
        }
    }

    /// Create an image with tag and repo.
    pub fn with_repo(name: S, tag: S, repo: S) -> Self {
        Self {
            name,
            tag,
            repo: Some(repo),
        }
    }

    /// Get a reference to the image
    pub fn tag(&self) -> String
    where
        S: Display,
    {
        self.to_string()
    }

    /// Pull the docker image struct.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Image/operation/ImageCreate)
    #[instrument]
    pub async fn pull(&self, client: &Client) -> Result<(), ImageError>
    where
        S: Debug + Display,
    {
        let options = CreateImageOptions::from(self);

        debug!("Creating the image {}", self);

        client
            // NOTE: Create the image by pulling it from the registry, here it's missing the root fs
            //       option and the registry auth
            .create_image(Some(options), None, None)
            .try_for_each(|create_info| {
                debug!("creating image: {:?}", create_info);

                future::ready(Ok(()))
            })
            .await
            .map_err(ImageError::Pull)?;

        info!("Image {} created", self);

        Ok(())
    }

    /// Inspect a docker image.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Image/operation/ImageInspect)
    #[instrument]
    pub async fn inspect(&self, client: &Client) -> Result<Option<ImageInspect>, ImageError>
    where
        S: Debug + AsRef<str>,
    {
        let res = client.inspect_image(self.name.as_ref()).await;

        debug!("received response {res:?}");

        // Check if the image was not found
        let inspect = match res {
            Ok(inspect) => inspect,
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                debug!("image not found: {message}");

                return Ok(None);
            }
            Err(err) => return Err(ImageError::Inspect(err)),
        };

        debug!("inspected image: {inspect:?}");

        Ok(Some(inspect))
    }

    /// Remove and image and it's parents.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Image/operation/ImageDelete)
    #[instrument]
    pub async fn remove(
        self,
        client: &Client,
    ) -> Result<Option<Vec<ImageDeleteResponseItem>>, ImageError>
    where
        S: Debug + Display,
    {
        let id = self.to_string();

        debug!("removing image: {id}");

        let res = client.remove_image(&id, None, None).await;

        debug!(?res);

        match res {
            Ok(delete_res) => Ok(Some(delete_res)),
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404,
                message,
            }) => {
                warn!("image not found: {message}");

                Ok(None)
            }
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 409,
                message,
            }) => {
                error!("cannot remove image in use: {}", &message);

                // We need to unpack an repack the struct to access the fields
                Err(ImageError::ImageInUse(
                    bollard::errors::Error::DockerResponseServerError {
                        status_code: 409,
                        message,
                    },
                ))
            }
            Err(err) => Err(ImageError::Remove(err)),
        }
    }
}

impl Image<String> {
    /// List the docker images.
    ///
    /// See the [Docker API reference](https://docs.docker.com/engine/api/v1.43/#tag/Image/operation/ImageList)
    #[instrument]
    pub async fn list<T>(
        client: &Client,
        options: Option<ListImagesOptions<T>>,
    ) -> Result<Vec<Self>, ImageError>
    where
        T: Into<String> + Serialize + Hash + Eq + Debug,
    {
        debug!("listing images");

        let options = Self::convert_option(options);

        client
            .list_images(options)
            .await
            .map_err(ImageError::List)
            .and_then(|images| {
                images
                    .into_iter()
                    .map(|summary| Image::try_from(summary).map(Image::into))
                    .collect::<Result<_, _>>()
                    .map_err(ImageError::from)
            })
    }

    /// Identity
    #[cfg(not(feature = "mock"))]
    #[inline]
    fn convert_option<T>(options: Option<ListImagesOptions<T>>) -> Option<ListImagesOptions<T>>
    where
        T: Debug + Serialize + Into<String> + Hash + Eq,
    {
        options
    }

    /// This is done to be able to mock the function and being able to call the list with a
    /// String.
    #[cfg(feature = "mock")]
    #[inline]
    fn convert_option<T>(options: Option<ListImagesOptions<T>>) -> Option<ListImagesOptions<String>>
    where
        T: Debug + Serialize + Into<String> + Hash + Eq,
    {
        options.map(
            |ListImagesOptions {
                 all,
                 filters,
                 digests,
             }| {
                let filters = filters
                    .into_iter()
                    .map(|(k, v)| (k.into(), v.into_iter().map(T::into).collect()))
                    .collect();
                ListImagesOptions::<String> {
                    all,
                    filters,
                    digests,
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    use crate::docker_mock;

    #[tokio::test]
    async fn poll_hello_world() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            use futures::{stream, StreamExt};
            let mut mock = Client::new();

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option.as_ref().map_or(false, |opt| {
                        opt.from_image == "hello-world:latest" && opt.tag == "latest"
                    })
                })
                .once()
                .returning(|_, _, _| stream::empty().boxed());

            mock
        });

        let image = Image::new("hello-world", "latest");

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

            mock.expect_create_image()
                .withf(|options, _, _| {
                    options.as_ref().map_or(false, |opt| {
                        opt.from_image == "hello-world:latest" && opt.tag == "latest"
                    })
                })
                .once()
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(|name| name == "hello-world")
                .once()
                .returning(|_| {
                    Ok(ImageInspect {
                        repo_tags: Some(vec!["hello-world:latest".to_string()]),
                        ..Default::default()
                    })
                });

            mock
        });

        let image = Image::new("hello-world", "latest");

        image.pull(&docker).await.expect("failed to poll image");

        let inspect = image
            .inspect(&docker)
            .await
            .expect("failed to inspect image")
            .expect("image not found");

        assert_eq!(
            inspect.repo_tags,
            Some(vec!["hello-world:latest".to_string()])
        );
    }

    #[tokio::test]
    async fn inspect_not_found() {
        // Random image name
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let name = time.to_string();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let img_name = name.clone();
            mock.expect_inspect_image()
                .withf(move |img| img == img_name)
                .once()
                .returning(|_| {
                    Err(bollard::errors::Error::DockerResponseServerError {
                        status_code: 404,
                        message: "not found".to_string(),
                    })
                });

            mock
        });

        let image = Image::new(name, "latest".to_string());

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

            mock.expect_create_image()
                .withf(|option, _, _| {
                    option.as_ref().map_or(false, |opt| {
                        opt.from_image == "alpine:edge" && opt.tag == "edge"
                    })
                })
                .once()
                .returning(|_, _, _| stream::empty().boxed());

            mock.expect_remove_image()
                .withf(|name, _, _| name == "alpine:edge")
                .once()
                .returning(|_, _, _| {
                    Ok(vec![ImageDeleteResponseItem {
                        untagged: Some("alpine:edge".to_string()),
                        deleted: None,
                    }])
                });

            mock
        });

        let image = Image::new("alpine", "edge");

        image.pull(&docker).await.expect("failed to pull");

        let res = image
            .remove(&docker)
            .await
            .expect("error removing")
            .expect("none response");

        assert_eq!(
            ImageDeleteResponseItem {
                untagged: Some("alpine:edge".to_string()),
                deleted: None
            },
            res[0]
        );
    }

    #[tokio::test]
    async fn remove_image_not_found() {
        // Random image name
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let name = time.to_string();

        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            let name_cl = name.clone();
            mock.expect_remove_image()
                .withf(move |name, _, _| name == format!("{name_cl}:latest"))
                .once()
                .returning(|_, _, _| {
                    Err(bollard::errors::Error::DockerResponseServerError {
                        status_code: 404,
                        message: "not found".to_string(),
                    })
                });

            mock
        });

        let image = Image::new(name, "latest".to_string());

        let res = image.remove(&docker).await.expect("error removing");
        assert_eq!(res, None);
    }

    #[tokio::test]
    async fn should_list() {
        let docker = docker_mock!(Client::connect_with_local_defaults().unwrap(), {
            let mut mock = Client::new();

            mock.expect_list_images().once().returning(|_| Ok(vec![]));

            mock
        });

        Image::list(&docker, None::<ListImagesOptions<String>>)
            .await
            .unwrap();
    }
}

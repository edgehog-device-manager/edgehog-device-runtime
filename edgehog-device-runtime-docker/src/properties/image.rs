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

//! Property for the `AvailableImage` interface.

use astarte_device_sdk::{
    error::Error as AstarteError,
    store::{PropertyStore, StoredProp},
    DeviceClient,
};
use async_trait::async_trait;
use tracing::warn;

use crate::{docker::image::Image, service::ServiceError};

use super::{astarte_type, replace_if_some, AvailableProp, LoadProp, PropError};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AvailableImage<S> {
    pub(crate) id: S,
    pub(crate) repo: Option<S>,
    pub(crate) name: Option<S>,
    pub(crate) tag: Option<S>,
    pub(crate) pulled: Option<bool>,
}

impl<S> AvailableImage<S> {
    pub(crate) const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.AvailableImages";

    pub(crate) fn new(id: S) -> Self
    where
        S: Default,
    {
        Self {
            id,
            ..Default::default()
        }
    }
}

impl<'a> AvailableImage<&'a str> {
    pub(crate) fn with_image(id: &'a str, image: &'a Image<String>) -> Self {
        Self::with_pulled(id, image, false)
    }

    pub(crate) fn with_pulled(id: &'a str, image: &'a Image<String>, pulled: bool) -> Self {
        Self {
            id,
            repo: image.repo.as_deref(),
            name: Some(&image.name),
            tag: Some(&image.tag),
            pulled: Some(pulled),
        }
    }
}

#[async_trait]
impl<S> AvailableProp for AvailableImage<S>
where
    S: AsRef<str> + Sync + Send,
{
    fn interface() -> &'static str {
        Self::INTERFACE
    }

    fn id(&self) -> &str {
        self.id.as_ref()
    }

    async fn store<T>(&self, device: &DeviceClient<T>) -> Result<(), AstarteError>
    where
        T: PropertyStore,
    {
        let repo = self.repo.as_ref().map(S::as_ref);
        let name = self.repo.as_ref().map(S::as_ref);
        let tag = self.repo.as_ref().map(S::as_ref);

        self.send(device, "repo", repo).await?;
        self.send(device, "name", name).await?;
        self.send(device, "tag", tag).await?;
        self.send(device, "pulled", self.pulled).await?;

        Ok(())
    }

    fn merge(&mut self, other: Self) -> &mut Self {
        self.id = other.id;
        replace_if_some(&mut self.repo, other.repo);
        replace_if_some(&mut self.name, other.name);
        replace_if_some(&mut self.tag, other.tag);
        replace_if_some(&mut self.pulled, other.pulled);

        self
    }
}

impl LoadProp for AvailableImage<String> {
    type Resource = Image<String>;

    fn dependencies(
        &self,
        _nodes: &mut crate::service::Nodes,
    ) -> Result<Vec<petgraph::prelude::NodeIndex>, ServiceError> {
        Ok(Vec::new())
    }
}

impl TryFrom<StoredProp> for AvailableImage<String> {
    type Error = PropError;

    fn try_from(value: StoredProp) -> Result<Self, Self::Error> {
        let (id, field) = Self::parse_endpoint(&value.path)?;

        let mut av_img = AvailableImage::new(id.to_string());

        match field {
            "repo" => astarte_type!(value.value, av_img, repo => String),
            "name" => astarte_type!(value.value, av_img, name => String),
            "tag" => astarte_type!(value.value, av_img, tag => String),
            "pulled" => astarte_type!(value.value, av_img, pulled => bool),
            _ => warn!(
                "unrecognized field for stored property for interface {}/{}:{}",
                value.interface, value.path, value.interface_major
            ),
        }

        Ok(av_img)
    }
}

impl TryFrom<AvailableImage<String>> for Image<String> {
    type Error = PropError;

    fn try_from(value: AvailableImage<String>) -> Result<Self, Self::Error> {
        let name = value.name.ok_or(PropError::field("name", "Image"))?;
        let tag = value.tag.ok_or(PropError::field("tag", "Image"))?;
        let repo = value.repo.ok_or(PropError::field("repo", "Image"))?;

        Ok(Self::with_repo(name, tag, repo))
    }
}

impl From<AvailableImage<&str>> for AvailableImage<String> {
    fn from(value: AvailableImage<&str>) -> Self {
        AvailableImage {
            id: value.id.to_string(),
            repo: value.repo.map(ToString::to_string),
            name: value.name.map(ToString::to_string),
            tag: value.tag.map(ToString::to_string),
            pulled: value.pulled,
        }
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::{interface::def::Ownership, types::AstarteType};

    use super::*;

    #[test]
    fn should_convert_image() {
        let cases = [
            (
                "/1234/name",
                AstarteType::String("image-name".to_string()),
                AvailableImage {
                    id: "1234",
                    name: Some("image-name"),
                    ..Default::default()
                },
            ),
            (
                "/1234/repo",
                AstarteType::String("image-repo".to_string()),
                AvailableImage {
                    id: "1234",
                    repo: Some("image-repo"),
                    ..Default::default()
                },
            ),
            (
                "/1234/tag",
                AstarteType::String("image-tag".to_string()),
                AvailableImage {
                    id: "1234",
                    tag: Some("image-tag"),
                    ..Default::default()
                },
            ),
            (
                "/1234/pulled",
                AstarteType::Boolean(true),
                AvailableImage {
                    id: "1234",
                    pulled: Some(true),
                    ..Default::default()
                },
            ),
        ];

        for (path, value, expected) in cases {
            let prop = StoredProp {
                interface: AvailableImage::<&str>::INTERFACE.to_string(),
                path: path.to_string(),
                value,
                interface_major: 1,
                ownership: Ownership::Device,
            };

            let image = AvailableImage::try_from(prop).unwrap();

            assert_eq!(image, expected.into());
        }
    }
}

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

//! Property to send to Astarte.

use astarte_device_sdk::{
    error::Error as AstarteError,
    store::{PropertyStore, StoredProp},
    types::AstarteType,
    Client, DeviceClient,
};
use tracing::warn;

use crate::image::Image;

/// Error from handling the Astarte properties.
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum PropError {
    /// endpoint missing prefix
    MissingPrefix,
    /// endpoint missing the id
    MissingId,
    /// couldn't convert field {field} from type {got:?} to {exp}
    Type {
        field: &'static str,
        exp: &'static str,
        got: AstarteType,
    },
    /// couldn't convert property into {into}, since it's missing the field {field}
    MissingField {
        field: &'static str,
        into: &'static str,
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AvailableImage<'a> {
    pub(crate) id: &'a str,
    pub(crate) repo: Option<&'a str>,
    pub(crate) name: Option<&'a str>,
    pub(crate) tag: Option<&'a str>,
    pub(crate) pulled: Option<bool>,
}

impl<'a> AvailableImage<'a> {
    pub(crate) const INTERFACE: &'static str = "io.edgehog.devicemanager.apps.AvailableImages";

    pub(crate) fn new(id: &'a str) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }

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

    pub(crate) fn parse_endpoint(endpoint: &str) -> Result<(&str, &str), PropError> {
        endpoint
            .strip_prefix('/')
            .ok_or(PropError::MissingPrefix)
            .and_then(|endpoint| endpoint.split_once('/').ok_or(PropError::MissingId))
    }

    async fn send<S, D>(
        &self,
        device: &DeviceClient<S>,
        field: &str,
        data: Option<D>,
    ) -> Result<(), AstarteError>
    where
        S: PropertyStore,
        D: Into<AstarteType> + Send,
    {
        let Some(data) = data else {
            return Ok(());
        };

        let endpoint = format!("/{}/{}", self.id, field);

        device.send(Self::INTERFACE, &endpoint, data).await
    }

    pub(crate) async fn store<S>(&self, device: &DeviceClient<S>) -> Result<(), AstarteError>
    where
        S: PropertyStore,
    {
        self.send(device, "repo", self.repo).await?;
        self.send(device, "name", self.name).await?;
        self.send(device, "tag", self.tag).await?;
        self.send(device, "pulled", self.pulled).await?;

        Ok(())
    }

    pub(crate) fn merge(self, other: Self) -> Self {
        Self {
            id: other.id,
            repo: other.repo.or(self.repo),
            name: other.name.or(self.name),
            tag: other.tag.or(self.tag),
            pulled: other.pulled.or(self.pulled),
        }
    }
}

impl<'a> TryFrom<&'a StoredProp> for AvailableImage<'a> {
    type Error = PropError;

    fn try_from(value: &'a StoredProp) -> Result<Self, Self::Error> {
        let (id, field) = Self::parse_endpoint(&value.path)?;

        let mut av_img = AvailableImage::new(id);

        match field {
            "repo" => {
                let AstarteType::String(repo) = &value.value else {
                    return Err(PropError::Type {
                        field: "repo",
                        exp: "string",
                        got: value.value.clone(),
                    });
                };

                av_img.repo.replace(repo);
            }
            "name" => {
                let AstarteType::String(name) = &value.value else {
                    return Err(PropError::Type {
                        field: "name",
                        exp: "string",
                        got: value.value.clone(),
                    });
                };

                av_img.name.replace(name);
            }
            "tag" => {
                let AstarteType::String(tag) = &value.value else {
                    return Err(PropError::Type {
                        field: "tag",
                        exp: "string",
                        got: value.value.clone(),
                    });
                };

                av_img.tag.replace(tag);
            }
            "pulled" => {
                let AstarteType::Boolean(pulled) = value.value else {
                    return Err(PropError::Type {
                        field: "pulled",
                        exp: "string",
                        got: value.value.clone(),
                    });
                };

                av_img.pulled.replace(pulled);
            }
            _ => warn!(
                "unrecognized field for stored property for interface {}/{}:{}",
                value.interface, value.path, value.interface_major
            ),
        }

        Ok(av_img)
    }
}

impl<'a> TryFrom<AvailableImage<'a>> for Image<String> {
    type Error = PropError;

    fn try_from(value: AvailableImage) -> Result<Self, Self::Error> {
        let name = value.name.ok_or(PropError::MissingField {
            field: "name",
            into: "Image",
        })?;
        let tag = value.tag.ok_or(PropError::MissingField {
            field: "tag",
            into: "Image",
        })?;
        let repo = value.repo.ok_or(PropError::MissingField {
            field: "repo",
            into: "Image",
        })?;

        Ok(Self::with_repo(
            name.to_string(),
            tag.to_string(),
            repo.to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_extract_id() {
        let endpoint = "/id/name";

        let (id, field) = AvailableImage::parse_endpoint(endpoint).unwrap();

        assert_eq!(id, "id");
        assert_eq!(field, "name");
    }
}

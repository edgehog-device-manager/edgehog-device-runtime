// This file is part of Edgehog.
//
// Copyright 2023 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Handles Docker request from Astarte.

use astarte_device_sdk::{event::FromEventError, from_event, DeviceEvent, FromEvent};
use itertools::Itertools;

use crate::volume::Volume;

/// Error from handling the Astarte request.
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum ReqError {
    /// couldn't parse option, expected key=value but got {0}
    Option(String),
}

/// Create request from Astarte.
#[derive(Debug, Clone)]
pub enum CreateRequests {
    /// Request to pull a (`Image`)[crate::image::Image].
    Image(CreateImage),
    /// Request to create a Volume
    Volume(CreateVolume),
}

impl FromEvent for CreateRequests {
    type Err = FromEventError;

    fn from_event(value: DeviceEvent) -> Result<Self, Self::Err> {
        match value.interface.as_str() {
            "io.edgehog.devicemanager.apps.CreateImageRequest" => {
                CreateImage::from_event(value).map(CreateRequests::Image)
            }
            "io.edgehog.devicemanager.apps.CreateVolumeRequest" => {
                CreateVolume::from_event(value).map(CreateRequests::Volume)
            }
            _ => Err(FromEventError::Interface(value.interface.clone())),
        }
    }
}

/// Request to pull a Docker Image.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateImageRequest",
    path = "/image"
)]
pub struct CreateImage {
    pub(crate) id: String,
    pub(crate) repo: String,
    pub(crate) name: String,
    pub(crate) tag: String,
}

/// Request to pull a Docker Image.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateVolumeRequest",
    path = "/volume"
)]
pub struct CreateVolume {
    pub(crate) id: String,
    pub(crate) driver: String,
    pub(crate) options: Vec<String>,
}

impl TryFrom<CreateVolume> for Volume<String> {
    type Error = ReqError;

    fn try_from(value: CreateVolume) -> Result<Self, Self::Error> {
        let options = value
            .options
            .into_iter()
            .map(|option| {
                option
                    .split_once('=')
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .ok_or(ReqError::Option(option))
            })
            .try_collect()?;

        Ok(Volume::with_options(value.id, value.driver, options))
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::{types::AstarteType, Value};

    use super::*;

    #[test]
    fn create_image_request() {
        let fields = [
            ("id", "id"),
            ("repo", "repo"),
            ("name", "name"),
            ("tag", "tag"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.into()))
        .collect();
        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateImageRequest".to_string(),
            path: "/image".to_string(),
            data: Value::Object(fields),
        };

        let create_image = CreateImage::from_event(event);

        let expect = CreateImage {
            id: "id".to_string(),
            repo: "repo".to_string(),
            name: "name".to_string(),
            tag: "tag".to_string(),
        };

        assert_eq!(create_image.unwrap(), expect);
    }

    #[test]
    fn create_volume() {
        let fields = [
            ("id", AstarteType::String("id".to_string())),
            ("driver", AstarteType::String("local".to_string())),
            (
                "options",
                AstarteType::StringArray(vec!["foo=bar".to_string()]),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateVolumeRequest".to_string(),
            path: "/volume".to_string(),
            data: Value::Object(fields),
        };

        let create_image = CreateVolume::from_event(event);

        let expect = CreateVolume {
            id: "id".to_string(),
            driver: "local".to_string(),
            options: vec!["foo=bar".to_string()],
        };

        assert_eq!(create_image.unwrap(), expect);
    }
}

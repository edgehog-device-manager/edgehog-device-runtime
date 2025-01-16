// This file is part of Edgehog.
//
// Copyright 2024 SECO Mind Srl
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

//! Create image request

use astarte_device_sdk::FromEvent;

use crate::image::Image;

use super::ReqUuid;

/// Request to pull a Docker Image.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateImageRequest",
    path = "/image",
    rename_all = "camelCase"
)]
pub struct CreateImage {
    pub(crate) id: ReqUuid,
    pub(crate) reference: String,
    pub(crate) registry_auth: String,
}

impl From<CreateImage> for Image<String> {
    fn from(value: CreateImage) -> Self {
        let registry_auth = if value.registry_auth.is_empty() {
            None
        } else {
            Some(value.registry_auth)
        };

        Image {
            id: None,
            reference: value.reference,
            registry_auth,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fmt::Display;

    use astarte_device_sdk::{DeviceEvent, Value};
    use uuid::Uuid;

    use super::*;

    pub fn mock_image_req(
        id: Uuid,
        reference: impl Into<String>,
        registry_auth: impl Into<String>,
    ) -> CreateImage {
        CreateImage {
            id: ReqUuid(id),
            reference: reference.into(),
            registry_auth: registry_auth.into(),
        }
    }

    pub fn create_image_request_event(
        id: impl Display,
        reference: &str,
        auth: &str,
    ) -> DeviceEvent {
        let fields = [
            ("id", id.to_string()),
            ("reference", reference.to_string()),
            ("registryAuth", auth.to_string()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.into()))
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateImageRequest".to_string(),
            path: "/image".to_string(),
            data: Value::Object(fields),
        }
    }

    #[test]
    fn create_image_request() {
        let id = Uuid::new_v4();
        let event = create_image_request_event(id.to_string(), "reference", "registry_auth");

        let request = CreateImage::from_event(event).unwrap();

        let expect = CreateImage {
            id: ReqUuid(id),
            reference: "reference".to_string(),
            registry_auth: "registry_auth".to_string(),
        };

        assert_eq!(request, expect);
    }

    #[test]
    fn should_convert_to_image() {
        let id = Uuid::new_v4();
        let event = create_image_request_event(id.to_string(), "reference", "registry_auth");

        let request = CreateImage::from_event(event).unwrap();

        let exp = Image {
            id: None,
            reference: "reference",
            registry_auth: Some("registry_auth"),
        };

        let img = Image::from(request);

        assert_eq!(img, exp);
    }
}

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

use super::ReqUuid;

/// Request to pull a Docker Image.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateImageRequest",
    path = "/image",
    rename_all = "camelCase",
    aggregation = "object"
)]
pub struct CreateImage {
    #[mapping(required)]
    pub(crate) id: ReqUuid,
    #[mapping(required)]
    pub(crate) deployment_id: ReqUuid,
    #[mapping(required)]
    pub(crate) reference: String,
    #[mapping(required)]
    pub(crate) registry_auth: String,
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{DeviceEvent, Value};
    use uuid::Uuid;

    use super::*;

    pub(crate) fn create_image_req(deployment_id: Uuid) -> CreateImage {
        CreateImage {
            id: ReqUuid(Uuid::new_v4()),
            deployment_id: ReqUuid(deployment_id),
            reference: "postgres:15".to_string(),
            registry_auth: String::new(),
        }
    }

    pub fn create_image_request_event(image: &CreateImage) -> DeviceEvent {
        let fields = [
            ("id", image.id.to_string()),
            ("deploymentId", image.deployment_id.to_string()),
            ("reference", image.reference.to_string()),
            ("registryAuth", image.registry_auth.to_string()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.into()))
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateImageRequest".to_string(),
            path: "/image".to_string(),
            data: Value::Object {
                data: fields,
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn create_image_request() {
        let deployment_id = Uuid::new_v4();

        let image = create_image_req(deployment_id);
        let event = create_image_request_event(&image);

        let request = CreateImage::from_event(event).unwrap();

        assert_eq!(request, image);
    }
}

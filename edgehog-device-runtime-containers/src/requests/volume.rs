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

/// Request to pull a Docker Volume.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateVolumeRequest",
    path = "/volume",
    rename_all = "camelCase",
    aggregation = "object"
)]
pub struct CreateVolume {
    #[mapping(required)]
    pub(crate) id: ReqUuid,
    #[mapping(required)]
    pub(crate) deployment_id: ReqUuid,
    #[mapping(required)]
    pub(crate) driver: String,
    #[mapping(required)]
    pub(crate) options: Vec<String>,
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{AstarteData, DeviceEvent, Value};
    use uuid::Uuid;

    use super::*;

    pub(crate) fn create_volume_req(deployment_id: Uuid) -> CreateVolume {
        let volume_id = ReqUuid(Uuid::new_v4());
        CreateVolume {
            id: volume_id,
            deployment_id: ReqUuid(deployment_id),
            driver: "local".to_string(),
            options: [
                "device=tmpfs",
                "o=size=100m,uid=1000",
                "type=tmpfs",
                "empty=",
            ]
            .map(str::to_string)
            .to_vec(),
        }
    }

    pub(crate) fn create_volume_request_event(volume: &CreateVolume) -> DeviceEvent {
        let fields = [
            ("id".to_string(), AstarteData::String(volume.id.to_string())),
            (
                "deploymentId".to_string(),
                AstarteData::String(volume.deployment_id.to_string()),
            ),
            (
                "driver".to_string(),
                AstarteData::String(volume.driver.to_string()),
            ),
            (
                "options".to_string(),
                AstarteData::StringArray(volume.options.clone()),
            ),
        ]
        .into_iter()
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateVolumeRequest".to_string(),
            path: "/volume".to_string(),
            data: Value::Object {
                data: fields,
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn create_volume_request() {
        let deployment_id = Uuid::new_v4();
        let expect = create_volume_req(deployment_id);
        let event = create_volume_request_event(&expect);

        let request = CreateVolume::from_event(event).unwrap();

        assert_eq!(request, expect);
    }
}

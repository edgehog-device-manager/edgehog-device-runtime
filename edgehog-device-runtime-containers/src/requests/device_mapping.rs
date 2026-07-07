// This file is part of Edgehog.
//
// Copyright 2024 - 2025 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Create network request

use astarte_device_sdk::FromEvent;

use super::{OptString, ReqUuid};

/// Request to pull a Docker Network.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateDeviceMappingRequest",
    path = "/deviceMapping",
    rename_all = "camelCase",
    aggregation = "object"
)]
pub struct CreateDeviceMapping {
    #[mapping(required)]
    pub(crate) id: ReqUuid,
    #[mapping(required)]
    pub(crate) deployment_id: ReqUuid,
    #[mapping(required)]
    pub(crate) path_on_host: String,
    #[mapping(required)]
    pub(crate) path_in_container: String,
    #[mapping(required)]
    pub(crate) c_group_permissions: OptString,
}

#[cfg(test)]
pub(crate) mod tests {

    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{AstarteData, DeviceEvent, Value};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    pub(crate) fn create_device_mapping_req(deployment_id: Uuid) -> CreateDeviceMapping {
        CreateDeviceMapping {
            id: ReqUuid(Uuid::new_v4()),
            deployment_id: ReqUuid(deployment_id),
            path_on_host: "/dev/tty12".to_string(),
            path_in_container: "dev/tty12".to_string(),
            c_group_permissions: OptString::from("msv".to_string()),
        }
    }

    pub fn create_device_mapping_request_event(req: &CreateDeviceMapping) -> DeviceEvent {
        let fields = [
            ("id", AstarteData::String(req.id.to_string())),
            (
                "deploymentId",
                AstarteData::String(req.deployment_id.to_string()),
            ),
            ("pathOnHost", AstarteData::String(req.path_on_host.clone())),
            (
                "pathInContainer",
                AstarteData::String(req.path_in_container.clone()),
            ),
            (
                "cGroupPermissions",
                AstarteData::String(req.c_group_permissions.0.clone().unwrap_or_default()),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateDeviceMappingRequest".to_string(),
            path: "/deviceMapping".to_string(),
            data: Value::Object {
                data: fields,
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn should_create_device_mapping() {
        let deployment_id = Uuid::new_v4();
        let expected = create_device_mapping_req(deployment_id);
        let event = create_device_mapping_request_event(&expected);

        let request = CreateDeviceMapping::from_event(event).unwrap();

        assert_eq!(request, expected);
    }
}

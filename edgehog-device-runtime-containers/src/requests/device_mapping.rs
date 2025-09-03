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
    pub(crate) id: ReqUuid,
    pub(crate) deployment_id: ReqUuid,
    pub(crate) path_on_host: String,
    pub(crate) path_in_container: String,
    pub(crate) c_group_permissions: OptString,
}

#[cfg(test)]
pub(crate) mod tests {

    use std::fmt::Display;

    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{AstarteData, DeviceEvent, Value};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    pub fn create_device_mapping_request_event(
        id: impl Display,
        deployment_id: impl Display,
        host: &str,
        container: &str,
    ) -> DeviceEvent {
        let fields = [
            ("id", AstarteData::String(id.to_string())),
            (
                "deploymentId",
                AstarteData::String(deployment_id.to_string()),
            ),
            ("pathOnHost", AstarteData::from(host)),
            ("pathInContainer", AstarteData::from(container)),
            ("cGroupPermissions", AstarteData::from("msv")),
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
    fn create_device_mapping() {
        let id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let event =
            create_device_mapping_request_event(id, deployment_id, "/dev/tty12", "/dev/tty12");

        let request = CreateDeviceMapping::from_event(event).unwrap();

        let expect = CreateDeviceMapping {
            id: ReqUuid(id),
            deployment_id: ReqUuid(deployment_id),
            path_on_host: "/dev/tty12".to_string(),
            path_in_container: "/dev/tty12".to_string(),
            c_group_permissions: OptString(Some("msv".to_string())),
        };

        assert_eq!(request, expect);
    }
}

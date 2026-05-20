// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
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

//! Device Request sent from Astarte

use astarte_device_sdk::{FromEvent, IntoAstarteObject};

use super::{OptString, ReqUuid};

/// Request to pull a Docker Network.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord, IntoAstarteObject)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateDeviceRequest",
    path = "/deviceRequest",
    rename_all = "camelCase",
    aggregation = "object"
)]
#[astarte_object(rename_all = "camelCase")]
pub struct CreateDeviceRequest {
    pub(crate) id: ReqUuid,
    pub(crate) deployment_id: ReqUuid,
    pub(crate) driver: OptString,
    pub(crate) count: i64,
    pub(crate) device_ids: Vec<String>,
    pub(crate) capabilities: Vec<String>,
    pub(crate) option_keys: Vec<String>,
    pub(crate) option_values: Vec<String>,
}

#[cfg(test)]
pub(crate) mod tests {

    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{DeviceEvent, Value};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    pub fn create_device_request(id: Uuid, deployment_id: Uuid) -> CreateDeviceRequest {
        CreateDeviceRequest {
            id: id.into(),
            deployment_id: deployment_id.into(),
            driver: "nvidia".into(),
            count: -1,
            device_ids: ["0", "1", "GPU-fef8089b-4820-abfc-e83e-94318197576e"]
                .map(str::to_string)
                .to_vec(),
            capabilities: vec![r#"["gpu","nvidia","compute"]"#.to_string()],
            option_keys: ["property1", "property2"].map(str::to_string).to_vec(),
            option_values: ["string", "string"].map(str::to_string).to_vec(),
        }
    }

    pub fn create_device_request_event(id: Uuid, deployment_id: Uuid) -> DeviceEvent {
        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateDeviceRequest".to_string(),
            path: "/deviceRequest".to_string(),
            data: Value::Object {
                data: AstarteObject::try_from(create_device_request(id, deployment_id)).unwrap(),
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn device_request_from_event() {
        let id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();
        let event = create_device_request_event(id, deployment_id);

        let request = CreateDeviceRequest::from_event(event).unwrap();

        let expect = create_device_request(id, deployment_id);

        assert_eq!(request, expect);
    }
}

// This file is part of Edgehog.
//
// Copyright 2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//
// SPDX-License-Identifier: Apache-2.0

//! Device Request sent from Astarte

use astarte_device_sdk::FromEvent;

use super::ReqUuid;

/// Request to pull a Docker Network.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateDeviceRequest",
    path = "/deviceRequest",
    rename_all = "camelCase",
    aggregation = "object"
)]
pub struct CreateDeviceRequest {
    #[mapping(required)]
    pub(crate) id: ReqUuid,
    #[mapping(required)]
    pub(crate) deployment_id: ReqUuid,
    pub(crate) driver: Option<String>,
    pub(crate) count: Option<i64>,
    pub(crate) device_ids: Option<Vec<String>>,
    pub(crate) capabilities: Option<Vec<String>>,
    pub(crate) option_keys: Option<Vec<String>>,
    pub(crate) option_values: Option<Vec<String>>,
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{AstarteData, DeviceEvent, Value};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    pub fn create_device_request(deployment_id: Uuid) -> CreateDeviceRequest {
        CreateDeviceRequest {
            id: ReqUuid(Uuid::new_v4()),
            deployment_id: deployment_id.into(),
            driver: Some("nvidia".into()),
            count: Some(4),
            device_ids: Some(
                ["0", "1", "GPU-fef8089b-4820-abfc-e83e-94318197576e"]
                    .map(str::to_string)
                    .to_vec(),
            ),
            capabilities: Some(vec![r#"["gpu","nvidia","compute"]"#.to_string()]),
            option_keys: Some(["property1", "property2"].map(str::to_string).to_vec()),
            option_values: Some(["string", "string"].map(str::to_string).to_vec()),
        }
    }

    pub fn create_device_request_event(value: &CreateDeviceRequest) -> DeviceEvent {
        let data = [
            ("id", AstarteData::from(value.id.to_string())),
            (
                "deploymentId",
                AstarteData::from(value.deployment_id.to_string()),
            ),
            ("driver", AstarteData::from(value.driver.clone().unwrap())),
            ("count", AstarteData::from(value.count.unwrap())),
            (
                "deviceIds",
                AstarteData::from(value.device_ids.clone().unwrap()),
            ),
            (
                "capabilities",
                AstarteData::from(value.capabilities.clone().unwrap()),
            ),
            (
                "optionKeys",
                AstarteData::from(value.option_keys.clone().unwrap()),
            ),
            (
                "optionValues",
                AstarteData::from(value.option_values.clone().unwrap()),
            ),
        ]
        .map(|(k, v)| (k.to_string(), v));

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateDeviceRequest".to_string(),
            path: "/deviceRequest".to_string(),
            data: Value::Object {
                data: AstarteObject::from_iter(data),
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn device_request_from_event() {
        let deployment_id = Uuid::new_v4();

        let expect = create_device_request(deployment_id);
        let event = create_device_request_event(&expect);

        let request = CreateDeviceRequest::from_event(event).unwrap();

        assert_eq!(request, expect);
    }
}

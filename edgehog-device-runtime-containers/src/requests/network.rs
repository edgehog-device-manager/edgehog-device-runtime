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

use super::ReqUuid;

/// Request to pull a Docker Network.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateNetworkRequest",
    path = "/network",
    rename_all = "camelCase",
    aggregation = "object"
)]
pub struct CreateNetwork {
    #[mapping(required)]
    pub(crate) id: ReqUuid,
    #[mapping(required)]
    pub(crate) deployment_id: ReqUuid,
    #[mapping(required)]
    pub(crate) driver: String,
    #[mapping(required)]
    pub(crate) internal: bool,
    #[mapping(required)]
    pub(crate) enable_ipv6: bool,
    #[mapping(required)]
    pub(crate) options: Vec<String>,
}

#[cfg(test)]
pub(crate) mod tests {

    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{AstarteData, DeviceEvent, Value};
    use uuid::Uuid;

    use super::*;

    pub(crate) fn create_network_req(deployment_id: Uuid) -> CreateNetwork {
        CreateNetwork {
            id: ReqUuid(Uuid::new_v4()),
            deployment_id: ReqUuid(deployment_id),
            driver: "bridge".to_string(),
            internal: true,
            enable_ipv6: false,
            options: vec!["isolate=true".to_string()],
        }
    }

    pub fn create_network_request_event(network: &CreateNetwork) -> DeviceEvent {
        let fields = [
            ("id", AstarteData::String(network.id.to_string())),
            (
                "deploymentId",
                AstarteData::String(network.deployment_id.to_string()),
            ),
            ("driver", AstarteData::String(network.driver.to_string())),
            ("internal", AstarteData::Boolean(network.internal)),
            ("enableIpv6", AstarteData::Boolean(network.enable_ipv6)),
            ("options", AstarteData::StringArray(network.options.clone())),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateNetworkRequest".to_string(),
            path: "/network".to_string(),
            data: Value::Object {
                data: fields,
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn should_create_network_request() {
        let deployment_id = Uuid::new_v4();
        let network = create_network_req(deployment_id);
        let event = create_network_request_event(&network);

        let request = CreateNetwork::from_event(event).unwrap();

        assert_eq!(request, network);
    }
}

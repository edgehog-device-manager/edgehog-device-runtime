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

//! Create network option request

use astarte_device_sdk::FromEvent;

use super::ReqUuid;

/// Request to pull a Docker Network.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateNetworkOptionsRequest",
    path = "/networkOptions",
    rename_all = "camelCase",
    aggregation = "object"
)]
pub struct CreateNetworkOptions {
    #[mapping(required)]
    pub(crate) id: ReqUuid,
    #[mapping(required)]
    pub(crate) deployment_id: ReqUuid,
    #[mapping(required)]
    pub(crate) container_id: ReqUuid,
    #[mapping(required)]
    pub(crate) network_id: ReqUuid,
    pub(crate) ipv4_address: Option<String>,
    pub(crate) ipv6_address: Option<String>,
    pub(crate) link_local_ips: Option<Vec<String>>,
    pub(crate) links: Option<Vec<String>>,
    pub(crate) mac_address: Option<String>,
    pub(crate) aliases: Option<Vec<String>>,
    pub(crate) driver_opt_keys: Option<Vec<String>>,
    pub(crate) driver_opt_values: Option<Vec<String>>,
    pub(crate) gw_priority: Option<i64>,
    pub(crate) gateway: Option<String>,
    pub(crate) ip_address: Option<String>,
    pub(crate) ip_prefix_len: Option<i32>,
    pub(crate) ipv6_gateway: Option<String>,
    pub(crate) global_ipv6_address: Option<String>,
    pub(crate) global_ipv6_prefix_len: Option<i64>,
    pub(crate) dns_names: Option<Vec<String>>,
}

#[cfg(test)]
pub(crate) mod tests {

    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{AstarteData, DeviceEvent, Value};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    pub(crate) fn create_network_options_req(
        deployment_id: Uuid,
        container_id: Uuid,
        network_id: Uuid,
    ) -> CreateNetworkOptions {
        let network_options_id = Uuid::new_v4();
        CreateNetworkOptions {
            id: ReqUuid(network_options_id),
            deployment_id: ReqUuid(deployment_id),
            container_id: ReqUuid(container_id),
            network_id: ReqUuid(network_id),
            ipv4_address: None,
            ipv6_address: None,
            link_local_ips: None,
            links: None,
            mac_address: None,
            aliases: None,
            driver_opt_keys: None,
            driver_opt_values: None,
            gw_priority: None,
            gateway: None,
            ip_address: None,
            ip_prefix_len: None,
            ipv6_gateway: None,
            global_ipv6_address: None,
            global_ipv6_prefix_len: None,
            dns_names: None,
        }
    }

    pub fn create_network_options_request_event(req: &CreateNetworkOptions) -> DeviceEvent {
        let fields = [
            ("id", AstarteData::String(req.id.to_string())),
            (
                "deploymentId",
                AstarteData::String(req.deployment_id.to_string()),
            ),
            (
                "containerId",
                AstarteData::String(req.container_id.to_string()),
            ),
            ("networkId", AstarteData::String(req.network_id.to_string())),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateNetworkOptionsRequest".to_string(),
            path: "/networkOptions".to_string(),
            data: Value::Object {
                data: fields,
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn should_create_network_options() {
        let deployment_id = Uuid::new_v4();
        let container_id = Uuid::new_v4();
        let network_id = Uuid::new_v4();
        let expected = create_network_options_req(deployment_id, container_id, network_id);
        let event = create_network_options_request_event(&expected);

        let request = CreateNetworkOptions::from_event(event).unwrap();

        assert_eq!(request, expected);
    }
}

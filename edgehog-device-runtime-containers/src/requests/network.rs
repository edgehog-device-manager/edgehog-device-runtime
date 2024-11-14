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

use crate::network::Network;

/// Request to pull a Docker Network.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateNetworkRequest",
    path = "/network",
    rename_all = "camelCase"
)]
pub struct CreateNetwork {
    pub(crate) id: String,
    pub(crate) driver: String,
    pub(crate) check_duplicate: bool,
    pub(crate) internal: bool,
    pub(crate) enable_ipv6: bool,
}

impl From<CreateNetwork> for Network<String> {
    fn from(value: CreateNetwork) -> Self {
        Network {
            id: None,
            name: value.id,
            driver: value.driver,
            check_duplicate: value.check_duplicate,
            internal: value.internal,
            enable_ipv6: value.enable_ipv6,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {

    use std::fmt::Display;

    use astarte_device_sdk::{AstarteType, DeviceEvent, Value};

    use super::*;

    pub fn create_network_request_event(id: impl Display, driver: &str) -> DeviceEvent {
        let fields = [
            ("id", AstarteType::String(id.to_string())),
            ("driver", AstarteType::String(driver.to_string())),
            ("checkDuplicate", AstarteType::Boolean(false)),
            ("internal", AstarteType::Boolean(false)),
            ("enableIpv6", AstarteType::Boolean(false)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateNetworkRequest".to_string(),
            path: "/network".to_string(),
            data: Value::Object(fields),
        }
    }

    #[test]
    fn create_network_request() {
        let event = create_network_request_event("id", "driver");

        let request = CreateNetwork::from_event(event).unwrap();

        let expect = CreateNetwork {
            id: "id".to_string(),
            driver: "driver".to_string(),
            check_duplicate: false,
            internal: false,
            enable_ipv6: false,
        };

        assert_eq!(request, expect);
    }
}

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

use super::{parse_kv_map, ReqError, ReqUuid};

/// Request to pull a Docker Network.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateNetworkRequest",
    path = "/network",
    rename_all = "camelCase"
)]
pub struct CreateNetwork {
    pub(crate) id: ReqUuid,
    pub(crate) driver: String,
    pub(crate) internal: bool,
    pub(crate) enable_ipv6: bool,
    pub(crate) options: Vec<String>,
}

impl TryFrom<CreateNetwork> for Network<String> {
    type Error = ReqError;

    fn try_from(value: CreateNetwork) -> Result<Self, Self::Error> {
        let driver_opts = parse_kv_map(&value.options)?;

        Ok(Network {
            id: None,
            name: value.id.to_string(),
            driver: value.driver,
            internal: value.internal,
            enable_ipv6: value.enable_ipv6,
            driver_opts,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {

    use std::fmt::Display;

    use astarte_device_sdk::{AstarteType, DeviceEvent, Value};
    use itertools::Itertools;
    use uuid::Uuid;

    use super::*;

    pub fn create_network_request_event(
        id: impl Display,
        driver: &str,
        options: &[&str],
    ) -> DeviceEvent {
        let options = options.iter().map(|s| s.to_string()).collect_vec();

        let fields = [
            ("id", AstarteType::String(id.to_string())),
            ("driver", AstarteType::String(driver.to_string())),
            ("checkDuplicate", AstarteType::Boolean(false)),
            ("internal", AstarteType::Boolean(false)),
            ("enableIpv6", AstarteType::Boolean(false)),
            ("options", AstarteType::StringArray(options)),
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
        let id = Uuid::new_v4();
        let event = create_network_request_event(id.to_string(), "driver", &["foo=bar", "some="]);

        let request = CreateNetwork::from_event(event).unwrap();

        let expect = CreateNetwork {
            id: ReqUuid(id),
            driver: "driver".to_string(),
            internal: false,
            enable_ipv6: false,
            options: ["foo=bar", "some="].map(str::to_string).to_vec(),
        };

        assert_eq!(request, expect);
    }
}

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

/// Request to pull a Docker Deployment.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateDeploymentRequest",
    path = "/deployment",
    rename_all = "camelCase"
)]
pub(crate) struct CreateDeployment {
    pub(crate) id: String,
    pub(crate) containers: Vec<String>,
}

#[cfg(test)]
pub mod tests {

    use astarte_device_sdk::{AstarteType, DeviceEvent, Value};

    use super::*;

    pub fn create_deployment_request_event(id: &str, containers: &[&str]) -> DeviceEvent {
        let fields = [
            ("id", AstarteType::String(id.to_string())),
            (
                "containers",
                AstarteType::StringArray(containers.iter().map(|s| s.to_string()).collect()),
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateDeploymentRequest".to_string(),
            path: "/deployment".to_string(),
            data: Value::Object(fields),
        }
    }

    #[test]
    fn create_deployment_request() {
        let event = create_deployment_request_event("id", &["container_id"]);

        let request = CreateDeployment::from_event(event).unwrap();

        let expect = CreateDeployment {
            id: "id".to_string(),
            containers: vec!["container_id".to_string()],
        };

        assert_eq!(request, expect);
    }
}

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

//! Env file creation request

use astarte_device_sdk::FromEvent;

use super::ReqUuid;

/// Request to pull a Docker Network.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateEnvFileRequest",
    path = "/env",
    rename_all = "camelCase",
    aggregation = "object"
)]
pub struct CreateEnvFile {
    #[mapping(required)]
    pub(crate) id: ReqUuid,
    #[mapping(required)]
    pub(crate) deployment_id: ReqUuid,
    #[mapping(required)]
    pub(crate) target_id: String,
    #[mapping(required)]
    pub(crate) target_type: String,
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{AstarteData, DeviceEvent, Value};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    pub(crate) fn create_env_file_req(deployment_id: Uuid) -> CreateEnvFile {
        CreateEnvFile {
            id: ReqUuid(Uuid::new_v4()),
            deployment_id: ReqUuid(deployment_id),
            target_id: Uuid::new_v4().to_string(),
            target_type: "storage".to_string(),
        }
    }

    pub(crate) fn create_env_file_event(req: &CreateEnvFile) -> DeviceEvent {
        let fields: AstarteObject = [
            ("id", AstarteData::String(req.id.to_string())),
            (
                "deploymentId",
                AstarteData::String(req.deployment_id.to_string()),
            ),
            ("targetId", AstarteData::String(req.target_id.clone())),
            ("targetType", AstarteData::String(req.target_type.clone())),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateEnvFileRequest".to_string(),
            path: "/env".to_string(),
            data: Value::Object {
                data: fields,
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn should_create_env_file() {
        let deployment_id = Uuid::new_v4();
        let expected = create_env_file_req(deployment_id);
        let event = create_env_file_event(&expected);

        let request = CreateEnvFile::from_event(event).unwrap();

        assert_eq!(request, expected);
    }
}

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

//! File bind creation request

use astarte_device_sdk::FromEvent;

use super::{OptString, ReqUuid};

/// Request to pull a Docker Network.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateFileBindRequest",
    path = "/bind",
    rename_all = "camelCase",
    aggregation = "object"
)]
pub struct CreateFileBind {
    #[mapping(required)]
    pub(crate) id: ReqUuid,
    #[mapping(required)]
    pub(crate) deployment_id: ReqUuid,
    #[mapping(required)]
    pub(crate) target_id: String,
    #[mapping(required)]
    pub(crate) target_type: String,
    #[mapping(required)]
    pub(crate) mountpoint: String,
    pub(crate) options: Option<OptString>,
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{AstarteData, DeviceEvent, Value};
    use pretty_assertions::assert_eq;
    use uuid::Uuid;

    use super::*;

    pub(crate) fn create_file_bind_req(deployment_id: Uuid) -> CreateFileBind {
        CreateFileBind {
            id: ReqUuid(Uuid::new_v4()),
            deployment_id: ReqUuid(deployment_id),
            target_id: Uuid::new_v4().to_string(),
            target_type: "storage".to_string(),
            mountpoint: "/data/config.toml".to_string(),
            options: Some(OptString::from("z")),
        }
    }

    pub(crate) fn create_file_bind_event(req: &CreateFileBind) -> DeviceEvent {
        let mut fields: AstarteObject = [
            ("id", AstarteData::String(req.id.to_string())),
            (
                "deploymentId",
                AstarteData::String(req.deployment_id.to_string()),
            ),
            ("targetId", AstarteData::String(req.target_id.clone())),
            ("targetType", AstarteData::String(req.target_type.clone())),
            ("mountpoint", AstarteData::String(req.mountpoint.clone())),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

        if let Some(options) = req.options.clone().and_then(Option::<String>::from) {
            fields.insert("options".to_string(), AstarteData::String(options));
        }

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateFileBindRequest".to_string(),
            path: "/bind".to_string(),
            data: Value::Object {
                data: fields,
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn should_create_file_bind() {
        let deployment_id = Uuid::new_v4();
        let expected = create_file_bind_req(deployment_id);
        let event = create_file_bind_event(&expected);

        let request = CreateFileBind::from_event(event).unwrap();

        assert_eq!(request, expected);
    }
}

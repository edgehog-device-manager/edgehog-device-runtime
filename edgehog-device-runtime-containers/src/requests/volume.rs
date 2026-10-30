// This file is part of Edgehog.
//
// Copyright 2024, 2026 SECO Mind Srl
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

//! Create image request

use astarte_device_sdk::{
    AstarteData, FromEvent, astarte_device_error, event::FromEventError, types::TypeError,
};
use tracing::error;
use uuid::Uuid;

use super::ReqUuid;

/// Request to pull a Docker Volume.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateVolumeRequest",
    path = "/volume",
    rename_all = "camelCase",
    aggregation = "object"
)]
pub struct CreateVolume {
    #[mapping(required)]
    pub(crate) id: ReqUuid,
    #[mapping(required)]
    pub(crate) deployment_id: ReqUuid,
    #[mapping(required)]
    pub(crate) driver: String,
    pub(crate) options: Option<Vec<String>>,
    pub(crate) reclaim_policy: Option<String>,
}

/// Command submitted to a specific volume.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct VolumeCommand {
    pub(crate) volume_id: Uuid,
    pub(crate) command: CommandValue,
}

impl FromEvent for VolumeCommand {
    type Err = astarte_device_error::Error<FromEventError>;

    fn from_event(event: astarte_device_sdk::DeviceEvent) -> Result<Self, Self::Err> {
        let volume_id = event
            .path
            .strip_prefix('/')
            .and_then(|path| path.strip_suffix("/command"))
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| {
                astarte_device_error::Error::with(
                    FromEventError::Interface(astarte_device_sdk::error::InterfaceError::Path),
                    "not a valid volume command path",
                )
            })?;

        let event = VolumeCommandEvent::from_event(event)?;

        match event {
            VolumeCommandEvent::Command(command) => Ok(Self { volume_id, command }),
        }
    }
}

#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.VolumeCommand",
    aggregation = "individual"
)]
enum VolumeCommandEvent {
    #[mapping(endpoint = "/%{volume_id}/command")]
    Command(CommandValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum CommandValue {
    /// delete a retained container
    Delete,
}

impl TryFrom<AstarteData> for CommandValue {
    type Error = astarte_device_error::Error<TypeError>;

    fn try_from(value: AstarteData) -> Result<Self, Self::Error> {
        let value = String::try_from(value)?;

        match value.as_str() {
            "Delete" => Ok(Self::Delete),
            _ => {
                error!("unrecognize VolumeCommand command value {value}");

                Err(astarte_device_error::Error::with(
                    TypeError::Conversion,
                    "unrecognize VolumeCommand command value",
                )
                .set_ctx(value))
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use astarte_device_sdk::chrono::Utc;
    use astarte_device_sdk::{AstarteData, DeviceEvent, Value};
    use uuid::Uuid;

    use super::*;

    pub(crate) fn create_volume_req(deployment_id: Uuid) -> CreateVolume {
        let volume_id = ReqUuid(Uuid::new_v4());
        CreateVolume {
            id: volume_id,
            deployment_id: ReqUuid(deployment_id),
            driver: "local".to_string(),
            options: Some(
                [
                    "device=tmpfs",
                    "o=size=100m,uid=1000",
                    "type=tmpfs",
                    "empty=",
                ]
                .map(str::to_string)
                .to_vec(),
            ),
            reclaim_policy: Some("Delete".to_string()),
        }
    }

    pub(crate) fn create_volume_request_event(volume: &CreateVolume) -> DeviceEvent {
        let fields = [
            ("id".to_string(), AstarteData::String(volume.id.to_string())),
            (
                "deploymentId".to_string(),
                AstarteData::String(volume.deployment_id.to_string()),
            ),
            (
                "driver".to_string(),
                AstarteData::String(volume.driver.to_string()),
            ),
            (
                "options".to_string(),
                AstarteData::StringArray(volume.options.clone().unwrap_or_default()),
            ),
            (
                "reclaimPolicy".to_string(),
                AstarteData::String(volume.reclaim_policy.clone().unwrap_or_default()),
            ),
        ]
        .into_iter()
        .collect();

        DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.CreateVolumeRequest".to_string(),
            path: "/volume".to_string(),
            data: Value::Object {
                data: fields,
                timestamp: Utc::now(),
            },
        }
    }

    #[test]
    fn create_volume_request() {
        let deployment_id = Uuid::new_v4();
        let expect = create_volume_req(deployment_id);
        let event = create_volume_request_event(&expect);

        let request = CreateVolume::from_event(event).unwrap();

        assert_eq!(request, expect);
    }
}

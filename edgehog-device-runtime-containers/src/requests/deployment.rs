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

use astarte_device_sdk::{event::FromEventError, types::TypeError, AstarteType, FromEvent};
use tracing::error;
use uuid::Uuid;

/// Request to pull a Docker Deployment.
#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.CreateDeploymentRequest",
    path = "/deployment",
    rename_all = "camelCase"
)]
pub struct CreateDeployment {
    pub(crate) id: String,
    pub(crate) containers: Vec<String>,
}

/// Command for a previously received deployment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeploymentCommand {
    pub(crate) id: Uuid,
    pub(crate) command: CommandValue,
}

impl FromEvent for DeploymentCommand {
    type Err = FromEventError;

    fn from_event(event: astarte_device_sdk::DeviceEvent) -> Result<Self, Self::Err> {
        let id = event
            .path
            .strip_prefix('/')
            .and_then(|path| path.strip_suffix("/command"))
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| FromEventError::Path {
                interface: "io.edgehog.devicemanager.apps.DeploymentCommand",
                base_path: event.path.clone(),
            })?;

        let event = DeploymentCommandEvent::from_event(event)?;

        match event {
            DeploymentCommandEvent::Command(command) => Ok(Self { id, command }),
        }
    }
}

#[derive(Debug, Clone, FromEvent)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.DeploymentCommand",
    aggregation = "individual"
)]
enum DeploymentCommandEvent {
    #[mapping(endpoint = "/%{deployment_id}/command")]
    Command(CommandValue),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandValue {
    Start,
    Stop,
    Delete,
}

impl TryFrom<AstarteType> for CommandValue {
    type Error = TypeError;

    fn try_from(value: AstarteType) -> Result<Self, Self::Error> {
        let value = String::try_from(value)?;

        match value.as_str() {
            "Start" => Ok(Self::Start),
            "Stop" => Ok(Self::Stop),
            "Delete" => Ok(Self::Delete),
            _ => {
                error!("unrecognize DeploymentCommand command value {value}");

                Err(TypeError::Conversion)
            }
        }
    }
}

/// Request to update between two deployments.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeploymentUpdate {
    pub(crate) from: Uuid,
    pub(crate) to: Uuid,
}

impl FromEvent for DeploymentUpdate {
    type Err = FromEventError;

    fn from_event(event: astarte_device_sdk::DeviceEvent) -> Result<Self, Self::Err> {
        let event = DeploymentUpdateEvent::from_event(event)?;

        let from = Uuid::parse_str(&event.from).map_err(|err| {
            error!(error = %eyre::Report::new(err), from = event.from, "invalid deployment update 'from' uuid");
            FromEventError::Conversion(TypeError::Conversion)
        })?;

        let to = Uuid::parse_str(&event.to).map_err(|err| {
            error!(error = %eyre::Report::new(err), to = event.to, "invalid deployment update 'to' uuid");
            FromEventError::Conversion(TypeError::Conversion)
        })?;

        Ok(Self { from, to })
    }
}

#[derive(Debug, Clone, FromEvent, PartialEq, Eq, PartialOrd, Ord)]
#[from_event(
    interface = "io.edgehog.devicemanager.apps.DeploymentUpdate",
    path = "/deployment",
    rename_all = "camelCase"
)]
struct DeploymentUpdateEvent {
    from: String,
    to: String,
}

#[cfg(test)]
pub(crate) mod tests {

    use std::collections::HashMap;

    use astarte_device_sdk::{AstarteType, DeviceEvent, Value};
    use pretty_assertions::assert_eq;

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

    #[test]
    fn deployment_command() {
        let id = Uuid::new_v4();

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.DeploymentCommand".to_string(),
            path: format!("/{id}/command"),
            data: Value::Individual("Start".into()),
        };

        let exp = DeploymentCommand {
            id,
            command: CommandValue::Start,
        };

        let cmd = DeploymentCommand::from_event(event).unwrap();
        assert_eq!(cmd, exp);

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.DeploymentCommand".to_string(),
            path: format!("/{id}/command"),
            data: Value::Individual("Stop".into()),
        };

        let exp = DeploymentCommand {
            id,
            command: CommandValue::Stop,
        };

        let cmd = DeploymentCommand::from_event(event).unwrap();
        assert_eq!(cmd, exp);
    }

    #[test]
    fn deployment_update() {
        let from = Uuid::new_v4();
        let to = Uuid::new_v4();

        let event = DeviceEvent {
            interface: "io.edgehog.devicemanager.apps.DeploymentUpdate".to_string(),
            path: "/deployment".to_string(),
            data: Value::Object(HashMap::from([
                ("from".to_string(), AstarteType::String(from.to_string())),
                ("to".to_string(), AstarteType::String(to.to_string())),
            ])),
        };

        let exp = DeploymentUpdate { from, to };

        let cmd = DeploymentUpdate::from_event(event).unwrap();
        assert_eq!(cmd, exp);
    }
}

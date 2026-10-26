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

//! Events sent from the device to Astarte

use std::fmt::Display;

use astarte_device_sdk::{AstarteData, IntoAstarteObject};

use super::DatastreamEvent;

/// Deployment status event
#[derive(Debug, Clone, IntoAstarteObject)]
#[astarte_object(rename_all = "camelCase")]
pub(crate) struct DeploymentEvent {
    pub(crate) status: DeploymentEventStatus,
    pub(crate) message: String,
    pub(crate) add_info: Vec<String>,
}

impl DatastreamEvent for DeploymentEvent {
    const INTERFACE: &str = "io.edgehog.devicemanager.apps.DeploymentEvent";

    type Status = DeploymentEventStatus;

    fn new(status: Self::Status, message: impl Into<String>, add_info: Vec<String>) -> Self {
        Self {
            status,
            message: message.into(),
            add_info,
        }
    }
}

impl Display for DeploymentEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "deployment event ({}) with message {}",
            self.status, self.message
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeploymentEventStatus {
    Starting,
    Started,
    Stopping,
    Stopped,
    Updating,
    Deleting,
    Error,
}

impl Display for DeploymentEventStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeploymentEventStatus::Starting => write!(f, "Starting"),
            DeploymentEventStatus::Started => write!(f, "Started"),
            DeploymentEventStatus::Stopping => write!(f, "Stopping"),
            DeploymentEventStatus::Stopped => write!(f, "Stopped"),
            DeploymentEventStatus::Updating => write!(f, "Updating"),
            DeploymentEventStatus::Deleting => write!(f, "Deleting"),
            DeploymentEventStatus::Error => write!(f, "Error"),
        }
    }
}

impl From<DeploymentEventStatus> for AstarteData {
    fn from(value: DeploymentEventStatus) -> Self {
        AstarteData::String(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::pairing::api::PairingApi;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk_mock::MockDeviceClient;
    use astarte_device_sdk_mock::mockall::Sequence;
    use mockall::predicate;
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn should_send_event() {
        let mut client = MockDeviceClient::<Mqtt<SqliteStore, PairingApi>>::new();
        let mut seq = Sequence::new();

        let id = Uuid::new_v4();

        let exp_p = format!("/{id}");
        client
            .expect_send_object_with_timestamp()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.DeploymentEvent"),
                predicate::eq(exp_p),
                predicate::eq(AstarteObject::from_iter([
                    ("status".to_string(), AstarteData::from("Starting")),
                    ("message".to_string(), AstarteData::from("")),
                    (
                        "addInfo".to_string(),
                        AstarteData::from(Vec::<String>::new()),
                    ),
                ])),
                predicate::always(),
            )
            .returning(|_, _, _, _| Ok(()));

        let event = DeploymentEvent::with_message(DeploymentEventStatus::Starting, "");

        event.send(&id, &mut client).await;
    }
}

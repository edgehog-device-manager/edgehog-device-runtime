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

//! Events sent from the device to Astarte

use std::fmt::Display;

use astarte_device_sdk::{AstarteAggregate, AstarteType};
use tracing::error;
use uuid::Uuid;

use crate::properties::Client;

const INTERFACE: &str = "io.edgehog.devicemanager.apps.DeploymentEvent";

/// Deployment status event
#[derive(Debug, Clone, AstarteAggregate)]
pub(crate) struct DeploymentEvent {
    status: EventStatus,
    message: String,
}

impl DeploymentEvent {
    pub(crate) fn new(status: EventStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub(crate) async fn send<D>(self, id: &Uuid, client: &D)
    where
        D: Client + Sync + 'static,
    {
        let res = client
            .send_object(INTERFACE, &format!("/{id}"), self.clone())
            .await;

        if let Err(err) = res {
            error!(error = %err, "couldn't send {self}, with id {id}")
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
pub(crate) enum EventStatus {
    Starting,
    Stopping,
    Updating,
    Deleting,
    Error,
}

impl Display for EventStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventStatus::Starting => write!(f, "Starting"),
            EventStatus::Stopping => write!(f, "Stopping"),
            EventStatus::Updating => write!(f, "Updating"),
            EventStatus::Deleting => write!(f, "Deleting"),
            EventStatus::Error => write!(f, "Error"),
        }
    }
}

impl From<EventStatus> for AstarteType {
    fn from(value: EventStatus) -> Self {
        AstarteType::String(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk_mock::mockall::Sequence;
    use astarte_device_sdk_mock::MockDeviceClient;

    use super::*;

    #[tokio::test]
    async fn should_send_event() {
        let mut client = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        let id = Uuid::new_v4();

        let exp_p = format!("/{id}");
        client
            .expect_send_object::<DeploymentEvent>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, data| {
                interface == "io.edgehog.devicemanager.apps.DeploymentEvent"
                    && path == exp_p
                    && data.status == EventStatus::Starting
                    && data.message.is_empty()
            })
            .returning(|_, _, _| Ok(()));

        let event = DeploymentEvent::new(EventStatus::Starting, "");

        event.send(&id, &client).await;
    }
}

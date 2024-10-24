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
use uuid::Uuid;

use crate::properties::Client;

/// couldn't send {event}, with id {id}
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub struct EventError {
    id: Uuid,
    event: DeploymentEvent,
    #[source]
    source: astarte_device_sdk::Error,
}

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

    pub(crate) async fn send<D>(self, id: &Uuid, client: &D) -> Result<(), EventError>
    where
        D: Client + Sync + 'static,
    {
        client
            .send_object(INTERFACE, &format!("/{id}"), self.clone())
            .await
            .map_err(|source| EventError {
                id: *id,
                event: self,
                source,
            })?;

        Ok(())
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

#[derive(Debug, Clone, Copy)]
pub(crate) enum EventStatus {
    Starting,
    _Stopping,
    Error,
}

impl Display for EventStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventStatus::Starting => write!(f, "Starting"),
            EventStatus::_Stopping => write!(f, "Stopping"),
            EventStatus::Error => write!(f, "Error"),
        }
    }
}

impl From<EventStatus> for AstarteType {
    fn from(value: EventStatus) -> Self {
        AstarteType::String(value.to_string())
    }
}

// This file is part of Edgehog.
//
// Copyright 2025 SECO Mind Srl
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

//! Handles incoming events from Astarte.
//!
//! When an event is received it will be handled by storing it to the [`StateStore`] and then the
//! [Service](super::Service) will be notified.

use tokio::sync::mpsc;
use tracing::{error, instrument};

use crate::{
    requests::{
        deployment::{DeploymentCommand, DeploymentUpdate},
        ContainerRequest,
    },
    store::StateStore,
};

/// Error returned by the [`ServiceHandle`]
#[derive(Debug, Clone, thiserror::Error, displaydoc::Display)]
pub enum EventError {
    /// couldn't handle the event since the service exited.
    Disconnected,
}

use super::{Id, ResourceType};

/// Handle to the [container service](super::Service).
#[derive(Debug)]
pub struct ServiceHandle {
    /// Queue of events received from Astarte.
    events: mpsc::UnboundedSender<AstarteEvent>,
    _store: StateStore,
}

impl ServiceHandle {
    /// Create the handle from the [channel](mpsc::UnboundedSender) shared with the [`Service`](super::Service).
    pub(crate) fn new(events: mpsc::UnboundedSender<AstarteEvent>, store: StateStore) -> Self {
        Self {
            events,
            _store: store,
        }
    }

    /// Handles an event from the image.
    #[instrument(skip_all)]
    pub async fn on_event(&self, event: ContainerRequest) -> Result<(), EventError> {
        // FIXME: store the resource into the database
        let event = AstarteEvent::from(&event);

        self.events.send(event).map_err(|_err| {
            error!("the container service disconnected");

            EventError::Disconnected
        })?;

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AstarteEvent {
    Resource(Id),
    DeploymentCmd(DeploymentCommand),
    DeploymentUpdate(DeploymentUpdate),
}

impl From<&ContainerRequest> for AstarteEvent {
    fn from(value: &ContainerRequest) -> Self {
        match value {
            ContainerRequest::Image(create_image) => {
                Self::Resource(Id::new(ResourceType::Image, create_image.id.0))
            }
            ContainerRequest::Volume(create_volume) => {
                Self::Resource(Id::new(ResourceType::Volume, create_volume.id.0))
            }
            ContainerRequest::Network(create_network) => {
                Self::Resource(Id::new(ResourceType::Network, create_network.id.0))
            }
            ContainerRequest::Container(create_container) => {
                Self::Resource(Id::new(ResourceType::Container, create_container.id.0))
            }
            ContainerRequest::Deployment(create_deployment) => {
                Self::Resource(Id::new(ResourceType::Deployment, create_deployment.id.0))
            }
            ContainerRequest::DeploymentCommand(cmd) => Self::DeploymentCmd(*cmd),
            ContainerRequest::DeploymentUpdate(update) => Self::DeploymentUpdate(*update),
        }
    }
}

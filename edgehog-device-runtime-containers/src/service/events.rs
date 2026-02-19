// This file is part of Edgehog.
//
// Copyright 2025, 2026 SECO Mind Srl
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
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

use edgehog_store::models::containers::deployment::DeploymentStatus;
use tokio::sync::mpsc;
use tracing::{error, instrument};
use uuid::Uuid;

use crate::{
    events::deployment::{DeploymentEvent, EventStatus},
    properties::Client,
    requests::{
        ContainerRequest,
        deployment::{DeploymentCommand, DeploymentUpdate},
    },
    store::StateStore,
};

/// Error returned by the [`ServiceHandle`]
#[derive(Debug, thiserror::Error, displaydoc::Display)]
pub enum EventError {
    /// couldn't handle the event since the service exited.
    Disconnected,
}

use super::{CommandValue, Id, ResourceType};

/// Handle to the [container service](super::Service).
#[derive(Debug)]
pub struct ServiceHandle<D> {
    /// Queue of events received from Astarte.
    events: mpsc::UnboundedSender<ContainerEvent>,
    device: D,
    store: StateStore,
}

impl<D> ServiceHandle<D> {
    /// Create the handle from the [channel](mpsc::UnboundedSender) shared with the [`Service`](super::Service).
    pub fn new(
        device: D,
        store: StateStore,
        events: mpsc::UnboundedSender<ContainerEvent>,
    ) -> Self {
        Self {
            events,
            device,
            store,
        }
    }

    /// Handles an event from the image.
    #[instrument(skip_all)]
    pub async fn on_event(&mut self, request: ContainerRequest) -> Result<(), EventError>
    where
        D: Client + Sync + 'static,
    {
        let event = ContainerEvent::from(&request);

        let deployment_id = request.deployment_id();

        self.persist_request(deployment_id, request).await;

        self.events.send(event).map_err(|_err| {
            error!("the container service disconnected");

            EventError::Disconnected
        })?;

        Ok(())
    }

    #[instrument(skip_all, fields(deployment_id))]
    async fn persist_request(&mut self, deployment_id: Uuid, request: ContainerRequest)
    where
        D: Client + Sync + 'static,
    {
        let res = match request {
            ContainerRequest::Image(create_image) => self.store.create_image(create_image).await,
            ContainerRequest::Volume(create_volume) => {
                self.store.create_volume(create_volume).await
            }
            ContainerRequest::Network(create_network) => {
                self.store.create_network(create_network).await
            }
            ContainerRequest::DeviceMapping(create_device_mapping) => {
                self.store
                    .create_device_mapping(create_device_mapping)
                    .await
            }
            ContainerRequest::Container(create_container) => {
                self.store.create_container(create_container).await
            }
            ContainerRequest::Deployment(create_deployment) => {
                self.store.create_deployment(create_deployment).await
            }
            ContainerRequest::DeploymentCommand(DeploymentCommand { id, command }) => {
                self.store
                    .update_deployment_status(id, command.into())
                    .await
            }
            ContainerRequest::DeploymentUpdate(DeploymentUpdate { from, to }) => {
                self.store.deployment_update(from, to).await
            }
        };

        if let Err(err) = res {
            let error = eyre::Report::new(err);

            error!(error = format!("{:#}", error), "couldn't store request");

            DeploymentEvent::with_error(EventStatus::Error, error.as_ref())
                .send(&deployment_id, &mut self.device)
                .await;
        }
    }
}

/// Event sent by the [`ServiceHandle`] to the [`Service`](crate::service::Service)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerEvent {
    /// Resource creation request event.
    Resource {
        /// Unique ID of the resource
        resource: Id,
        /// Deployment ID of the request
        deployment: Uuid,
    },
    /// Deployment command event.
    DeploymentCmd(DeploymentCommand),
    /// Deployment update event.
    DeploymentUpdate(DeploymentUpdate),
    /// Container runtime event happened.
    ///
    /// We need to handle deletion or a container stopped outside of Edgehog.
    Refresh(Id),
}

impl From<&ContainerRequest> for ContainerEvent {
    fn from(value: &ContainerRequest) -> Self {
        match value {
            ContainerRequest::Image(create_image) => {
                let resource = Id::new(ResourceType::Image, create_image.id.0);

                ContainerEvent::Resource {
                    resource,
                    deployment: create_image.deployment_id.0,
                }
            }
            ContainerRequest::Volume(create_volume) => {
                let resource = Id::new(ResourceType::Volume, create_volume.id.0);

                ContainerEvent::Resource {
                    resource,
                    deployment: create_volume.deployment_id.0,
                }
            }
            ContainerRequest::Network(create_network) => {
                let resource = Id::new(ResourceType::Network, create_network.id.0);

                ContainerEvent::Resource {
                    resource,
                    deployment: create_network.deployment_id.0,
                }
            }
            ContainerRequest::DeviceMapping(create_device_mapping) => {
                let resource = Id::new(ResourceType::DeviceMapping, create_device_mapping.id.0);

                ContainerEvent::Resource {
                    resource,
                    deployment: create_device_mapping.deployment_id.0,
                }
            }
            ContainerRequest::Container(create_container) => {
                let resource = Id::new(ResourceType::Container, create_container.id.0);

                ContainerEvent::Resource {
                    resource,
                    deployment: create_container.deployment_id.0,
                }
            }
            ContainerRequest::Deployment(create_deployment) => {
                let resource = Id::new(ResourceType::Deployment, create_deployment.id.0);

                ContainerEvent::Resource {
                    resource,
                    deployment: create_deployment.id.0,
                }
            }
            ContainerRequest::DeploymentCommand(cmd) => Self::DeploymentCmd(*cmd),
            ContainerRequest::DeploymentUpdate(update) => Self::DeploymentUpdate(*update),
        }
    }
}

impl From<CommandValue> for DeploymentStatus {
    fn from(value: CommandValue) -> Self {
        match value {
            CommandValue::Start => Self::Started,
            CommandValue::Stop => Self::Stopped,
            CommandValue::Delete => Self::Deleted,
        }
    }
}

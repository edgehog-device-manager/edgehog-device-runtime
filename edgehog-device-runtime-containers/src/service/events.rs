// This file is part of Edgehog.
//
// Copyright 2025, 2026 SECO Mind Srl
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
            ContainerRequest::DeviceRequest(create_device_request) => {
                self.store
                    .create_device_request(create_device_request)
                    .await
            }
            ContainerRequest::FileBind(create_file_bind) => {
                self.store.create_file_bind(create_file_bind).await
            }
            ContainerRequest::EnvFile(create_env_file) => {
                self.store.create_env_file(create_env_file).await
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
            ContainerRequest::DeviceRequest(create_device_request) => {
                let resource = Id::new(ResourceType::DeviceRequest, create_device_request.id.0);

                ContainerEvent::Resource {
                    resource,
                    deployment: create_device_request.deployment_id.0,
                }
            }
            ContainerRequest::FileBind(create_file_bind) => {
                let resource = Id::new(ResourceType::FileBind, create_file_bind.id.0);

                ContainerEvent::Resource {
                    resource,
                    deployment: create_file_bind.deployment_id.0,
                }
            }
            ContainerRequest::EnvFile(create_env_file) => {
                let resource = Id::new(ResourceType::EnvFile, create_env_file.id.0);

                ContainerEvent::Resource {
                    resource,
                    deployment: create_env_file.deployment_id.0,
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

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::requests::container::CreateContainer;
    use crate::requests::container::tests::create_container_req;
    use crate::requests::device_mapping::tests::create_device_mapping_req;
    use crate::requests::device_request::tests::create_device_request;
    use crate::requests::env_file::tests::create_env_file_req;
    use crate::requests::file_bind::tests::create_file_bind_req;
    use crate::requests::image::tests::create_image_req;
    use crate::requests::network::tests::create_network_req;
    use crate::requests::volume::tests::create_volume_req;

    use super::*;

    const RESOURCE_ID: Uuid = uuid::uuid!("11017edd-9738-4617-bcd3-9fdd12dce5a5");
    const DEPLOYMENT_ID: Uuid = uuid::uuid!("923f765a-3bbb-4252-be88-b2c85e1d9911");

    fn mock_container_req() -> Box<CreateContainer> {
        Box::new(create_container_req(
            DEPLOYMENT_ID,
            &create_image_req(DEPLOYMENT_ID),
            &create_volume_req(DEPLOYMENT_ID),
            &create_network_req(DEPLOYMENT_ID),
            &create_device_mapping_req(DEPLOYMENT_ID),
            &create_device_request(DEPLOYMENT_ID),
            &create_file_bind_req(DEPLOYMENT_ID),
            &create_env_file_req(DEPLOYMENT_ID),
        ))
    }

    #[rstest]
    #[case(ContainerRequest::Image(create_image_req(DEPLOYMENT_ID)), ContainerEvent::Resource {
        resource: Id::new(ResourceType::Image, RESOURCE_ID),
        deployment: DEPLOYMENT_ID,
    })]
    #[case(ContainerRequest::Volume(create_volume_req(DEPLOYMENT_ID)), ContainerEvent::Resource {
        resource: Id::new(ResourceType::Volume, RESOURCE_ID),
        deployment: DEPLOYMENT_ID,
    })]
    #[case(ContainerRequest::Network(create_network_req(DEPLOYMENT_ID)), ContainerEvent::Resource {
        resource: Id::new(ResourceType::Network, RESOURCE_ID),
        deployment: DEPLOYMENT_ID,
    })]
    #[case(ContainerRequest::DeviceMapping(create_device_mapping_req(DEPLOYMENT_ID)), ContainerEvent::Resource {
        resource: Id::new(ResourceType::DeviceMapping, RESOURCE_ID),
        deployment: DEPLOYMENT_ID,
    })]
    #[case(ContainerRequest::DeviceRequest(create_device_request(DEPLOYMENT_ID)), ContainerEvent::Resource {
        resource: Id::new(ResourceType::DeviceRequest, RESOURCE_ID),
        deployment: DEPLOYMENT_ID,
    })]
    #[case(ContainerRequest::FileBind(create_file_bind_req(DEPLOYMENT_ID)), ContainerEvent::Resource {
        resource: Id::new(ResourceType::FileBind, RESOURCE_ID),
        deployment: DEPLOYMENT_ID,
    })]
    #[case(ContainerRequest::EnvFile(create_env_file_req(DEPLOYMENT_ID)), ContainerEvent::Resource {
        resource: Id::new(ResourceType::EnvFile, RESOURCE_ID),
        deployment: DEPLOYMENT_ID,
    })]
    #[case(ContainerRequest::Container(mock_container_req()), ContainerEvent::Resource {
        resource: Id::new(ResourceType::Container, RESOURCE_ID),
        deployment: DEPLOYMENT_ID,
    })]
    fn event_from_request(#[case] case: ContainerRequest, #[case] exp: ContainerEvent) {
        let res = ContainerEvent::from(&case);

        assert_eq!(res, exp);
    }

    #[rstest]
    #[case(CommandValue::Start, DeploymentStatus::Started)]
    #[case(CommandValue::Stop, DeploymentStatus::Stopped)]
    #[case(CommandValue::Delete, DeploymentStatus::Deleted)]
    fn deployment_status_from_command_value(
        #[case] case: CommandValue,
        #[case] exp: DeploymentStatus,
    ) {
        let res = DeploymentStatus::from(case);

        assert_eq!(res, exp);
    }
}

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

use edgehog_store::models::containers::{deployment::DeploymentStatus, volume::VolumeStatus};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, instrument};
use uuid::Uuid;

use crate::{
    events::deployment::{DeploymentEvent, EventStatus},
    properties::Client,
    requests::{
        ContainerRequest,
        container::CreateContainer,
        deployment::{CreateDeployment, DeploymentCommand, DeploymentUpdate},
        device_mapping::CreateDeviceMapping,
        device_request::CreateDeviceRequest,
        env_file::CreateEnvFile,
        file_bind::CreateFileBind,
        image::CreateImage,
        network::CreateNetwork,
        volume::{self, CreateVolume, VolumeCommand},
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

enum PersistRequest {
    Deployment(DeploymentRequest),
    Volume(VolumeRequest),
}

impl From<ContainerRequest> for PersistRequest {
    fn from(value: ContainerRequest) -> Self {
        match value {
            ContainerRequest::Image(create_image) => {
                Self::Deployment(DeploymentRequest::Image(create_image))
            }
            ContainerRequest::Volume(create_volume) => {
                Self::Deployment(DeploymentRequest::Volume(create_volume))
            }
            ContainerRequest::Network(create_network) => {
                Self::Deployment(DeploymentRequest::Network(create_network))
            }
            ContainerRequest::DeviceMapping(create_device_mapping) => {
                Self::Deployment(DeploymentRequest::DeviceMapping(create_device_mapping))
            }
            ContainerRequest::DeviceRequest(create_device_request) => {
                Self::Deployment(DeploymentRequest::DeviceRequest(create_device_request))
            }
            ContainerRequest::FileBind(create_file_bind) => {
                Self::Deployment(DeploymentRequest::FileBind(create_file_bind))
            }
            ContainerRequest::EnvFile(create_env_file) => {
                Self::Deployment(DeploymentRequest::EnvFile(create_env_file))
            }
            ContainerRequest::Container(create_container) => {
                Self::Deployment(DeploymentRequest::Container(create_container))
            }
            ContainerRequest::Deployment(create_deployment) => {
                Self::Deployment(DeploymentRequest::Deployment(create_deployment))
            }
            ContainerRequest::DeploymentCommand(deployment_command) => {
                Self::Deployment(DeploymentRequest::DeploymentCommand(deployment_command))
            }
            ContainerRequest::DeploymentUpdate(deployment_update) => {
                Self::Deployment(DeploymentRequest::DeploymentUpdate(deployment_update))
            }
            ContainerRequest::VolumeCommand(volume_command) => {
                Self::Volume(VolumeRequest::VolumeCommand(volume_command))
            }
        }
    }
}

enum DeploymentRequest {
    /// Request to create an image.
    Image(CreateImage),
    /// Request to create a volume.
    Volume(CreateVolume),
    /// Request to create a network.
    Network(CreateNetwork),
    /// Request to create a device mapping.
    DeviceMapping(CreateDeviceMapping),
    /// Request to create a device request.
    DeviceRequest(CreateDeviceRequest),
    /// Request to create a file bind.
    FileBind(CreateFileBind),
    /// Request to create an env file.
    EnvFile(CreateEnvFile),
    /// Request to create a container.
    Container(Box<CreateContainer>),
    /// Request to create a deployment.
    Deployment(CreateDeployment),
    /// Command for a deployment
    DeploymentCommand(DeploymentCommand),
    /// Update between two deployments
    DeploymentUpdate(DeploymentUpdate),
}

impl DeploymentRequest {
    pub(crate) fn deployment_id(&self) -> Uuid {
        match self {
            DeploymentRequest::Image(value) => value.deployment_id.0,
            DeploymentRequest::Volume(value) => value.deployment_id.0,
            DeploymentRequest::Network(value) => value.deployment_id.0,
            DeploymentRequest::Container(value) => value.deployment_id.0,
            DeploymentRequest::DeviceMapping(value) => value.deployment_id.0,
            DeploymentRequest::DeviceRequest(value) => value.deployment_id.0,
            DeploymentRequest::FileBind(value) => value.deployment_id.0,
            DeploymentRequest::EnvFile(value) => value.deployment_id.0,
            DeploymentRequest::Deployment(create_deployment) => create_deployment.id.0,
            DeploymentRequest::DeploymentCommand(deployment_command) => deployment_command.id,
            DeploymentRequest::DeploymentUpdate(deployment_update) => deployment_update.from,
        }
    }
}

enum VolumeRequest {
    /// Request to execute a command on a volume
    VolumeCommand(VolumeCommand),
}

/// Handle to the [container service](super::Service).
#[derive(Debug)]
pub struct ServiceHandle<D> {
    /// Queue of events received from Astarte.
    events: mpsc::Sender<ContainerEvent>,
    device: D,
    store: StateStore,
}

impl<D> ServiceHandle<D> {
    /// Create the handle from the [channel](mpsc::UnboundedSender) shared with the [`Service`](super::Service).
    pub fn new(device: D, store: StateStore, events: mpsc::Sender<ContainerEvent>) -> Self {
        Self {
            events,
            device,
            store,
        }
    }

    /// Handles an event from the image.
    #[instrument(skip_all)]
    pub async fn on_event(
        &mut self,
        cancel: &CancellationToken,
        request: ContainerRequest,
    ) -> Result<(), EventError>
    where
        D: Client + Sync + 'static,
    {
        let event = ContainerEvent::from(&request);

        self.persist_request(request.into()).await;

        let Some(res) = cancel.run_until_cancelled(self.events.send(event)).await else {
            return Ok(());
        };

        res.map_err(|_err| {
            error!("the container service disconnected");

            EventError::Disconnected
        })?;

        Ok(())
    }

    #[instrument(skip_all, fields(deployment_id))]
    async fn persist_request(&mut self, request: PersistRequest)
    where
        D: Client + Sync + 'static,
    {
        match request {
            PersistRequest::Deployment(deployment_request) => {
                self.persist_deployment_request(deployment_request).await
            }
            PersistRequest::Volume(volume_request) => {
                self.persist_volume_request(volume_request).await
            }
        }
    }

    #[instrument(skip_all)]
    async fn persist_deployment_request(&mut self, deployment_request: DeploymentRequest)
    where
        D: Client + Sync + 'static,
    {
        let deployment_id = deployment_request.deployment_id();

        let res = match deployment_request {
            DeploymentRequest::Image(create_image) => self.store.create_image(create_image).await,
            DeploymentRequest::Volume(create_volume) => {
                self.store.create_volume(create_volume).await
            }
            DeploymentRequest::Network(create_network) => {
                self.store.create_network(create_network).await
            }
            DeploymentRequest::DeviceMapping(create_device_mapping) => {
                self.store
                    .create_device_mapping(create_device_mapping)
                    .await
            }
            DeploymentRequest::DeviceRequest(create_device_request) => {
                self.store
                    .create_device_request(create_device_request)
                    .await
            }
            DeploymentRequest::FileBind(create_file_bind) => {
                self.store.create_file_bind(create_file_bind).await
            }
            DeploymentRequest::EnvFile(create_env_file) => {
                self.store.create_env_file(create_env_file).await
            }
            DeploymentRequest::Container(create_container) => {
                self.store.create_container(create_container).await
            }
            DeploymentRequest::Deployment(create_deployment) => {
                self.store.create_deployment(create_deployment).await
            }
            DeploymentRequest::DeploymentCommand(DeploymentCommand { id, command }) => {
                self.store
                    .update_deployment_status(id, command.into())
                    .await
            }
            DeploymentRequest::DeploymentUpdate(DeploymentUpdate { from, to }) => {
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

    async fn persist_volume_request(&mut self, volume_request: VolumeRequest)
    where
        D: Client + Sync + 'static,
    {
        let res = match volume_request {
            VolumeRequest::VolumeCommand(VolumeCommand {
                volume_id, command, ..
            }) => {
                self.store
                    .update_volume_status(volume_id, command.into())
                    .await
            }
        };

        if let Err(err) = res {
            let error = eyre::Report::new(err);

            error!(%error, "couldn't store volume command");
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
    /// Volume command event
    VolumeCmd(VolumeCommand),
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
            ContainerRequest::VolumeCommand(cmd) => Self::VolumeCmd(*cmd),
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

impl From<volume::CommandValue> for VolumeStatus {
    fn from(value: volume::CommandValue) -> Self {
        match value {
            volume::CommandValue::Delete => Self::Deleting,
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::requests::container::tests::create_container_req;
    use crate::requests::device_mapping::tests::create_device_mapping_req;
    use crate::requests::device_request::tests::create_device_request;
    use crate::requests::env_file::tests::create_env_file_req;
    use crate::requests::file_bind::tests::create_file_bind_req;
    use crate::requests::image::tests::create_image_req;
    use crate::requests::network::tests::create_network_req;
    use crate::requests::volume::tests::create_volume_req;

    use super::*;

    const DEPLOYMENT_ID: Uuid = uuid::uuid!("923f765a-3bbb-4252-be88-b2c85e1d9911");

    #[test]
    fn event_from_request() {
        let image = create_image_req(DEPLOYMENT_ID);
        let volume = create_volume_req(DEPLOYMENT_ID);
        let network = create_network_req(DEPLOYMENT_ID);
        let device_mapping = create_device_mapping_req(DEPLOYMENT_ID);
        let device_request = create_device_request(DEPLOYMENT_ID);
        let file_bind = create_file_bind_req(DEPLOYMENT_ID);
        let env_file = create_env_file_req(DEPLOYMENT_ID);
        let container = create_container_req(
            DEPLOYMENT_ID,
            &image,
            &volume,
            &network,
            &device_mapping,
            &device_request,
            &file_bind,
            &env_file,
        );

        let cases = [
            (
                ContainerRequest::Image(image.clone()),
                ContainerEvent::Resource {
                    resource: Id::new(ResourceType::Image, image.id.0),
                    deployment: DEPLOYMENT_ID,
                },
            ),
            (
                ContainerRequest::Volume(volume.clone()),
                ContainerEvent::Resource {
                    resource: Id::new(ResourceType::Volume, volume.id.0),
                    deployment: DEPLOYMENT_ID,
                },
            ),
            (
                ContainerRequest::Network(network.clone()),
                ContainerEvent::Resource {
                    resource: Id::new(ResourceType::Network, network.id.0),
                    deployment: DEPLOYMENT_ID,
                },
            ),
            (
                ContainerRequest::DeviceMapping(device_mapping.clone()),
                ContainerEvent::Resource {
                    resource: Id::new(ResourceType::DeviceMapping, device_mapping.id.0),
                    deployment: DEPLOYMENT_ID,
                },
            ),
            (
                ContainerRequest::DeviceRequest(device_request.clone()),
                ContainerEvent::Resource {
                    resource: Id::new(ResourceType::DeviceRequest, device_request.id.0),
                    deployment: DEPLOYMENT_ID,
                },
            ),
            (
                ContainerRequest::FileBind(file_bind.clone()),
                ContainerEvent::Resource {
                    resource: Id::new(ResourceType::FileBind, file_bind.id.0),
                    deployment: DEPLOYMENT_ID,
                },
            ),
            (
                ContainerRequest::EnvFile(env_file.clone()),
                ContainerEvent::Resource {
                    resource: Id::new(ResourceType::EnvFile, env_file.id.0),
                    deployment: DEPLOYMENT_ID,
                },
            ),
            (
                ContainerRequest::Container(Box::new(container.clone())),
                ContainerEvent::Resource {
                    resource: Id::new(ResourceType::Container, container.id.0),
                    deployment: DEPLOYMENT_ID,
                },
            ),
        ];

        for (case, exp) in cases {
            let res = ContainerEvent::from(&case);

            assert_eq!(res, exp);
        }
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

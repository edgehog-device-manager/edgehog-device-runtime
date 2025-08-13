// This file is part of Edgehog.
//
// Copyright 2024 - 2025 SECO Mind Srl
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

//! Service to receive and handle the Astarte events.

use std::fmt::{Debug, Display};

use astarte_device_sdk::event::FromEventError;
use edgehog_store::{conversions::SqlUuid, models::containers::deployment::DeploymentStatus};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use crate::{
    error::DockerError,
    events::deployment::{DeploymentEvent, EventStatus},
    properties::{deployment::AvailableDeployment, AvailableProp, Client},
    requests::{
        deployment::{CommandValue, DeploymentCommand, DeploymentUpdate},
        ReqError,
    },
    resource::{
        container::ContainerResource, deployment::Deployment, image::ImageResource,
        network::NetworkResource, volume::VolumeResource, Context, Create, Resource, ResourceError,
        State,
    },
    store::{StateStore, StoreError},
    Docker,
};

use self::events::AstarteEvent;

pub mod events;

type Result<T> = std::result::Result<T, ServiceError>;

/// Error from the [`Service`].
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum ServiceError {
    /// error converting event
    FromEvent(#[from] FromEventError),
    /// docker operation failed
    Docker(#[source] DockerError),
    /// couldn't send data to Astarte
    Astarte(#[from] astarte_device_sdk::Error),
    /// couldn't process request
    Request(#[from] ReqError),
    /// store operation failed
    Store(#[from] StoreError),
    /// couldn't complete operation on a resource
    Resource(#[from] ResourceError),
}

impl<T> From<T> for ServiceError
where
    T: Into<DockerError>,
{
    fn from(value: T) -> Self {
        ServiceError::Docker(value.into())
    }
}

/// Manages the state of the Nodes.
///
/// It handles the events received from Astarte, storing and updating the new container resources
/// and commands that are received by the Runtime.
#[derive(Debug)]
pub struct Service<D> {
    client: Docker,
    device: D,
    /// Queue of events received from Astarte.
    events: mpsc::UnboundedReceiver<AstarteEvent>,
    store: StateStore,
}

impl<D> Service<D> {
    /// Create a new service
    #[doc(hidden)]
    #[must_use]
    pub fn new(
        client: Docker,
        device: D,
        events: mpsc::UnboundedReceiver<AstarteEvent>,
        store: StateStore,
    ) -> Self {
        Self {
            client,
            device,
            events,
            store,
        }
    }

    fn context(&mut self, id: impl Into<Uuid>) -> Context<'_, D> {
        Context {
            id: id.into(),
            store: &mut self.store,
            device: &mut self.device,
            client: &mut self.client,
        }
    }

    /// Initialize the service, it will load all the already stored properties
    #[instrument(skip_all)]
    pub async fn init(&mut self) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        self.publish_received().await?;

        // Delete and stop must be before the start to ensure the ports and other resources are
        // freed before starting the containers.
        self.init_delete_deployments().await?;
        self.init_stop_deployments().await?;
        self.init_start_deployments().await?;

        info!("init completed");

        Ok(())
    }

    #[instrument(skip_all)]
    async fn publish_received(&mut self) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        for id in self.store.load_images_to_publish().await? {
            ImageResource::publish(self.context(id)).await?;
        }

        for id in self.store.load_volumes_to_publish().await? {
            VolumeResource::publish(self.context(id)).await?;
        }

        for id in self.store.load_networks_to_publish().await? {
            NetworkResource::publish(self.context(id)).await?;
        }

        for id in self.store.load_containers_to_publish().await? {
            ContainerResource::publish(self.context(id)).await?;
        }

        for id in self
            .store
            .load_deployments_in(DeploymentStatus::Received)
            .await?
        {
            Deployment::publish(self.context(id)).await?;
        }

        Ok(())
    }

    #[instrument(skip_all)]
    async fn init_start_deployments(&mut self) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        for id in self
            .store
            .load_deployments_in(DeploymentStatus::Started)
            .await?
        {
            self.start(*id).await;
        }

        Ok(())
    }

    #[instrument(skip_all)]
    async fn init_stop_deployments(&mut self) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        for id in self
            .store
            .load_deployments_in(DeploymentStatus::Stopped)
            .await?
        {
            self.stop(*id).await;
        }

        Ok(())
    }

    #[instrument(skip_all)]
    async fn init_delete_deployments(&mut self) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        for id in self
            .store
            .load_deployments_in(DeploymentStatus::Deleted)
            .await?
        {
            self.delete(*id).await;
        }

        Ok(())
    }

    /// Blocking call that will handle the events from Astarte and the containers.
    #[instrument(skip_all)]
    pub async fn handle_events(&mut self)
    where
        D: Client + Send + Sync + 'static,
    {
        while let Some(event) = self.events.recv().await {
            self.on_event(event).await;
        }

        info!("event receiver disconnected");
    }

    #[instrument(skip_all)]
    async fn on_event(&mut self, event: AstarteEvent)
    where
        D: Client + Send + Sync + 'static,
    {
        match event {
            AstarteEvent::Resource {
                resource,
                deployment,
            } => {
                self.resource_req(resource, deployment).await;
            }
            AstarteEvent::DeploymentCmd(DeploymentCommand {
                id,
                command: CommandValue::Start,
            }) => {
                self.start(id).await;
            }
            AstarteEvent::DeploymentCmd(DeploymentCommand {
                id,
                command: CommandValue::Stop,
            }) => {
                self.stop(id).await;
            }
            AstarteEvent::DeploymentCmd(DeploymentCommand {
                id,
                command: CommandValue::Delete,
            }) => {
                self.delete(id).await;
            }
            AstarteEvent::DeploymentUpdate(from_to) => {
                self.update(from_to).await;
            }
        }
    }

    #[instrument(skip_all, fields(%id))]
    async fn resource_req(&mut self, id: Id, deployment_id: Uuid)
    where
        D: Client + Send + Sync + 'static,
    {
        let res = match id.resource_type() {
            ResourceType::Image => ImageResource::publish(self.context(*id.uuid())).await,
            ResourceType::Volume => VolumeResource::publish(self.context(*id.uuid())).await,
            ResourceType::Network => NetworkResource::publish(self.context(*id.uuid())).await,
            ResourceType::Container => ContainerResource::publish(self.context(*id.uuid())).await,
            ResourceType::Deployment => Deployment::publish(self.context(*id.uuid())).await,
        };

        if let Err(err) = res {
            let error = format!("{:#}", eyre::Report::new(err));
            error!(error, "failed to create resource");

            DeploymentEvent::new(EventStatus::Error, error)
                .send(&deployment_id, &mut self.device)
                .await;
        }
    }

    /// Will start a Deployment
    #[instrument(skip(self))]
    pub async fn start(&mut self, id: Uuid)
    where
        D: Client + Send + Sync + 'static,
    {
        let deployment = match self.store.find_complete_deployment(id).await {
            Ok(Some(deployment)) => deployment,
            Ok(None) => {
                error!("{id} not found");

                DeploymentEvent::new(EventStatus::Error, format!("{id} not found"))
                    .send(&id, &mut self.device)
                    .await;

                return;
            }
            Err(err) => {
                let err = format!("{:#}", eyre::Report::new(err));

                error!(error = err, "couldn't start deployment");

                DeploymentEvent::new(EventStatus::Error, err)
                    .send(&id, &mut self.device)
                    .await;

                return;
            }
        };

        info!("starting deployment");

        DeploymentEvent::new(EventStatus::Starting, "")
            .send(&id, &mut self.device)
            .await;

        if let Err(err) = self.start_deployment(id, deployment).await {
            let err = format!("{:#}", eyre::Report::new(err));

            error!(error = err, "couldn't start deployment");

            DeploymentEvent::new(EventStatus::Error, err)
                .send(&id, &mut self.device)
                .await;

            return;
        }

        DeploymentEvent::new(EventStatus::Started, "")
            .send(&id, &mut self.device)
            .await;

        info!("deployment started");
    }

    async fn start_deployment(&mut self, deployment_id: Uuid, deployment: Deployment) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        for id in deployment.images {
            ImageResource::up(self.context(id)).await?;
        }

        for id in deployment.volumes {
            VolumeResource::up(self.context(id)).await?;
        }

        for id in deployment.networks {
            NetworkResource::up(self.context(id)).await?;
        }

        for id in deployment.containers {
            let mut container = ContainerResource::up(self.context(id)).await?;

            container.start(self.context(id)).await?;
        }

        AvailableDeployment::new(&deployment_id)
            .send(
                &mut self.device,
                crate::properties::deployment::DeploymentStatus::Started,
            )
            .await
            .map_err(ResourceError::Property)?;

        Ok(())
    }

    /// Will stop an application
    #[instrument(skip(self))]
    pub async fn stop(&mut self, id: Uuid)
    where
        D: Client + Send + Sync + 'static,
    {
        let containers = match self.store.load_deployment_containers(id).await {
            Ok(Some(containers)) => containers,
            Ok(None) => {
                error!("{id} not found");

                DeploymentEvent::new(EventStatus::Error, format!("{id} not found"))
                    .send(&id, &mut self.device)
                    .await;

                return;
            }
            Err(err) => {
                let err = format!("{:#}", eyre::Report::new(err));

                error!(error = err, "couldn't start deployment");

                DeploymentEvent::new(EventStatus::Error, err)
                    .send(&id, &mut self.device)
                    .await;

                return;
            }
        };

        info!("stopping deployment");

        DeploymentEvent::new(EventStatus::Stopping, "")
            .send(&id, &mut self.device)
            .await;

        if let Err(err) = self.stop_deployment(id, containers).await {
            let err = format!("{:#}", eyre::Report::new(err));

            error!(error = err, "couldn't stop deployment");

            DeploymentEvent::new(EventStatus::Error, err)
                .send(&id, &mut self.device)
                .await;

            return;
        }

        DeploymentEvent::new(EventStatus::Stopped, "")
            .send(&id, &mut self.device)
            .await;

        info!("deployment stopped");
    }

    #[instrument(skip(self, containers))]
    async fn stop_deployment(&mut self, deployment: Uuid, containers: Vec<SqlUuid>) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        for id in containers {
            debug!(%id, "stopping container");

            let mut ctx = self.context(id);
            let (state, mut container) = ContainerResource::fetch(&mut ctx).await?;

            match state {
                State::Missing => {
                    warn!(%id, "container already missing, cannot stop");

                    continue;
                }
                State::Created => {}
            }

            container.stop(ctx).await?;
        }

        AvailableDeployment::new(&deployment)
            .send(
                &mut self.device,
                crate::properties::deployment::DeploymentStatus::Stopped,
            )
            .await
            .map_err(ResourceError::from)?;

        Ok(())
    }

    /// Will delete an application
    #[instrument(skip(self))]
    pub async fn delete(&mut self, id: Uuid)
    where
        D: Client + Send + Sync + 'static,
    {
        let deployment = match self.store.find_deployment_for_delete(id).await {
            Ok(Some(deployment)) => deployment,
            Ok(None) => {
                error!("{id} not found");

                DeploymentEvent::new(EventStatus::Error, format!("{id} not found"))
                    .send(&id, &mut self.device)
                    .await;

                return;
            }
            Err(err) => {
                let err = format!("{:#}", eyre::Report::new(err));

                error!(error = err, "couldn't delete deployment");

                DeploymentEvent::new(EventStatus::Error, err)
                    .send(&id, &mut self.device)
                    .await;

                return;
            }
        };

        info!("deleting deployment");

        DeploymentEvent::new(EventStatus::Deleting, "")
            .send(&id, &mut self.device)
            .await;

        if let Err(err) = self.delete_deployment(id, deployment).await {
            let err = format!("{:#}", eyre::Report::new(err));

            error!(error = err, "couldn't delete deployment");

            DeploymentEvent::new(EventStatus::Error, err)
                .send(&id, &mut self.device)
                .await;

            return;
        }

        info!("deployment deleted");
    }

    async fn delete_deployment(&mut self, deployment_id: Uuid, deployment: Deployment) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        for id in deployment.containers {
            ContainerResource::down(self.context(id)).await?;
        }

        for id in deployment.volumes {
            VolumeResource::down(self.context(id)).await?;
        }

        for id in deployment.networks {
            NetworkResource::down(self.context(id)).await?;
        }

        for id in deployment.images {
            ImageResource::down(self.context(id)).await?;
        }

        AvailableDeployment::new(&deployment_id)
            .unset(&mut self.device)
            .await
            .map_err(ResourceError::from)?;

        self.store.delete_deployment(deployment_id).await?;

        Ok(())
    }

    /// Will update an application between deployments
    #[instrument(skip(self))]
    pub async fn update(&mut self, bundle: DeploymentUpdate)
    where
        D: Client + Send + Sync + 'static,
    {
        let from_deployment = match self
            .store
            .load_deployment_containers_update_from(bundle)
            .await
        {
            Ok(Some(deployment)) => deployment,
            Ok(None) => {
                let msg = format!("{} not found", bundle.from);
                error!("{msg}");

                DeploymentEvent::new(EventStatus::Error, msg)
                    .send(&bundle.from, &mut self.device)
                    .await;

                return;
            }
            Err(err) => {
                let err = format!("{:#}", eyre::Report::new(err));

                error!(error = err, "couldn't update deployment");

                DeploymentEvent::new(EventStatus::Error, err)
                    .send(&bundle.from, &mut self.device)
                    .await;

                return;
            }
        };

        let to_deployment = match self.store.find_complete_deployment(bundle.to).await {
            Ok(Some(deployment)) => deployment,
            Ok(None) => {
                let msg = format!("{} not found", bundle.to);
                error!("{msg}");

                DeploymentEvent::new(EventStatus::Error, msg)
                    .send(&bundle.to, &mut self.device)
                    .await;

                return;
            }
            Err(err) => {
                let err = format!("{:#}", eyre::Report::new(err));

                error!(error = err, "couldn't update deployment");

                DeploymentEvent::new(EventStatus::Error, err)
                    .send(&bundle.to, &mut self.device)
                    .await;

                return;
            }
        };

        info!("updating deployment");

        DeploymentEvent::new(EventStatus::Updating, "")
            .send(&bundle.from, &mut self.device)
            .await;

        // TODO: consider if it's necessary re-start the `from` containers or a retry logic
        if let Err(err) = self
            .update_deployment(bundle, from_deployment, to_deployment)
            .await
        {
            let err = format!("{:#}", eyre::Report::new(err));

            error!(error = err, "couldn't update deployment");

            DeploymentEvent::new(EventStatus::Error, err)
                .send(&bundle.from, &mut self.device)
                .await;

            return;
        }

        info!("deployment updated");
    }

    async fn update_deployment(
        &mut self,
        bundle: DeploymentUpdate,
        to_stop: Vec<SqlUuid>,
        to_start: Deployment,
    ) -> Result<()>
    where
        D: Client + Send + Sync + 'static,
    {
        self.stop_deployment(bundle.from, to_stop).await?;

        self.start_deployment(bundle.to, to_start).await?;

        Ok(())
    }
}

/// Id of the nodes in the Service graph
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Id {
    rt: ResourceType,
    id: Uuid,
}

impl Id {
    /// Create a new ID
    pub fn new(rt: ResourceType, id: Uuid) -> Self {
        Self { rt, id }
    }

    pub(crate) fn uuid(&self) -> &Uuid {
        &self.id
    }

    fn resource_type(&self) -> ResourceType {
        self.rt
    }
}

impl Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.rt, self.id)
    }
}

/// Type of the container resource [`Id`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ResourceType {
    /// Image resource.
    Image = 0,
    /// Volume resource.
    Volume = 1,
    /// Network resource.
    Network = 2,
    /// Container resource.
    Container = 3,
    /// Deployment resource.
    Deployment = 4,
}

impl Display for ResourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResourceType::Image => write!(f, "Image"),
            ResourceType::Volume => write!(f, "Volume"),
            ResourceType::Network => write!(f, "Network"),
            ResourceType::Container => write!(f, "Container"),
            ResourceType::Deployment => write!(f, "Deployment"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use astarte_device_sdk::aggregate::AstarteObject;
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::transport::mqtt::Mqtt;
    use astarte_device_sdk::{AstarteData, FromEvent};
    use astarte_device_sdk_mock::mockall::Sequence;
    use astarte_device_sdk_mock::MockDeviceClient;
    use bollard::query_parameters::CreateImageOptions;
    use edgehog_store::db;
    use mockall::predicate;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use crate::container::{Binding, Container, ContainerId, PortBindingMap};
    use crate::image::Image;
    use crate::network::{Network, NetworkId};
    use crate::properties::container::ContainerStatus;
    use crate::properties::deployment::DeploymentStatus;
    use crate::requests::container::tests::create_container_request_event;
    use crate::requests::container::RestartPolicy;
    use crate::requests::deployment::tests::create_deployment_request_event;
    use crate::requests::image::tests::create_image_request_event;
    use crate::requests::network::tests::create_network_request_event;
    use crate::requests::volume::tests::create_volume_request_event;
    use crate::requests::ContainerRequest;
    use crate::volume::{Volume, VolumeId};
    use crate::{docker, docker_mock};

    use super::events::ServiceHandle;
    use super::*;

    async fn mock_service(
        tempdir: &TempDir,
        client: Docker,
        device: MockDeviceClient<Mqtt<SqliteStore>>,
    ) -> (
        Service<MockDeviceClient<Mqtt<SqliteStore>>>,
        ServiceHandle<MockDeviceClient<Mqtt<SqliteStore>>>,
    ) {
        let db_file = tempdir.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        let handle = ServiceHandle::new(device.clone(), store.clone_lazy(), tx);
        let service = Service::new(client, device, rx, store);

        (service, handle)
    }

    #[tokio::test]
    async fn should_add_an_image() {
        let tmpdir = TempDir::new().unwrap();

        let id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();

        let client = Docker::connect().await.unwrap();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        device
            .expect_clone()
            .once()
            .in_sequence(&mut seq)
            .returning(MockDeviceClient::<Mqtt<SqliteStore>>::new);

        let image_path = format!("/{id}/pulled");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableImages"),
                predicate::eq(image_path),
                predicate::eq(AstarteData::Boolean(false)),
            )
            .returning(|_, _, _| Ok(()));

        let (mut service, mut handle) = mock_service(&tmpdir, client, device).await;

        let reference = "docker.io/nginx:stable-alpine-slim";
        let create_image_req = create_image_request_event(id, deployment_id, reference, "");

        let req = ContainerRequest::from_event(create_image_req).unwrap();

        handle.on_event(req).await.unwrap();

        let event = service.events.recv().await.unwrap();
        service.on_event(event).await;

        let resource = service.store.find_image(id).await.unwrap().unwrap();

        let exp = Image::new(None, reference.to_string(), None);

        assert_eq!(resource.image, exp);
    }

    #[tokio::test]
    async fn should_add_a_volume() {
        let tempdir = TempDir::new().unwrap();

        let id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();

        let client = Docker::connect().await.unwrap();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        device
            .expect_clone()
            .once()
            .in_sequence(&mut seq)
            .returning(MockDeviceClient::<Mqtt<SqliteStore>>::new);

        let endpoint = format!("/{id}/created");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableVolumes"),
                predicate::eq(endpoint),
                predicate::eq(AstarteData::Boolean(false)),
            )
            .returning(|_, _, _| Ok(()));

        let (mut service, mut handle) = mock_service(&tempdir, client, device).await;

        let create_volume_req =
            create_volume_request_event(id, deployment_id, "local", &["foo=bar", "some="]);

        let req = ContainerRequest::from_event(create_volume_req).unwrap();

        handle.on_event(req).await.unwrap();
        let event = service.events.recv().await.unwrap();
        service.on_event(event).await;

        let resource = service.store.find_volume(id).await.unwrap().unwrap();

        let exp = Volume {
            id: VolumeId::new(id),
            driver: "local".to_string(),
            driver_opts: HashMap::from([
                ("foo".to_string(), "bar".to_string()),
                ("some".to_string(), "".to_string()),
            ]),
        };

        assert_eq!(resource.volume, exp);
    }

    #[tokio::test]
    async fn should_add_a_network() {
        let tempdir = TempDir::new().unwrap();

        let id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();

        let client = Docker::connect().await.unwrap();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        device
            .expect_clone()
            .once()
            .in_sequence(&mut seq)
            .returning(MockDeviceClient::<Mqtt<SqliteStore>>::new);

        let endpoint = format!("/{id}/created");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableNetworks"),
                predicate::eq(endpoint),
                predicate::eq(AstarteData::Boolean(false)),
            )
            .returning(|_, _, _| Ok(()));

        let (mut service, mut handle) = mock_service(&tempdir, client, device).await;

        let create_network_req = create_network_request_event(id, deployment_id, "bridged", &[]);

        let req = ContainerRequest::from_event(create_network_req).unwrap();

        handle.on_event(req).await.unwrap();

        let event = service.events.recv().await.unwrap();
        service.on_event(event).await;

        let resource = service.store.find_network(id).await.unwrap().unwrap();

        let exp = Network {
            id: NetworkId::new(None, id),
            driver: "bridged".to_string(),
            internal: false,
            enable_ipv6: false,
            driver_opts: HashMap::new(),
        };

        assert_eq!(resource.network, exp);
    }

    #[tokio::test]
    async fn should_add_a_container() {
        let tempdir = TempDir::new().unwrap();

        let id = Uuid::new_v4();
        let image_id = Uuid::new_v4();
        let network_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();

        let client = Docker::connect().await.unwrap();
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        device
            .expect_clone()
            .once()
            .in_sequence(&mut seq)
            .returning(MockDeviceClient::<Mqtt<SqliteStore>>::new);

        let image_path = format!("/{image_id}/pulled");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableImages"),
                predicate::eq(image_path),
                predicate::eq(AstarteData::Boolean(false)),
            )
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{network_id}/created");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableNetworks"),
                predicate::eq(endpoint),
                predicate::eq(AstarteData::Boolean(false)),
            )
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{id}/status");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableContainers"),
                predicate::eq(endpoint),
                predicate::eq(AstarteData::from("Received")),
            )
            .returning(|_, _, _| Ok(()));

        let (mut service, mut handle) = mock_service(&tempdir, client, device).await;

        // Image
        let reference = "docker.io/nginx:stable-alpine-slim";
        let create_image_req = create_image_request_event(image_id, deployment_id, reference, "");

        let req = ContainerRequest::from_event(create_image_req).unwrap();
        handle.on_event(req).await.unwrap();
        let event = service.events.recv().await.unwrap();
        service.on_event(event).await;

        // Network
        let create_network_req =
            create_network_request_event(network_id, deployment_id, "bridged", &[]);
        let req = ContainerRequest::from_event(create_network_req).unwrap();
        handle.on_event(req).await.unwrap();
        let event = service.events.recv().await.unwrap();
        service.on_event(event).await;

        // Container
        let create_container_req =
            create_container_request_event(id, deployment_id, image_id, "image", &[network_id]);

        let req = ContainerRequest::from_event(create_container_req).unwrap();

        handle.on_event(req).await.unwrap();

        let event = service.events.recv().await.unwrap();
        service.on_event(event).await;

        let resource = service.store.find_container(id).await.unwrap().unwrap();

        let exp = Container {
            id: ContainerId::new(None, id),
            image: "docker.io/nginx:stable-alpine-slim".to_string(),
            network_mode: "bridge".to_string(),
            networks: vec![network_id.to_string()],
            hostname: Some("hostname".to_string()),
            restart_policy: RestartPolicy::No,
            env: vec!["env".to_string()],
            binds: vec!["binds".to_string()],
            port_bindings: PortBindingMap(HashMap::from_iter([(
                "80/tcp".to_string(),
                vec![Binding {
                    host_ip: None,
                    host_port: Some(80),
                }],
            )])),
            extra_hosts: vec!["host.docker.internal:host-gateway".to_string()],
            privileged: false,
        };

        assert_eq!(resource, exp);
    }

    #[tokio::test]
    async fn should_start_deployment() {
        let tempdir = TempDir::new().unwrap();

        let image_id = Uuid::new_v4();
        let container_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();

        let reference = "docker.io/nginx:stable-alpine-slim";

        let client = docker_mock!(docker::Client::connect_with_local_defaults().unwrap(), {
            use self::docker::tests::not_found_response;
            use futures::StreamExt;

            let mut mock = docker::Client::new();
            let mut seq = mockall::Sequence::new();

            mock.expect_inspect_image()
                .withf(move |name| name == reference)
                .once()
                .in_sequence(&mut seq)
                .returning(|_| Err(not_found_response()));

            mock.expect_create_image()
                .with(
                    predicate::eq(Some(CreateImageOptions {
                        from_image: Some(reference.to_string()),
                        ..Default::default()
                    })),
                    predicate::always(),
                    predicate::eq(None),
                )
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _, _| futures::stream::empty().boxed());

            mock.expect_inspect_image()
                .withf(move |name| name ==reference)
                .once()
                .in_sequence(&mut seq)
                .returning(|_| {
                    Ok(bollard::models::ImageInspect {
                    id: Some(
                        "sha256:d2c94e258dcb3c5ac2798d32e1249e42ef01cba4841c2234249495f87264ac5a".to_string(),
                    ),
                    ..Default::default()
                })
                });

            let container_name = container_id.to_string();
            mock.expect_inspect_container()
                .withf(move |name, _option| name == container_name)
                .once()
                .in_sequence(&mut seq)
                .returning(|_, _| Err(docker::tests::not_found_response()));

            let name_exp = container_id.to_string();
            mock.expect_create_container()
                .withf(move |option, config| {
                    option
                        .as_ref()
                        .and_then(|opt| opt.name.as_ref())
                        .is_some_and(|name| *name == name_exp)
                        && config
                            .image
                            .as_ref()
                            .is_some_and(|image| image == reference)
                })
                .once()
                .in_sequence(&mut seq)
                .returning(move |_, _| {
                    Ok(bollard::models::ContainerCreateResponse {
                        id: "container_id".to_string(),
                        warnings: Vec::new(),
                    })
                });

            mock.expect_start_container()
                .withf(move |id, _| id == "container_id")
                .once()
                .in_sequence(&mut seq)
                .returning(move |_, _| Ok(()));

            mock
        });
        let mut device = MockDeviceClient::<Mqtt<SqliteStore>>::new();
        let mut seq = Sequence::new();

        device
            .expect_clone()
            .once()
            .in_sequence(&mut seq)
            .returning(MockDeviceClient::<Mqtt<SqliteStore>>::new);

        let image_path = format!("/{image_id}/pulled");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableImages"),
                predicate::eq(image_path),
                predicate::eq(AstarteData::Boolean(false)),
            )
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{container_id}/status");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableContainers"),
                predicate::eq(endpoint),
                predicate::eq(AstarteData::from("Received")),
            )
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{deployment_id}/status");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.AvailableDeployments"),
                predicate::eq(endpoint),
                predicate::eq(AstarteData::String("Stopped".to_string())),
            )
            .returning(|_, _, _| Ok(()));

        device
            .expect_send_object_with_timestamp()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.DeploymentEvent"),
                predicate::eq(format!("/{deployment_id}")),
                predicate::eq(AstarteObject::from_iter([
                    ("status".to_string(), AstarteData::from("Starting")),
                    ("message".to_string(), AstarteData::from("")),
                ])),
                predicate::always(),
            )
            .returning(|_, _, _, _| Ok(()));

        let image_path = format!("/{image_id}/pulled");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableImages"
                    && path == (image_path)
                    && *value == AstarteData::Boolean(true)
            })
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{container_id}/status");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableContainers"
                    && path == endpoint
                    && *value == ContainerStatus::Created.to_string()
            })
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{container_id}/status");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableContainers"
                    && path == endpoint
                    && *value == ContainerStatus::Running.to_string()
            })
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{deployment_id}/status");
        device
            .expect_set_property()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableDeployments"
                    && path == endpoint
                    && *value == DeploymentStatus::Started.to_string()
            })
            .returning(|_, _, _| Ok(()));

        device
            .expect_send_object_with_timestamp()
            .once()
            .in_sequence(&mut seq)
            .with(
                predicate::eq("io.edgehog.devicemanager.apps.DeploymentEvent"),
                predicate::eq(format!("/{deployment_id}")),
                predicate::eq(AstarteObject::from_iter([
                    ("status".to_string(), AstarteData::from("Started")),
                    ("message".to_string(), AstarteData::from("")),
                ])),
                predicate::always(),
            )
            .returning(|_, _, _, _| Ok(()));

        let (mut service, mut handle) = mock_service(&tempdir, client, device).await;

        let create_image_req = create_image_request_event(image_id, deployment_id, reference, "");

        let image_req = ContainerRequest::from_event(create_image_req).unwrap();

        let create_container_req = create_container_request_event(
            container_id,
            deployment_id,
            image_id,
            reference,
            &Vec::<Uuid>::new(),
        );

        let container_req = ContainerRequest::from_event(create_container_req).unwrap();

        let create_deployment_req = create_deployment_request_event(
            &deployment_id.to_string(),
            &[&container_id.to_string()],
        );

        let deployment_req = ContainerRequest::from_event(create_deployment_req).unwrap();

        let start = ContainerRequest::DeploymentCommand(DeploymentCommand {
            id: deployment_id,
            command: CommandValue::Start,
        });

        handle.on_event(image_req).await.unwrap();
        handle.on_event(container_req).await.unwrap();
        handle.on_event(deployment_req).await.unwrap();
        handle.on_event(start).await.unwrap();

        let image_event = service.events.recv().await.unwrap();
        service.on_event(image_event).await;
        let container_event = service.events.recv().await.unwrap();
        service.on_event(container_event).await;
        let deployment_event = service.events.recv().await.unwrap();
        service.on_event(deployment_event).await;
        let start_event = service.events.recv().await.unwrap();
        service.on_event(start_event).await;
    }
}

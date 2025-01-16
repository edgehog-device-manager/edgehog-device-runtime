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

use std::{
    collections::HashSet,
    fmt::{Debug, Display},
};

use astarte_device_sdk::event::FromEventError;
use edgehog_store::models::containers::deployment::DeploymentStatus;
use events::ServiceHandle;
use futures::{FutureExt, TryStreamExt};
use itertools::Itertools;
use petgraph::{stable_graph::NodeIndex, visit::Walker};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, error, info, instrument, trace, warn};
use uuid::Uuid;

use crate::{
    error::DockerError,
    events::{DeploymentEvent, EventStatus},
    image::Image,
    network::Network,
    properties::{deployment::AvailableDeployment, AvailableProp, Client},
    requests::{
        deployment::{CommandValue, DeploymentCommand, DeploymentUpdate},
        ReqError,
    },
    resource::{
        container::ContainerResource, deployment::Deployment, Context, Create, Resource,
        ResourceError, State,
    },
    store::{StateStore, StoreError},
    volume::Volume,
    Docker,
};

use self::events::AstarteEvent;

pub mod events;
pub(crate) mod resource;

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
    /// couldn't {ctx} resource {id}, because it's missing
    Missing {
        /// Operation where the error originated
        ctx: &'static str,
        /// Id of the resource that is missing
        id: Id,
    },
    /// relation is missing given the index
    MissingRelation,
    /// couldn't process request
    Request(#[from] ReqError),
    /// store operation failed
    Store(#[from] StoreError),
    /// couldn't complete operation on a resource
    Resource(#[from] ResourceError),
    /// couldn't parse id, it's an invalid UUID
    Uuid {
        /// The invalid [`Id`]
        id: String,
        /// Error with the reason it's invalid
        #[source]
        source: uuid::Error,
    },
    /// BUG couldn't convert missing node
    BugMissing,
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
    #[must_use]
    pub fn new(client: Docker, store: StateStore, device: D) -> (Self, ServiceHandle<D>)
    where
        D: Clone,
    {
        let (tx, rx) = mpsc::unbounded_channel();

        // Clone the store lazily since the service handle will only write to the database.
        let handle = ServiceHandle::new(tx, device.clone(), store.clone_lazy());

        let service = Self {
            client,
            device,
            events: rx,
            store,
        };

        (service, handle)
    }

    fn context(&mut self, id: impl Into<Uuid>) -> Context<D> {
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
        D: Client + Sync + 'static,
    {
        self.publish_received().await?;

        self.start_deployments().await?;
        self.stop_deployments().await?;
        self.delete_deployments().await?;

        info!("init completed");

        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn publish_received(&mut self) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        for id in self.store.unpublished_images().await? {
            Image::publish(self.context(id)).await?;
        }

        for id in self.store.unpublished_volumes().await? {
            Volume::publish(self.context(id)).await?;
        }

        for id in self.store.unpublished_networks().await? {
            Network::publish(self.context(id)).await?;
        }

        for id in self.store.unpublished_containers().await? {
            ContainerResource::publish(self.context(id)).await?;
        }

        for id in self
            .store
            .deployments_in(DeploymentStatus::Received)
            .await?
        {
            Deployment::publish(self.context(id)).await?;
        }

        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn start_deployments(&mut self) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        for id in self.store.deployments_in(DeploymentStatus::Started).await? {
            self.start(*id).await;
        }

        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn stop_deployments(&mut self) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        for id in self.store.deployments_in(DeploymentStatus::Stopped).await? {
            self.stop(*id).await;
        }

        Ok(())
    }

    #[instrument(skip_all)]
    pub async fn delete_deployments(&mut self) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        for id in self.store.deployments_in(DeploymentStatus::Deleted).await? {
            self.delete(*id).await;
        }

        Ok(())
    }

    /// Blocking call that will handle the events from Astarte and the containers.
    #[instrument(skip_all)]
    pub async fn handle_events(&mut self)
    where
        D: Client + Sync + 'static,
    {
        while let Some(event) = self.events.recv().await {
            self.on_event(event).await;
        }

        info!("event receiver disconnected");
    }

    #[instrument(skip_all)]
    async fn on_event(&mut self, event: AstarteEvent)
    where
        D: Client + Sync + 'static,
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
            AstarteEvent::DeploymentUpdate(DeploymentUpdate { from, to }) => {
                self.update(from, to).await;
            }
        }
    }

    #[instrument(skip_all, fields(%id))]
    async fn resource_req(&mut self, id: Id, deployment: Option<Uuid>)
    where
        D: Client + Sync + 'static,
    {
        // TODO: error handling with a DeploymentEvent
        let res = match id.resource_type() {
            ResourceType::Image => Image::publish(self.context(*id.uuid())).await,
            ResourceType::Volume => Volume::publish(self.context(*id.uuid())).await,
            ResourceType::Network => Network::publish(self.context(*id.uuid())).await,
            ResourceType::Container => ContainerResource::publish(self.context(*id.uuid())).await,
            ResourceType::Deployment => Deployment::publish(self.context(*id.uuid())).await,
        };

        if let Err(err) = res {
            let error = format!("{:#}", eyre::Report::new(err));
            error!(error, "failed to create resource");

            if let Some(deployment_id) = deployment {
                DeploymentEvent::new(EventStatus::Error, error)
                    .send(&deployment_id, &self.device)
                    .await;
            }
        }
    }

    /// Will start a [`Deployment`]
    #[instrument(skip(self))]
    pub async fn start(&mut self, id: Uuid)
    where
        D: Client + Sync + 'static,
    {
        let deployment = match self.store.complete_deployment(id).await {
            Ok(Some(deployment)) => deployment,
            Ok(None) => {
                error!("{id} not found");

                DeploymentEvent::new(EventStatus::Error, format!("{id} not found"))
                    .send(&id, &self.device)
                    .await;

                return;
            }
            Err(err) => {
                let err = format!("{:#}", eyre::Report::new(err));

                error!(error = err, "couldn't start deployment");

                DeploymentEvent::new(EventStatus::Error, err)
                    .send(&id, &self.device)
                    .await;

                return;
            }
        };

        info!("starting deployment");

        DeploymentEvent::new(EventStatus::Starting, "")
            .send(&id, &self.device)
            .await;

        if let Err(err) = self.start_deployment(id, deployment).await {
            let err = format!("{:#}", eyre::Report::new(err));

            error!(error = err, "couldn't start deployment");

            DeploymentEvent::new(EventStatus::Error, err)
                .send(&id, &self.device)
                .await;
        }
    }

    async fn start_deployment(&mut self, deployment_id: Uuid, deployment: Deployment) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        for id in deployment.images {
            Image::up(self.context(id)).await?;
        }

        for id in deployment.volumes {
            Volume::up(self.context(id)).await?;
        }

        for id in deployment.networks {
            Network::up(self.context(id)).await?;
        }

        for id in deployment.containers {
            let mut container = ContainerResource::up(self.context(id)).await?;

            container.start(self.context(id)).await?;
        }

        AvailableDeployment::new(&deployment_id)
            .send(
                &self.device,
                crate::properties::deployment::DeploymentStatus::Started,
            )
            .await;

        Ok(())
    }

    /// Will stop an application
    #[instrument(skip(self))]
    pub async fn stop(&mut self, id: Uuid)
    where
        D: Client + Sync + 'static,
    {
        let containers = match self.store.deployment_containers(id).await {
            Ok(Some(containers)) => containers,
            Ok(None) => {
                error!("{id} not found");

                DeploymentEvent::new(EventStatus::Error, format!("{id} not found"))
                    .send(&id, &self.device)
                    .await;

                return;
            }
            Err(err) => {
                let err = format!("{:#}", eyre::Report::new(err));

                error!(error = err, "couldn't start deployment");

                DeploymentEvent::new(EventStatus::Error, err)
                    .send(&id, &self.device)
                    .await;

                return;
            }
        };

        info!("starting deployment");

        DeploymentEvent::new(EventStatus::Stopping, "")
            .send(&id, &self.device)
            .await;

        if let Err(err) = self.stop_deployment(id, containers).await {
            let err = format!("{:#}", eyre::Report::new(err));

            error!(error = err, "couldn't stop deployment");

            DeploymentEvent::new(EventStatus::Error, err)
                .send(&id, &self.device)
                .await;
        }
    }

    async fn stop_deployment(&mut self, deployment: Uuid, containers: Vec<Uuid>) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        for id in containers {
            let mut ctx = self.context(id);
            let (state, mut container) = ContainerResource::fetch(&mut ctx).await?;

            match state {
                State::Missing => {
                    warn!(%id, "contaienr already missing, cannot stop");

                    continue;
                }
                State::Created => {}
            }

            container.stop(ctx).await?;
        }

        AvailableDeployment::new(&deployment)
            .send(
                &self.device,
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
        D: Client + Sync + 'static,
    {
        let deployment = match self.store.complete_deployment(id).await {
            Ok(Some(deployment)) => deployment,
            Ok(None) => {
                error!("{id} not found");

                DeploymentEvent::new(EventStatus::Error, format!("{id} not found"))
                    .send(&id, &self.device)
                    .await;

                return;
            }
            Err(err) => {
                let err = format!("{:#}", eyre::Report::new(err));

                error!(error = err, "couldn't start deployment");

                DeploymentEvent::new(EventStatus::Error, err)
                    .send(&id, &self.device)
                    .await;

                return;
            }
        };

        info!("starting deployment");

        DeploymentEvent::new(EventStatus::Deleting, "")
            .send(&id, &self.device)
            .await;

        if let Err(err) = self.delete_deployment(id, deployment).await {
            let err = format!("{:#}", eyre::Report::new(err));

            error!(error = err, "couldn't delete deployment");

            DeploymentEvent::new(EventStatus::Error, err)
                .send(&id, &self.device)
                .await;
        }
    }

    // FIXME: remove only unused resources
    async fn delete_deployment(&mut self, deployment_id: Uuid, deployment: Deployment) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        for id in deployment.containers {
            ContainerResource::down(self.context(id)).await?;
        }

        for id in deployment.volumes {
            Volume::down(self.context(id)).await?;
        }

        for id in deployment.networks {
            Network::down(self.context(id)).await?;
        }

        for id in deployment.images {
            Image::down(self.context(id)).await?;
        }

        AvailableDeployment::new(&deployment_id)
            .unset(&self.device)
            .await
            .map_err(ResourceError::from)?;

        self.store.delete_deployment(deployment_id).await?;

        Ok(())
    }

    /// Will update an application between deployments
    #[instrument(skip(self))]
    pub async fn update(&mut self, from: Uuid, to: Uuid)
    where
        D: Client + Sync + 'static,
    {
        let from = Id::new(ResourceType::Deployment, from);
        let to = Id::new(ResourceType::Deployment, to);

        DeploymentEvent::new(EventStatus::Updating, "")
            .send(from.uuid(), &self.device)
            .await;

        // TODO: consider if it's necessary re-start the `from` containers or a retry logic
        if let Err(err) = self.update_deployment(from, to).await {
            let err = format!("{:#}", eyre::Report::new(err));

            error!(error = err, "couldn't update deployment");

            DeploymentEvent::new(EventStatus::Error, err)
                .send(from.uuid(), &self.device)
                .await;
        }
    }

    async fn update_deployment(&mut self, from: Id, to: Id) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let to_deployment = self.nodes.node(&to).ok_or(ServiceError::Missing {
            ctx: "update",
            id: to,
        })?;

        let space = petgraph::visit::DfsPostOrder::new(self.nodes.relations(), to_deployment.idx);
        let to_start_ids = space
            .iter(self.nodes.relations())
            .map(|idx| {
                self.nodes
                    .get_id(idx)
                    .copied()
                    .ok_or(ServiceError::MissingRelation)
            })
            .collect::<Result<HashSet<Id>>>()?;

        let from_deployment = self.nodes.node(&from).ok_or(ServiceError::Missing {
            ctx: "update",
            id: to,
        })?;

        let space = petgraph::visit::DfsPostOrder::new(self.nodes.relations(), from_deployment.idx);
        let to_stop_ids = space
            .iter(self.nodes.relations())
            .filter_map(|idx| {
                let Some(id) = self.nodes.get_id(idx) else {
                    return Some(Err(ServiceError::MissingRelation));
                };

                // Skip container in the start set
                if to_start_ids.contains(id) {
                    trace!("{id} present in the update");

                    return None;
                }

                Some(Ok(*id))
            })
            .collect::<Result<Vec<Id>>>()?;

        debug!("stopping {} containers", to_stop_ids.len());

        for id in to_stop_ids {
            let node = self
                .nodes
                .node_mut(&id)
                .ok_or_else(|| ServiceError::Missing { id, ctx: "update" })?;

            node.stop(&self.device, &self.client).await?;
        }

        for id in to_start_ids {
            let node = self
                .nodes
                .node_mut(&id)
                .ok_or_else(|| ServiceError::Missing { id, ctx: "update" })?;

            node.up(&self.device, &self.client).await?;
        }

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

    /// Returns `true` if the [`ResourceType`] is a [`Deployment`]
    ///
    /// [`Deployment`]: ResourceType::Deployment
    fn is_deployment(&self) -> bool {
        matches!(self.rt, ResourceType::Deployment)
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

    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk::FromEvent;
    use astarte_device_sdk_mock::mockall::Sequence;
    use astarte_device_sdk_mock::MockDeviceClient;
    use edgehog_store::db;
    use pretty_assertions::assert_eq;
    use resource::{NodeResource, NodeType};
    use tempfile::TempDir;
    use uuid::uuid;

    use crate::container::{Binding, Container, PortBindingMap};
    use crate::network::Network;
    use crate::properties::container::ContainerStatus;
    use crate::properties::deployment::DeploymentStatus;
    use crate::requests::container::tests::create_container_request_event;
    use crate::requests::container::RestartPolicy;
    use crate::requests::deployment::tests::create_deployment_request_event;
    use crate::requests::image::tests::create_image_request_event;
    use crate::requests::network::tests::create_network_request_event;
    use crate::requests::volume::tests::create_volume_request_event;
    use crate::requests::ContainerRequest;
    use crate::volume::Volume;
    use crate::{docker, docker_mock};

    use super::*;

    async fn mock_service(
        tempdir: &TempDir,
        client: Docker,
        device: MockDeviceClient<SqliteStore>,
    ) -> (
        Service<MockDeviceClient<SqliteStore>>,
        ServiceHandle<MockDeviceClient<SqliteStore>>,
    ) {
        let db_file = tempdir.path().join("state.db");
        let db_file = db_file.to_str().unwrap();

        let handle = db::Handle::open(db_file).await.unwrap();
        let store = StateStore::new(handle);

        Service::new(client, store, device)
    }

    #[tokio::test]
    async fn should_add_an_image() {
        let tmpdir = TempDir::new().unwrap();

        let id = uuid!("5b705c7b-e6c7-4455-ba9b-a081be020c43");

        let client = Docker::connect().await.unwrap();
        let mut device = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        let image_path = format!("/{id}/pulled");
        device
            .expect_send::<bool>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableImages"
                    && path == (image_path)
                    && !*value
            })
            .returning(|_, _, _| Ok(()));

        let (mut service, handle) = mock_service(&tmpdir, client, device).await;

        let reference = "docker.io/nginx:stable-alpine-slim";
        let create_image_req = create_image_request_event(id.to_string(), reference, "");

        let req = ContainerRequest::from_event(create_image_req).unwrap();

        handle.on_event(req).await.unwrap();

        let event = service.events.recv().await.unwrap();
        service.on_event(event).await;

        let id = Id::new(ResourceType::Image, id);
        let node = service.nodes.node(&id).unwrap();

        let Some(NodeResource {
            value: NodeType::Image(image),
            ..
        }) = &node.resource
        else {
            panic!("incorrect node {node:?}");
        };

        let exp = Image {
            id: None,
            reference: reference.to_string(),
            registry_auth: None,
        };

        assert_eq!(*image, exp);
    }

    #[tokio::test]
    async fn should_add_a_volume() {
        let tempdir = TempDir::new().unwrap();

        let id = uuid!("e605c1bf-a168-4878-a7cb-41a57847bbca");

        let client = Docker::connect().await.unwrap();
        let mut device = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        let endpoint = format!("/{id}/created");
        device
            .expect_send::<bool>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableVolumes"
                    && path == (endpoint)
                    && !*value
            })
            .returning(|_, _, _| Ok(()));

        let (mut service, handle) = mock_service(&tempdir, client, device).await;

        let create_volume_req = create_volume_request_event(id, "local", &["foo=bar", "some="]);

        let req = ContainerRequest::from_event(create_volume_req).unwrap();

        handle.on_event(req).await.unwrap();
        let event = service.events.recv().await.unwrap();
        service.on_event(event).await;

        let id = Id::new(ResourceType::Volume, id);
        let node = service.nodes.node(&id).unwrap();

        let Some(NodeResource {
            value: NodeType::Volume(volume),
            ..
        }) = &node.resource
        else {
            panic!("incorrect node {node:?}");
        };

        let name = id.uuid().to_string();
        let exp = Volume {
            name: name.as_str(),
            driver: "local",
            driver_opts: HashMap::from([("foo".to_string(), "bar"), ("some".to_string(), "")]),
        };

        assert_eq!(*volume, exp);
    }

    #[tokio::test]
    async fn should_add_a_network() {
        let tempdir = TempDir::new().unwrap();

        let id = uuid!("e605c1bf-a168-4878-a7cb-41a57847bbca");

        let client = Docker::connect().await.unwrap();
        let mut device = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        let endpoint = format!("/{id}/created");
        device
            .expect_send::<bool>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableNetworks"
                    && path == (endpoint)
                    && !*value
            })
            .returning(|_, _, _| Ok(()));

        let (mut service, handle) = mock_service(&tempdir, client, device).await;

        let create_network_req = create_network_request_event(id, "bridged", &[]);

        let req = ContainerRequest::from_event(create_network_req).unwrap();

        handle.on_event(req).await.unwrap();

        let event = service.events.recv().await.unwrap();
        service.on_event(event).await;

        let id = Id::new(ResourceType::Network, id);
        let node = service.nodes.node(&id).unwrap();

        let Some(NodeResource {
            value: NodeType::Network(network),
            ..
        }) = &node.resource
        else {
            panic!("incorrect node {node:?}");
        };

        let id = id.uuid().to_string();
        let exp = Network {
            id: None,
            name: id.as_str(),
            driver: "bridged",
            internal: false,
            enable_ipv6: false,
            driver_opts: HashMap::new(),
        };

        assert_eq!(*network, exp);
    }

    #[tokio::test]
    async fn should_add_a_container() {
        let tempdir = TempDir::new().unwrap();

        let id = uuid!("e605c1bf-a168-4878-a7cb-41a57847bbca");

        let client = Docker::connect().await.unwrap();
        let mut device = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        let endpoint = format!("/{id}/status");
        device
            .expect_send::<ContainerStatus>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableContainers"
                    && path == endpoint
                    && *value == ContainerStatus::Received
            })
            .returning(|_, _, _| Ok(()));

        let (mut service, handle) = mock_service(&tempdir, client, device).await;

        let image_id = Uuid::new_v4().to_string();
        let create_container_req = create_container_request_event(
            id,
            &image_id,
            "image",
            &["9808bbd5-2e81-4f99-83e7-7cc60623a196"],
        );

        let req = ContainerRequest::from_event(create_container_req).unwrap();

        handle.on_event(req).await.unwrap();

        let event = service.events.recv().await.unwrap();
        service.on_event(event).await;

        let id = Id::new(ResourceType::Container, id);
        let node = service.nodes.node(&id).unwrap();

        let Some(NodeResource {
            value: NodeType::Container(container),
            ..
        }) = &node.resource
        else {
            panic!("incorrect node {node:?}");
        };

        let id = id.uuid().to_string();
        let exp = Container {
            id: None,
            name: id.as_str(),
            image: "image",
            network_mode: "bridge",
            networks: vec!["9808bbd5-2e81-4f99-83e7-7cc60623a196"],
            hostname: Some("hostname"),
            restart_policy: RestartPolicy::No,
            env: vec!["env"],
            binds: vec!["binds"],
            port_bindings: PortBindingMap::<&str>(HashMap::from_iter([(
                "80/tcp".to_string(),
                vec![Binding {
                    host_ip: None,
                    host_port: Some(80),
                }],
            )])),
            privileged: false,
        };

        assert_eq!(*container, exp);
    }

    #[tokio::test]
    async fn should_start_deployment() {
        let tempdir = TempDir::new().unwrap();

        let image_id = Uuid::new_v4();
        let container_id = Uuid::new_v4();
        let deployment_id = Uuid::new_v4();

        let reference = "docker.io/nginx:stable-alpine-slim";

        let client = docker_mock!(docker::Client::connect_with_local_defaults().unwrap(), {
            use futures::StreamExt;

            let mut mock = docker::Client::new();
            let mut seq = mockall::Sequence::new();

            mock.expect_create_image()
                .withf(move |option, _, _| {
                    option
                        .as_ref()
                        .is_some_and(|opt| opt.from_image == reference)
                })
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

            let name = container_id.to_string();
            mock.expect_create_container()
                .withf(move |option, config| {
                    option.as_ref().is_some_and(|opt| opt.name == name)
                        && config.image == Some(reference)
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
        let mut device = MockDeviceClient::<SqliteStore>::new();
        let mut seq = Sequence::new();

        let image_path = format!("/{image_id}/pulled");
        device
            .expect_send::<bool>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableImages"
                    && path == (image_path)
                    && !*value
            })
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{container_id}/status");
        device
            .expect_send::<ContainerStatus>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableContainers"
                    && path == endpoint
                    && *value == ContainerStatus::Received
            })
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{deployment_id}/status");
        device
            .expect_send::<DeploymentStatus>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableDeployments"
                    && path == endpoint
                    && *value == DeploymentStatus::Stopped
            })
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{deployment_id}");
        device
            .expect_send_object::<DeploymentEvent>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.DeploymentEvent"
                    && path == endpoint
                    && value.status == EventStatus::Starting
            })
            .returning(|_, _, _| Ok(()));

        let image_path = format!("/{image_id}/pulled");
        device
            .expect_send::<bool>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableImages"
                    && path == (image_path)
                    && *value
            })
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{container_id}/status");
        device
            .expect_send::<ContainerStatus>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableContainers"
                    && path == endpoint
                    && *value == ContainerStatus::Created
            })
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{container_id}/status");
        device
            .expect_send::<ContainerStatus>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableContainers"
                    && path == endpoint
                    && *value == ContainerStatus::Running
            })
            .returning(|_, _, _| Ok(()));

        let endpoint = format!("/{deployment_id}/status");
        device
            .expect_send::<DeploymentStatus>()
            .once()
            .in_sequence(&mut seq)
            .withf(move |interface, path, value| {
                interface == "io.edgehog.devicemanager.apps.AvailableDeployments"
                    && path == endpoint
                    && *value == DeploymentStatus::Started
            })
            .returning(|_, _, _| Ok(()));

        let (mut service, handle) = mock_service(&tempdir, client, device).await;

        let create_image_req = create_image_request_event(image_id.to_string(), reference, "");

        let image_req = ContainerRequest::from_event(create_image_req).unwrap();

        let create_container_req = create_container_request_event(
            container_id,
            &image_id.to_string(),
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

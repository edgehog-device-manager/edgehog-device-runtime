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

//! Service to receive and handle the Astarte events.

use std::{
    fmt::{Debug, Display},
    str::FromStr,
};

use astarte_device_sdk::{event::FromEventError, properties::PropAccess, DeviceEvent, FromEvent};
use itertools::Itertools;
use petgraph::stable_graph::NodeIndex;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use crate::{
    container::Container,
    error::DockerError,
    events::{DeploymentEvent, EventError, EventStatus},
    image::Image,
    network::Network,
    properties::{Client, PropError},
    requests::{
        container::CreateContainer, deployment::CreateDeployment, image::CreateImage,
        network::CreateNetwork, volume::CreateVolume, CreateRequests, ReqError,
    },
    service::resource::NodeType,
    store::{Resource, StateStore, StateStoreError},
    volume::Volume,
    Docker,
};

use self::{collection::NodeGraph, node::Node, state::State};

pub(crate) mod collection;
pub(crate) mod node;
pub(crate) mod resource;
pub(crate) mod state;

type Result<T> = std::result::Result<T, ServiceError>;

/// Error from the [`Service`].
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum ServiceError {
    /// error converting event
    FromEvent(#[from] FromEventError),
    /// docker operation failed
    Docker(#[source] DockerError),
    /// couldn't save the property
    Prop(#[from] PropError),
    /// node {0} is missing
    MissingNode(Id),
    /// relation is missing given the index
    MissingRelation,
    /// couldn't process request
    Request(#[from] ReqError),
    /// couldn't create for missing node {0}
    Create(Id),
    /// couldn't start for missing node {0}
    Start(Id),
    /// couldn't operate on missing node {0}
    Missing(Id),
    /// state store operation failed
    StateStore(#[from] StateStoreError),
    /// couldn't send the event to Astarte
    Event(#[from] EventError),
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
pub struct Service<D>
where
    D: Client + PropAccess,
{
    client: Docker,
    store: StateStore,
    device: D,
    nodes: NodeGraph,
}

impl<D> Service<D>
where
    D: Client + PropAccess + Sync,
{
    /// Create a new service
    #[must_use]
    pub(crate) fn new(client: Docker, store: StateStore, device: D) -> Self {
        Self {
            client,
            store,
            device,
            nodes: NodeGraph::new(),
        }
    }

    /// Initialize the service, it will load all the already stored properties
    #[instrument(skip_all)]
    pub async fn init(client: Docker, store: StateStore, device: D) -> Result<Self> {
        let mut service = Self::new(client, store, device);

        let stored = service.store.load().await?;

        debug!("loaded {} resources from state store", stored.len());

        for value in stored {
            let id = value.id;

            debug!("adding stored {id}");

            match value.resource {
                Some(Resource::Image(state)) => {
                    let image = Image::from(state);

                    service.nodes.add_node_sync(id, &[], |id, node_idx| {
                        Ok(Node::with_state(id, node_idx, State::Stored(image.into())))
                    })?;
                }
                Some(Resource::Volume(state)) => {
                    let volume = Volume::from(state);

                    service.nodes.add_node_sync(id, &[], |id, node_idx| {
                        Ok(Node::with_state(id, node_idx, State::Stored(volume.into())))
                    })?;
                }
                Some(Resource::Network(state)) => {
                    let network = Network::from(state);

                    service.nodes.add_node_sync(id, &[], |id, node_idx| {
                        Ok(Node::with_state(
                            id,
                            node_idx,
                            State::Stored(network.into()),
                        ))
                    })?;
                }
                Some(Resource::Container(state)) => {
                    let container = Container::from(state);

                    service
                        .nodes
                        .add_node_sync(id, &value.deps, |id, node_idx| {
                            Ok(Node::with_state(
                                id,
                                node_idx,
                                State::Stored(container.into()),
                            ))
                        })?;
                }
                Some(Resource::Deployment) => {
                    service
                        .nodes
                        .add_node_sync(id, &value.deps, |id, node_idx| {
                            Ok(Node::with_state(
                                id,
                                node_idx,
                                State::Stored(NodeType::Deployment),
                            ))
                        })?;
                }
                None => {
                    debug!("adding missing resource");

                    service
                        .nodes
                        .add_node_sync(id, &value.deps, |id, node_idx| {
                            Ok(Node::new(id, node_idx))
                        })?;
                }
            }
        }

        Ok(service)
    }

    /// Handles an event from the image.
    #[instrument(skip_all)]
    pub async fn on_event(&mut self, event: DeviceEvent) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let event = CreateRequests::from_event(event)?;

        match event {
            CreateRequests::Image(req) => {
                self.create_image(req).await?;
            }
            CreateRequests::Volume(req) => {
                self.create_volume(req).await?;
            }
            CreateRequests::Network(req) => {
                self.create_network(req).await?;
            }
            CreateRequests::Container(req) => {
                self.create_container(req).await?;
            }
            CreateRequests::Deployment(req) => {
                self.create_deployment(req).await?;
            }
        }

        self.store.store(&self.nodes).await?;

        Ok(())
    }

    /// Store the create image request
    #[instrument(skip_all)]
    async fn create_image(&mut self, req: CreateImage) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let id = Id::try_from_str(ResourceType::Image, &req.id)?;

        debug!("creating image with id {id}");

        let device = &self.device;
        let store = &mut self.store;

        self.nodes
            .add_node(id, |id, idx| async move {
                let image = Image::from(req);

                let mut node = Node::new(id, idx);

                node.store(store, device, image, Vec::new()).await?;

                Ok(node)
            })
            .await?;

        Ok(())
    }

    /// Store the create volume request
    #[instrument(skip_all)]
    async fn create_volume(&mut self, req: CreateVolume) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let id = Id::try_from_str(ResourceType::Volume, &req.id)?;

        debug!("creating volume with id {id}");

        let device = &self.device;
        let store = &mut self.store;

        self.nodes
            .add_node(id, |id, idx| async move {
                let image = Volume::try_from(req)?;

                let mut node = Node::new(id, idx);

                node.store(store, device, image, Vec::new()).await?;

                Ok(node)
            })
            .await?;

        Ok(())
    }

    /// Store the create network request
    #[instrument(skip_all)]
    async fn create_network(&mut self, req: CreateNetwork) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let id = Id::try_from_str(ResourceType::Network, &req.id)?;

        debug!("creating network with id {id}");

        let device = &self.device;
        let store = &mut self.store;

        self.nodes
            .add_node(id, |id, idx| async move {
                let network = Network::from(req);

                let mut node = Node::new(id, idx);

                node.store(store, device, network, Vec::new()).await?;

                Ok(node)
            })
            .await?;

        Ok(())
    }

    /// Store the create container request
    #[instrument(skip_all)]
    async fn create_container(&mut self, req: CreateContainer) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let id = Id::try_from_str(ResourceType::Container, &req.id)?;

        debug!("creating container with id {id}");

        let device = &self.device;
        let store = &mut self.store;

        let deps = req.dependencies()?;

        self.nodes
            .add_node_with_deps(id, deps, |id, idx, deps| async move {
                let image = Container::try_from(req)?;

                let mut node = Node::new(id, idx);

                node.store(store, device, image, deps).await?;

                Ok(node)
            })
            .await?;

        Ok(())
    }

    /// Store the create deployment request
    #[instrument(skip_all)]
    async fn create_deployment(&mut self, req: CreateDeployment) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let id = Id::try_from_str(ResourceType::Deployment, &req.id)?;

        debug!("creating deployment with id {id}");

        let device = &self.device;
        let store = &mut self.store;

        let deps = req
            .containers
            .iter()
            .map(|id| Id::try_from_str(ResourceType::Container, id))
            .try_collect()?;

        self.nodes
            .add_node_with_deps(id, deps, |id, idx, deps| async move {
                let mut node = Node::new(id, idx);

                node.store(store, device, NodeType::Deployment, deps)
                    .await?;

                Ok(node)
            })
            .await?;

        Ok(())
    }

    /// Will start an application
    #[instrument(skip(self))]
    pub async fn start(&mut self, id: &Id) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        debug!("starting {id}");

        let node = self
            .nodes
            .node(id)
            .ok_or_else(|| ServiceError::MissingNode(*id))?;

        if node.is_deployment() {
            DeploymentEvent::new(EventStatus::Starting, "")
                .send(node.id.uuid(), &self.device)
                .await?;
        } else {
            warn!("starting a {node:?} and not a deployment");
        }

        let idx = node.idx;

        let res = self.start_node(idx).await;

        if let Err(err) = &res {
            DeploymentEvent::new(EventStatus::Error, err.to_string())
                .send(id.uuid(), &self.device)
                .await?;
        }

        res
    }

    async fn start_node(&mut self, idx: NodeIndex) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let mut space = petgraph::visit::DfsPostOrder::new(self.nodes.relations(), idx);

        let mut relations = Vec::new();
        while let Some(idx) = space.next(self.nodes.relations()) {
            let id = *self
                .nodes
                .get_id(idx)
                .ok_or(ServiceError::MissingRelation)?;

            relations.push(id);
        }

        for id in relations {
            let node = self
                .nodes
                .node_mut(&id)
                .ok_or_else(|| ServiceError::MissingNode(id))?;

            node.up(&self.device, &self.client).await?;

            self.store.store(&self.nodes).await?;
        }

        Ok(())
    }

    /// Will stop an application
    #[instrument(skip(self))]
    pub async fn stop(&mut self, id: &Id) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        debug!("stopping {id}");

        let node = self
            .nodes
            .node(id)
            .ok_or_else(|| ServiceError::MissingNode(*id))?;

        if node.is_deployment() {
            DeploymentEvent::new(EventStatus::Stopping, "")
                .send(id.uuid(), &self.device)
                .await?;
        } else {
            warn!("stopping a {node:?} and not a deployment");
        }

        let idx = node.idx;

        let res = self.stop_node(idx).await;

        if let Err(err) = &res {
            DeploymentEvent::new(EventStatus::Error, err.to_string())
                .send(id.uuid(), &self.device)
                .await?;
        }

        res
    }

    async fn stop_node(&mut self, start_idx: NodeIndex) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let mut space = petgraph::visit::DfsPostOrder::new(self.nodes.relations(), start_idx);

        let mut relations = Vec::new();
        let graph = self.nodes.relations();
        while let Some(idx) = space.next(graph) {
            if idx == start_idx {
                // The deployment stopped should be the last event.
                debug!("skipping the starting node");
                continue;
            }

            // filter the dependents deployment, and check that are not started
            let running_deployments = self
                .nodes
                .dependent(idx)
                .filter_map(|id| {
                    if id.is_deployment() {
                        self.nodes.node(id)
                    } else {
                        None
                    }
                })
                .any(|deployment| deployment.is_up());

            if running_deployments {
                debug!(
                    "skipping {:?} which has another running deployment ",
                    self.nodes.get_id(idx)
                );

                continue;
            }

            let id = *self
                .nodes
                .get_id(idx)
                .ok_or(ServiceError::MissingRelation)?;

            relations.push(id);
        }

        let start_id = *self
            .nodes
            .get_id(start_idx)
            .ok_or(ServiceError::MissingRelation)?;
        // Push the deployment last
        relations.push(start_id);

        for id in relations {
            let node = self
                .nodes
                .node_mut(&id)
                .ok_or_else(|| ServiceError::MissingNode(id))?;

            node.stop(&self.device, &self.client).await?;

            self.store.store(&self.nodes).await?;
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

    pub(crate) fn try_from_str(rt: ResourceType, id: &str) -> Result<Self> {
        let id = Uuid::from_str(id).map_err(|err| ServiceError::Uuid {
            id: id.to_string(),
            source: err,
        })?;

        Ok(Self { rt, id })
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
    use astarte_device_sdk_mock::mockall::Sequence;
    use astarte_device_sdk_mock::MockDeviceClient;
    use bollard::secret::RestartPolicyNameEnum;
    use pretty_assertions::assert_eq;
    use resource::NodeType;
    use tempfile::TempDir;
    use uuid::uuid;

    use crate::container::{Binding, PortBindingMap};
    use crate::properties::container::ContainerStatus;
    use crate::requests::container::tests::create_container_request_event;
    use crate::requests::image::tests::create_image_request_event;
    use crate::requests::network::tests::create_network_request_event;
    use crate::requests::volume::tests::create_volume_request_event;

    use super::*;

    #[tokio::test]
    async fn should_add_an_image() {
        let tempdir = TempDir::new().unwrap();

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

        let store = StateStore::open(tempdir.path().join("state.json"))
            .await
            .unwrap();

        let mut service = Service::new(client, store, device);

        let reference = "docker.io/nginx:stable-alpine-slim";
        let create_image_req = create_image_request_event(id.to_string(), reference, "");

        service.on_event(create_image_req).await.unwrap();

        let id = Id::new(ResourceType::Image, id);
        let node = service.nodes.node(&id).unwrap();

        let State::Stored(NodeType::Image(image)) = node.state() else {
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

        let store = StateStore::open(tempdir.path().join("state.json"))
            .await
            .unwrap();

        let mut service = Service::new(client, store, device);

        let create_volume_req = create_volume_request_event(id, "local", &["foo=bar", "some="]);

        service.on_event(create_volume_req).await.unwrap();

        let id = Id::new(ResourceType::Volume, id);
        let node = service.nodes.node(&id).unwrap();

        let State::Stored(NodeType::Volume(volume)) = node.state() else {
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

        let store = StateStore::open(tempdir.path().join("state.json"))
            .await
            .unwrap();

        let mut service = Service::new(client, store, device);

        let create_network_req = create_network_request_event(id, "bridged");

        service.on_event(create_network_req).await.unwrap();

        let id = Id::new(ResourceType::Network, id);
        let node = service.nodes.node(&id).unwrap();

        let State::Stored(NodeType::Network(network)) = node.state() else {
            panic!("incorrect node {node:?}");
        };

        let id = id.uuid().to_string();
        let exp = Network {
            id: None,
            name: id.as_str(),
            driver: "bridged",
            check_duplicate: false,
            internal: false,
            enable_ipv6: false,
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

        let store = StateStore::open(tempdir.path().join("state.json"))
            .await
            .unwrap();

        let mut service = Service::new(client, store, device);

        let image_id = Uuid::new_v4().to_string();
        let create_container_req = create_container_request_event(id, &image_id);

        service.on_event(create_container_req).await.unwrap();

        let id = Id::new(ResourceType::Container, id);
        let node = service.nodes.node(&id).unwrap();

        let State::Stored(NodeType::Container(container)) = node.state() else {
            panic!("incorrect node {node:?}");
        };

        let id = id.uuid().to_string();
        let exp = Container {
            id: None,
            name: id.as_str(),
            image: "image",
            networks: vec!["networks"],
            hostname: Some("hostname"),
            restart_policy: RestartPolicyNameEnum::NO,
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
}

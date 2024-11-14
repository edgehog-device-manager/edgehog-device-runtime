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

use astarte_device_sdk::event::FromEventError;
use itertools::Itertools;
use petgraph::{stable_graph::NodeIndex, visit::Walker};
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};
use uuid::Uuid;

use crate::{
    container::Container,
    error::DockerError,
    events::{DeploymentEvent, EventStatus},
    image::Image,
    network::Network,
    properties::Client,
    requests::{
        container::CreateContainer,
        deployment::{CommandValue, CreateDeployment, DeploymentCommand},
        image::CreateImage,
        network::CreateNetwork,
        volume::CreateVolume,
        ContainerRequest, ReqError,
    },
    service::resource::NodeType,
    store::{StateStore, StateStoreError},
    volume::Volume,
    Docker,
};

use self::collection::NodeGraph;
use self::resource::NodeResource;

pub(crate) mod collection;
pub(crate) mod node;
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
    /// state store operation failed
    StateStore(#[from] StateStoreError),
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
    store: StateStore,
    device: D,
    nodes: NodeGraph,
}

impl<D> Service<D> {
    /// Create a new service
    #[must_use]
    pub fn new(client: Docker, store: StateStore, device: D) -> Self {
        Self {
            client,
            store,
            device,
            nodes: NodeGraph::new(),
        }
    }

    /// Initialize the service, it will load all the already stored properties
    #[instrument(skip_all)]
    pub async fn init(&mut self) -> Result<()> {
        let stored = self.store.load().await?;

        debug!("loaded {} resources from state store", stored.len());

        for value in stored {
            let id = value.id;

            debug!("adding stored {id}");

            match value.resource {
                Some(resource) => {
                    let node = NodeType::from(resource);

                    self.nodes
                        .get_or_insert(id, NodeResource::stored(node), &value.deps);
                }
                None => {
                    debug!("adding missing resource");

                    debug_assert!(value.deps.is_empty());

                    self.nodes.get_or_add_missing(id);
                }
            }
        }

        Ok(())
    }

    /// Handles an event from the image.
    #[instrument(skip_all)]
    pub async fn on_event(&mut self, event: ContainerRequest) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        match event {
            ContainerRequest::Image(req) => {
                self.create_image(req).await?;
            }
            ContainerRequest::Volume(req) => {
                self.create_volume(req).await?;
            }
            ContainerRequest::Network(req) => {
                self.create_network(req).await?;
            }
            ContainerRequest::Container(req) => {
                self.create_container(req).await?;
            }
            ContainerRequest::Deployment(req) => {
                self.create_deployment(req).await?;
            }
            ContainerRequest::DeploymentCommand(DeploymentCommand {
                id,
                command: CommandValue::Start,
            }) => {
                let id = Id::new(ResourceType::Deployment, id);

                self.start(&id).await?;
            }
            ContainerRequest::DeploymentCommand(DeploymentCommand {
                id,
                command: CommandValue::Stop,
            }) => {
                let id = Id::new(ResourceType::Deployment, id);

                self.stop(&id).await?;
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

        let image = Image::from(req);

        let node = self
            .nodes
            .get_or_insert(id, NodeResource::with_default(image.into()), &[]);

        node.store(store, device, Vec::new()).await?;

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

        let volume = Volume::try_from(req)?;
        let node = self
            .nodes
            .get_or_insert(id, NodeResource::with_default(volume.into()), &[]);

        node.store(store, device, Vec::new()).await?;

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

        let network = Network::from(req);
        let node = self
            .nodes
            .get_or_insert(id, NodeResource::with_default(network.into()), &[]);

        node.store(store, device, Vec::new()).await?;

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

        let container = Container::try_from(req)?;
        let node =
            self.nodes
                .get_or_insert(id, NodeResource::with_default(container.into()), &deps);

        node.store(store, device, deps).await?;

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

        let deps: Vec<Id> = req
            .containers
            .iter()
            .map(|id| Id::try_from_str(ResourceType::Container, id))
            .try_collect()?;

        let node =
            self.nodes
                .get_or_insert(id, NodeResource::with_default(NodeType::Deployment), &deps);

        node.store(store, device, deps).await?;

        Ok(())
    }

    /// Will start an application
    #[instrument(skip(self))]
    pub async fn start(&mut self, id: &Id) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        debug!("starting {id}");

        let node = self.nodes.node(id).ok_or_else(|| ServiceError::Missing {
            id: *id,
            ctx: "start",
        })?;

        if id.is_deployment() {
            DeploymentEvent::new(EventStatus::Starting, "")
                .send(node.id.uuid(), &self.device)
                .await;
        } else {
            warn!("starting a {node:?} and not a deployment");
        }

        let idx = node.idx;

        let res = self.start_node(idx).await;

        if let Err(err) = &res {
            DeploymentEvent::new(EventStatus::Error, err.to_string())
                .send(id.uuid(), &self.device)
                .await;
        }

        res
    }

    async fn start_node(&mut self, idx: NodeIndex) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let space = petgraph::visit::DfsPostOrder::new(self.nodes.relations(), idx);

        let relations = space
            .iter(self.nodes.relations())
            .map(|idx| {
                self.nodes
                    .get_id(idx)
                    .copied()
                    .ok_or(ServiceError::MissingRelation)
            })
            .collect::<Result<Vec<Id>>>()?;

        for id in relations {
            let node = self
                .nodes
                .node_mut(&id)
                .ok_or_else(|| ServiceError::Missing {
                    id,
                    ctx: "start node",
                })?;

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

        let node = self.nodes.node(id).ok_or_else(|| ServiceError::Missing {
            id: *id,
            ctx: "stop",
        })?;

        if id.is_deployment() {
            DeploymentEvent::new(EventStatus::Stopping, "")
                .send(id.uuid(), &self.device)
                .await;
        } else {
            warn!("stopping a {node:?} and not a deployment");
        }

        let idx = node.idx;

        let res = self.stop_node(node.id, idx).await;

        if let Err(err) = &res {
            DeploymentEvent::new(EventStatus::Error, err.to_string())
                .send(id.uuid(), &self.device)
                .await;
        }

        res
    }

    async fn stop_node(&mut self, id: Id, start_idx: NodeIndex) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let space = petgraph::visit::DfsPostOrder::new(self.nodes.relations(), start_idx);

        let relations = space
            .iter(self.nodes.relations())
            .filter(|idx| {
                // Current deployment
                let current = id.is_deployment().then_some(id);

                // filter the dependents deployment, and check that are not started
                let other_deployment = self.nodes.has_dependant_deployments(*idx, current);

                if other_deployment {
                    debug!(
                        "skipping {:?} which has another running deployment ",
                        self.nodes.get_id(*idx)
                    );
                }

                other_deployment
            })
            .map(|idx| {
                self.nodes
                    .get_id(idx)
                    .copied()
                    .ok_or(ServiceError::MissingRelation)
            })
            .collect::<Result<Vec<Id>>>()?;

        debug_assert_eq!(
            relations
                .last()
                .and_then(|id| self.nodes.node(id))
                .expect("there should be at least the starting node")
                .idx,
            start_idx
        );

        for id in relations {
            let node = self
                .nodes
                .node_mut(&id)
                .ok_or_else(|| ServiceError::Missing {
                    id,
                    ctx: "stop node",
                })?;

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
    use astarte_device_sdk::FromEvent;
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

        let store = StateStore::open(tempdir.path().join("containers/state.json"))
            .await
            .unwrap();

        let mut service = Service::new(client, store, device);

        let reference = "docker.io/nginx:stable-alpine-slim";
        let create_image_req = create_image_request_event(id.to_string(), reference, "");

        let req = ContainerRequest::from_event(create_image_req).unwrap();

        service.on_event(req).await.unwrap();

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

        let store = StateStore::open(tempdir.path().join("state.json"))
            .await
            .unwrap();

        let mut service = Service::new(client, store, device);

        let create_volume_req = create_volume_request_event(id, "local", &["foo=bar", "some="]);

        let req = ContainerRequest::from_event(create_volume_req).unwrap();

        service.on_event(req).await.unwrap();

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

        let store = StateStore::open(tempdir.path().join("state.json"))
            .await
            .unwrap();

        let mut service = Service::new(client, store, device);

        let create_network_req = create_network_request_event(id, "bridged");

        let req = ContainerRequest::from_event(create_network_req).unwrap();

        service.on_event(req).await.unwrap();

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

        let req = ContainerRequest::from_event(create_container_req).unwrap();

        service.on_event(req).await.unwrap();

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

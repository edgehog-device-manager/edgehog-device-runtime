// This file is part of Edgehog.
//
// Copyright 2023 SECO Mind Srl
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

//! Service to receive the Astarte events.

use std::{
    borrow::Borrow,
    collections::{hash_map::Entry, HashMap},
    fmt::{Debug, Display},
    ops::{Deref, DerefMut},
    pin::pin,
    sync::Arc,
};

use astarte_device_sdk::{
    event::FromEventError, store::PropertyStore, DeviceClient, DeviceEvent, Error as AstarteError,
    FromEvent,
};
use futures::Future;
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use tracing::{debug, error, info, instrument, warn};

use crate::{
    container::Container,
    error::DockerError,
    image::Image,
    network::Network,
    properties::{
        container::AvailableContainer, image::AvailableImage, network::AvailableNetwork,
        volume::AvailableVolume, AvailableProp, LoadProp, PropError,
    },
    request::{
        CreateContainer, CreateImage, CreateNetwork, CreateRequests, CreateVolume, ReqError,
    },
    volume::Volume,
    Docker,
};

type SResult<T> = Result<T, ServiceError>;

/// Error from the [`Service`].
#[non_exhaustive]
#[derive(Debug, displaydoc::Display, thiserror::Error)]
pub enum ServiceError {
    /// error converting event
    FromEvent(#[from] FromEventError),
    /// error from the Astarte sdk
    Astarte(#[from] AstarteError),
    /// docker operation failed
    Docker(#[source] DockerError),
    /// couldn't get the property for {interface}
    Prop {
        /// Interface of the property
        interface: &'static str,
        /// Backtrace
        #[source]
        backtrace: PropError,
    },
    /// node {0} is missing
    MissingNode(String),
    /// relation is missing given the index
    MissingRelation,
    /// couldn't process request
    Request(#[from] ReqError),
    /// couldn't create for missing node {0}
    Create(String),
}

impl<T> From<T> for ServiceError
where
    T: Into<DockerError>,
{
    fn from(value: T) -> Self {
        ServiceError::Docker(value.into())
    }
}

/// Manages the state of the Nodes
#[derive(Debug, Clone)]
pub struct Service<S> {
    client: Docker,
    device: DeviceClient<S>,
    nodes: Nodes,
}

impl<S> Service<S> {
    /// Create a new service
    #[must_use]
    pub fn new(client: Docker, device: DeviceClient<S>) -> Self {
        Self {
            client,
            device,
            nodes: Nodes::new(),
        }
    }

    /// Initialize the service, it will load all the already stored properties
    #[instrument]
    pub async fn init(client: Docker, device: DeviceClient<S>) -> Result<Self, ServiceError>
    where
        S: PropertyStore,
    {
        let mut services = Self::new(client, device);

        AvailableImage::load_resource(&services.device, &mut services.nodes).await?;
        AvailableVolume::load_resource(&services.device, &mut services.nodes).await?;
        AvailableNetwork::load_resource(&services.device, &mut services.nodes).await?;
        AvailableContainer::load_resource(&services.device, &mut services.nodes).await?;

        Ok(services)
    }

    /// Handles an event from the image.
    #[instrument(skip(self))]
    pub async fn on_event(&mut self, event: DeviceEvent) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        let event = CreateRequests::from_event(event)?;

        match event {
            CreateRequests::Image(req) => self.create_image(req).await,
            CreateRequests::Volume(req) => self.create_volume(req).await,
            CreateRequests::Network(req) => self.create_network(req).await,
            CreateRequests::Container(req) => self.create_container(req).await,
        }
    }

    /// Store the create image request
    #[instrument(skip(self))]
    async fn create_image(&mut self, req: CreateImage) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        let id = Id::new(req.id);

        let device = pin!(&self.device);

        self.nodes
            .add_node(
                id,
                |id, idx| async move {
                    let image = Image::with_repo(req.name, req.tag, req.repo);

                    let mut node = Node::new(id, idx);

                    node.store(&device, image).await?;

                    Ok(node)
                },
                &[],
            )
            .await?;

        Ok(())
    }

    /// Store the create image request
    #[instrument(skip(self))]
    async fn create_volume(&mut self, req: CreateVolume) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        let id = Id::new(req.id.clone());

        let device = pin!(&self.device);

        self.nodes
            .add_node(
                id,
                |id, idx| async move {
                    let volume = Volume::try_from(req)?;

                    let mut node = Node::new(id, idx);

                    node.store(&device, volume).await?;

                    Ok(node)
                },
                &[],
            )
            .await?;

        Ok(())
    }

    /// Store the create network request
    #[instrument(skip(self))]
    async fn create_network(&mut self, req: CreateNetwork) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        let id = Id::new(req.id.clone());

        let device = pin!(&self.device);

        self.nodes
            .add_node(
                id,
                |id, idx| async move {
                    let network = Network::from(req);

                    let mut node = Node::new(id, idx);

                    node.store(&device, network).await?;

                    Ok(node)
                },
                &[],
            )
            .await?;

        Ok(())
    }

    /// Store the create container request
    #[instrument(skip(self))]
    async fn create_container(&mut self, req: CreateContainer) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        let id = Id::new(req.id.clone());

        let device = pin!(&self.device);

        self.nodes
            .add_node(
                id,
                |id, idx| async move {
                    let container = ContainerNode::try_from(req)?;

                    let mut node = Node::new(id, idx);

                    node.store(&device, container).await?;

                    Ok(node)
                },
                &[],
            )
            .await?;

        Ok(())
    }

    /// Will start an application
    #[instrument(skip(self))]
    pub async fn start(&mut self, id: &str) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        let id = Id::new(id.to_string());

        let start_idx = self
            .nodes
            .nodes
            .get(&id)
            .filter(|node| !node.inner.is_missing())
            .ok_or_else(|| ServiceError::MissingNode(id.to_string()))?
            .idx;

        let mut space =
            petgraph::visit::DfsPostOrder::new(&self.nodes.relations.relations, start_idx);

        while let Some(idx) = space.next(&self.nodes.relations.relations) {
            let id = self
                .nodes
                .relations
                .node_weight(idx)
                .ok_or(ServiceError::MissingRelation)?;

            let node = self
                .nodes
                .nodes
                .get_mut(id)
                .ok_or_else(|| ServiceError::MissingNode(id.to_string()))?;

            node.create(&self.device, &self.client).await?;
        }

        Ok(())
    }
}

/// Struct used to keep the collection of nodes and the relations between them.
///
/// The Nodes struct is needed since on the [`Service`] we need to have mixed mutable and immutable
/// reference to the various parts of the struct.
#[derive(Debug, Clone)]
pub(crate) struct Nodes {
    nodes: HashMap<Id, Node>,
    relations: Graph,
}

impl Nodes {
    fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            relations: Graph::new(),
        }
    }

    pub(crate) fn get_idx(&self, id: &str) -> Option<NodeIndex> {
        self.nodes.get(id).map(|node| node.idx)
    }

    pub(crate) fn add_node_sync<F>(
        &mut self,
        id: Id,
        f: F,
        deps: &[NodeIndex],
    ) -> Result<&mut Node, ServiceError>
    where
        F: FnOnce(Id, NodeIndex) -> Result<Node, ServiceError>,
    {
        match self.nodes.entry(id) {
            Entry::Occupied(node) => {
                self.relations.relate(node.get().idx, deps)?;

                Ok(node.into_mut())
            }
            Entry::Vacant(entry) => {
                let id = entry.key().clone();

                self.relations.add_sync(id.clone(), deps, |idx| {
                    let node = (f)(id, idx)?;

                    Ok(entry.insert(node))
                })
            }
        }
    }

    pub(crate) async fn add_node<F, O>(
        &mut self,
        id: Id,
        f: F,
        deps: &[NodeIndex],
    ) -> Result<&mut Node, ServiceError>
    where
        F: FnOnce(Id, NodeIndex) -> O,
        O: Future<Output = Result<Node, ServiceError>>,
    {
        match self.nodes.entry(id) {
            Entry::Occupied(node) => {
                self.relations.relate(node.get().idx, deps)?;

                Ok(node.into_mut())
            }
            Entry::Vacant(entry) => {
                let id = entry.key().clone();

                self.relations
                    .add(id.clone(), deps, |idx| async move {
                        let node = (f)(id, idx).await?;

                        Ok(entry.insert(node))
                    })
                    .await
            }
        }
    }
}

#[derive(Debug, Clone)]
struct Graph {
    relations: StableDiGraph<Id, ()>,
}

impl Graph {
    fn new() -> Self {
        Self {
            relations: StableDiGraph::new(),
        }
    }

    async fn add<F, O, R>(&mut self, id: Id, deps: &[NodeIndex], f: F) -> SResult<R>
    where
        F: FnOnce(NodeIndex) -> O,
        O: Future<Output = SResult<R>>,
    {
        let idx = self.add_node(id);

        let res = (f)(idx).await;

        self.relate_ok_or_rm(res, idx, deps)
    }

    fn add_sync<F, O>(&mut self, id: Id, deps: &[NodeIndex], f: F) -> SResult<O>
    where
        F: FnOnce(NodeIndex) -> SResult<O>,
    {
        let idx = self.add_node(id);

        let res = (f)(idx);

        self.relate_ok_or_rm(res, idx, deps)
    }

    fn relate_ok_or_rm<R>(
        &mut self,
        res: SResult<R>,
        idx: NodeIndex,
        deps: &[NodeIndex],
    ) -> Result<R, ServiceError> {
        let res = res.and_then(|r| {
            self.relate(idx, deps)?;

            Ok(r)
        });

        if res.is_err() {
            debug!("error adding node, removing relation");

            self.remove_node(idx);
        }

        res
    }

    #[instrument]
    pub(crate) fn relate(
        &mut self,
        node: NodeIndex,
        deps: &[NodeIndex],
    ) -> Result<(), ServiceError> {
        // We need to check each node or it will panic if not existing
        if !self.relations.contains_node(node) {
            error!("node is missing");

            return Err(ServiceError::MissingRelation);
        }

        for dep in deps {
            let dep = *dep;
            if !self.relations.contains_node(dep) {
                error!("dependency {} is missing", dep.index());

                return Err(ServiceError::MissingRelation);
            }

            self.relations.add_edge(node, dep, ());
        }

        Ok(())
    }
}

impl Deref for Graph {
    type Target = StableDiGraph<Id, ()>;

    fn deref(&self) -> &Self::Target {
        &self.relations
    }
}

impl DerefMut for Graph {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.relations
    }
}

impl Borrow<StableDiGraph<Id, ()>> for Graph {
    fn borrow(&self) -> &StableDiGraph<Id, ()> {
        &self.relations
    }
}

impl Default for Nodes {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Node {
    id: Id,
    idx: NodeIndex,
    inner: State,
}

impl Node {
    pub(crate) fn new(id: Id, idx: NodeIndex) -> Self {
        Self {
            id,
            idx,
            inner: State::Missing,
        }
    }

    pub(crate) fn with_state(id: Id, idx: NodeIndex, state: State) -> Self {
        Self {
            id,
            idx,
            inner: state,
        }
    }

    // TODO Store outside in the try node, add missing and then pass the type into store
    #[instrument]
    async fn store<S, T>(&mut self, device: &DeviceClient<S>, inner: T) -> Result<(), ServiceError>
    where
        S: PropertyStore,
        T: Into<NodeType> + Debug,
    {
        self.inner.store(&self.id, device, inner).await
    }

    #[instrument]
    async fn create<S>(
        &mut self,
        device: &DeviceClient<S>,
        client: &Docker,
    ) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        self.inner.create(&self.id, device, client).await
    }
}

/// State of the object for the request.
#[derive(Debug, Clone, Default)]
pub(crate) enum State {
    #[default]
    Missing,
    Stored(NodeType),
    Created(NodeType),
    Started(NodeType),
}

impl State {
    #[instrument]
    async fn store<S, T>(
        &mut self,
        id: &Id,
        device: &DeviceClient<S>,
        node: T,
    ) -> Result<(), ServiceError>
    where
        S: PropertyStore,
        T: Into<NodeType> + Debug,
    {
        match self {
            State::Missing => {
                let mut node = node.into();

                node.store(id, device).await?;

                self.store_node(node);
            }
            State::Stored(_) | State::Created(_) | State::Started(_) => {
                debug!("node already created");
            }
        }
        Ok(())
    }

    #[instrument]
    async fn create<S>(
        &mut self,
        id: &Id,
        device: &DeviceClient<S>,
        client: &Docker,
    ) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        match self {
            State::Missing => return Err(ServiceError::Create(id.to_string())),
            State::Stored(node) => {
                node.create(id, device, client).await?;

                self.next();
            }
            State::Created(_) | State::Started(_) => {
                debug!("node already created");
            }
        }
        Ok(())
    }

    #[instrument]
    fn store_node(&mut self, node: NodeType) {
        debug_assert!(self.is_missing(), "BUG: state is not missing");

        match std::mem::replace(self, State::Stored(node)) {
            State::Missing => {
                debug!("state stored");
            }
            State::Stored(_) | State::Created(_) | State::Started(_) => {
                error!("BUG: state replaced");
            }
        }
    }

    fn next(&mut self) {
        debug_assert!(!self.is_missing(), "BUG: state still missing");

        *self = match std::mem::take(self) {
            State::Missing => {
                warn!("BUG: state still missing");
                State::Missing
            }
            State::Stored(node) => State::Created(node),
            State::Created(node) | State::Started(node) => State::Started(node),
        };
    }

    /// Returns `true` if the state is [`Missing`].
    ///
    /// [`Missing`]: State::Missing
    #[must_use]
    pub(crate) fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum NodeType {
    Image(Image<String>),
    Volume(Volume<String>),
    Network(Network<String>),
    Container(ContainerNode),
}

impl NodeType {
    #[instrument]
    async fn store<S>(&mut self, id: &Id, device: &DeviceClient<S>) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        match &self {
            NodeType::Image(image) => {
                AvailableImage::with_image(id, image).store(device).await?;

                info!("stored image with id {}", id);
            }
            NodeType::Volume(volume) => {
                AvailableVolume::with_volume(id, volume)
                    .store(device)
                    .await?;

                info!("stored volume with id {}", id);
            }
            NodeType::Network(network) => {
                AvailableNetwork::with_network(id, network)
                    .store(device)
                    .await?;

                info!("stored network with id {}", id);
            }
            NodeType::Container(container) => {
                AvailableContainer::with_container(id, &container.container, &container.volumes)
                    .store(device)
                    .await?;

                info!("stored container with id {}", id);
            }
        }

        Ok(())
    }

    #[instrument]
    async fn create<S>(
        &mut self,
        id: &Id,
        device: &DeviceClient<S>,
        client: &Docker,
    ) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        match self {
            NodeType::Image(ref image) => {
                image.pull(client).await?;

                AvailableImage::with_pulled(id, image, true)
                    .store(device)
                    .await?;
            }
            NodeType::Volume(ref volume) => {
                volume.create(client).await?;

                AvailableVolume::with_created(id, volume, true)
                    .store(device)
                    .await?;
            }
            NodeType::Network(ref mut network) => {
                network.inspect_or_create(client).await?;

                AvailableNetwork::with_network(id, network)
                    .store(device)
                    .await?;
            }
            NodeType::Container(ref mut container) => {
                container.container.inspect_or_create(client).await?;

                AvailableContainer::with_container(id, &container.container, &container.volumes)
                    .store(device)
                    .await?;
            }
        }

        Ok(())
    }
}

impl From<Image<String>> for NodeType {
    fn from(value: Image<String>) -> Self {
        Self::Image(value)
    }
}

impl From<Volume<String>> for NodeType {
    fn from(value: Volume<String>) -> Self {
        Self::Volume(value)
    }
}

impl From<Network<String>> for NodeType {
    fn from(value: Network<String>) -> Self {
        Self::Network(value)
    }
}

impl From<ContainerNode> for NodeType {
    fn from(value: ContainerNode) -> Self {
        Self::Container(value)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ContainerNode {
    container: Container<String>,
    volumes: Vec<Id>,
}

impl ContainerNode {
    pub(crate) fn new(container: Container<String>, volumes: Vec<Id>) -> Self {
        Self { container, volumes }
    }
}

/// Id of the nodes in the Service graph
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Id(Arc<str>);

impl Id {
    /// Create a new ID
    pub(crate) fn new(id: String) -> Self {
        Self(id.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for Id {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Borrow<str> for Id {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

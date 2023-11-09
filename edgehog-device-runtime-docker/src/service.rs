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
    fmt::Display,
    ops::Deref,
    sync::Arc,
};

use astarte_device_sdk::{
    event::FromEventError, store::PropertyStore, DeviceClient, DeviceEvent, Error as AstarteError,
    FromEvent,
};
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use tracing::{debug, info, instrument, warn};

use crate::{
    error::DockerError,
    image::Image,
    network::Network,
    properties::{
        image::AvailableImage, network::AvailableNetwork, volume::AvailableVolume, AvailableProp,
        LoadProp, PropError,
    },
    request::{CreateImage, CreateNetwork, CreateRequests, CreateVolume, ReqError},
    volume::Volume,
    Docker,
};

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
        }
    }

    /// Store the create image request
    #[instrument(skip(self))]
    async fn create_image(&mut self, req: CreateImage) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        let id = Id::new(req.id);

        let node = self.nodes.add_node(id, |id, idx| {
            let image = Image::with_repo(req.name, req.tag, req.repo);

            Node::new(id, idx, State::Missing, image)
        });

        node.store(&self.device).await?;

        Ok(())
    }

    /// Store the create image request
    #[instrument(skip(self))]
    async fn create_volume(&mut self, req: CreateVolume) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        let id = Id::new(req.id.clone());

        let node = self.nodes.try_add_node(id, |id, idx| {
            let volume = Volume::try_from(req)?;

            Ok(Node::new(id, idx, State::Missing, volume))
        })?;

        node.store(&self.device).await?;

        Ok(())
    }

    /// Store the create network request
    #[instrument(skip(self))]
    async fn create_network(&mut self, req: CreateNetwork) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        let id = Id::new(req.id.clone());

        let node = self.nodes.add_node(id, |id, idx| {
            let network = Network::from(req);

            Node::new(id, idx, State::Missing, network)
        });

        node.store(&self.device).await?;

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
            .filter(|node| node.state == State::Missing)
            .ok_or_else(|| ServiceError::MissingNode(id.to_string()))?
            .idx;

        let mut space = petgraph::visit::DfsPostOrder::new(&self.nodes.relations, start_idx);

        while let Some(idx) = space.next(&self.nodes.relations) {
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
    relations: StableDiGraph<Id, ()>,
}

impl Nodes {
    fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            relations: StableDiGraph::new(),
        }
    }

    pub(crate) fn add_node<F>(&mut self, id: Id, f: F) -> &mut Node
    where
        F: FnOnce(Id, NodeIndex) -> Node,
    {
        self.nodes.entry(id).or_insert_with_key(|id| {
            let idx = self.relations.add_node(id.clone());

            f(id.clone(), idx)
        })
    }

    fn try_add_node<F>(&mut self, id: Id, f: F) -> Result<&mut Node, ServiceError>
    where
        F: FnOnce(Id, NodeIndex) -> Result<Node, ServiceError>,
    {
        match self.nodes.entry(id) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),

            Entry::Vacant(entry) => {
                let idx = self.relations.add_node(entry.key().clone());

                // Reset the relations in case of errors
                let node = match f(entry.key().clone(), idx) {
                    Ok(node) => node,
                    Err(err) => {
                        debug!("error creating node, removing relation");

                        self.relations.remove_node(idx);

                        return Err(err);
                    }
                };

                Ok(entry.insert(node))
            }
        }
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
    state: State,
    inner: NodeType,
}

impl Node {
    pub(crate) fn new<T>(id: Id, idx: NodeIndex, state: State, node: T) -> Self
    where
        T: Into<NodeType>,
    {
        Self {
            id,
            idx,
            state,
            inner: node.into(),
        }
    }

    #[instrument]
    async fn store<S>(&mut self, device: &DeviceClient<S>) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        match &self.inner {
            NodeType::Image { image, .. } => {
                AvailableImage::with_image(&self.id, image)
                    .store(device)
                    .await?;

                info!("stored image with id {}", self.id);
            }
            NodeType::Volume { volume } => {
                AvailableVolume::with_volume(&self.id, volume)
                    .store(device)
                    .await?;
            }
            NodeType::Network { network } => {
                AvailableNetwork::with_network(&self.id, network)
                    .store(device)
                    .await?;
            }
        }

        self.state.store();

        Ok(())
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
        match self.inner {
            NodeType::Image { ref image, .. } => {
                image.pull(client).await?;

                AvailableImage::with_pulled(&self.id, image, true)
                    .store(device)
                    .await?;
            }
            NodeType::Volume { ref volume } => {
                volume.create(client).await?;

                AvailableVolume::with_created(&self.id, volume, true)
                    .store(device)
                    .await?;
            }
            NodeType::Network { ref mut network } => {
                network.inspect_or_create(client).await?;

                AvailableNetwork::with_network(&self.id, network)
                    .store(device)
                    .await?;
            }
        }

        self.state.created();

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) enum NodeType {
    Image { image: Image<String> },
    Volume { volume: Volume<String> },
    Network { network: Network<String> },
}

impl From<Image<String>> for NodeType {
    fn from(value: Image<String>) -> Self {
        Self::Image { image: value }
    }
}

impl From<Volume<String>> for NodeType {
    fn from(value: Volume<String>) -> Self {
        Self::Volume { volume: value }
    }
}

impl From<Network<String>> for NodeType {
    fn from(value: Network<String>) -> Self {
        Self::Network { network: value }
    }
}

/// State of the object for the request.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum State {
    #[default]
    Missing,
    Stored,
    Created,
}

impl State {
    #[instrument]
    fn store(&mut self) {
        match self {
            State::Missing => *self = State::Stored,
            State::Stored | State::Created => {
                debug!("state already stored");
            }
        }
    }

    #[instrument]
    fn created(&mut self) {
        match self {
            State::Missing | State::Stored => *self = State::Created,
            State::Created => {
                debug!("state already created");
            }
        }
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

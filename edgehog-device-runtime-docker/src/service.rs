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

use std::{borrow::Borrow, collections::HashMap, fmt::Display, ops::Deref, rc::Rc};

use astarte_device_sdk::{
    error::Error as AstarteError, event::FromEventError, properties::PropAccess,
    store::PropertyStore, DeviceClient, DeviceEvent, FromEvent,
};
use itertools::Itertools;
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use tracing::{debug, info, instrument, warn};

use crate::{
    error::DockerError,
    image::Image,
    properties::{AvailableImage, PropError},
    request::{CreateImage, CreateRequests},
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
    /// Node {0} is missing
    MissingNode(String),
    /// Relation is missing given the index
    MissingRelation,
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

        services.load_images().await?;

        Ok(services)
    }

    #[instrument]
    async fn load_images(&mut self) -> Result<(), ServiceError>
    where
        S: PropertyStore,
    {
        let av_imgs_sprop = self
            .device
            .interface_props(AvailableImage::INTERFACE)
            .await?;

        let av_imgs = av_imgs_sprop
            .iter()
            .map(AvailableImage::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| ServiceError::Prop {
                interface: AvailableImage::INTERFACE,
                backtrace: err,
            })?;

        av_imgs
            .into_iter()
            .chunk_by(|av_img| av_img.id)
            .into_iter()
            .map(|(_, group)| group.reduce(AvailableImage::merge))
            .filter_map(|av_img| av_img.map(|av_img| (av_img.id, av_img)))
            .try_for_each(|(id, av_img)| -> Result<(), ServiceError> {
                let img = Image::try_from(av_img).map_err(|err| ServiceError::Prop {
                    interface: AvailableImage::INTERFACE,
                    backtrace: err,
                })?;

                let id = Id::new(id.to_string());

                self.nodes
                    .add_node(id, |id, idx| Node::new(id, idx, State::Stored, img));

                Ok(())
            })
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

    /// Will start an application
    #[instrument(skip(self))]
    pub async fn star(&mut self, id: &str) -> Result<(), ServiceError>
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
struct Nodes {
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

    fn add_node<F>(&mut self, id: Id, f: F) -> &mut Node
    where
        F: FnOnce(Id, NodeIndex) -> Node,
    {
        self.nodes.entry(id).or_insert_with_key(|id| {
            let idx = self.relations.add_node(id.clone());

            f(id.clone(), idx)
        })
    }
}

impl Default for Nodes {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
struct Node {
    id: Id,
    idx: NodeIndex,
    state: State,
    inner: NodeType,
}

impl Node {
    fn new<T>(id: Id, idx: NodeIndex, state: State, node: T) -> Self
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
        match &self.inner {
            NodeType::Image { image, .. } => {
                image.pull(client).await?;

                AvailableImage::with_pulled(&self.id, image, true)
                    .store(device)
                    .await?;
            }
        }

        self.state.created();

        Ok(())
    }
}

#[derive(Debug, Clone)]
enum NodeType {
    Image { image: Image<String> },
}

impl From<Image<String>> for NodeType {
    fn from(value: Image<String>) -> Self {
        Self::Image { image: value }
    }
}

/// State of the object for the request.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
enum State {
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
struct Id(Rc<String>);

impl Id {
    /// Create a new ID
    fn new(id: String) -> Self {
        Self(Rc::new(id))
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

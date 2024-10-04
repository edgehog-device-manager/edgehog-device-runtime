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
    borrow::Borrow,
    fmt::{Debug, Display},
    ops::Deref,
    sync::Arc,
};

use astarte_device_sdk::{
    event::FromEventError, properties::PropAccess, Client, DeviceEvent, Error as AstarteError,
    FromEvent,
};
use petgraph::stable_graph::NodeIndex;
use tracing::{debug, info, instrument};

use crate::{
    error::DockerError,
    properties::PropError,
    requests::{CreateRequests, ReqError},
    Docker,
};

use self::node::Nodes;

pub(crate) mod node;

type SResult<T> = Result<T, ServiceError>;

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
    MissingNode(String),
    /// relation is missing given the index
    MissingRelation,
    /// couldn't process request
    Request(#[from] ReqError),
    /// couldn't store for existing node {0}
    Store(String),
    /// couldn't create for missing node {0}
    Create(String),
    /// couldn't start for missing node {0}
    Start(String),
    /// couldn't operate on missing node {0}
    Missing(String),
    /// error from the Astarte SDK
    Astarte(#[from] AstarteError),
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

/// Manages the state of the Nodes
#[derive(Debug, Clone)]
pub struct Service<D>
where
    D: Debug + Client + PropAccess,
{
    client: Docker,
    device: D,
    nodes: Nodes,
}

impl<D> Service<D>
where
    D: Debug + Client + PropAccess + Sync,
{
    /// Create a new service
    #[must_use]
    pub fn new(client: Docker, device: D) -> Self {
        Self {
            client,
            device,
            nodes: Nodes::new(),
        }
    }

    /// Initialize the service, it will load all the already stored properties
    #[instrument(skip_all)]
    pub async fn init(client: Docker, device: D) -> SResult<Self> {
        let services = Self::new(client, device);

        // TODO: load the resources

        Ok(services)
    }

    /// Handles an event from the image.
    #[instrument(skip_all)]
    pub async fn on_event(&mut self, event: DeviceEvent) -> SResult<()>
    where
        D: Client,
    {
        let event = CreateRequests::from_event(event)?;

        match event {}
    }

    /// Will start an application
    #[instrument(skip(self))]
    pub async fn start(&mut self, id: &str) -> SResult<()> {
        let id = Id::new(id.to_string());

        let start_idx = self
            .nodes
            .node(&id)
            .ok_or_else(|| ServiceError::MissingNode(id.to_string()))?
            .idx;

        let mut space = petgraph::visit::DfsPostOrder::new(self.nodes.relations(), start_idx);

        let mut relations = Vec::new();
        while let Some(idx) = space.next(self.nodes.relations()) {
            let id = self
                .nodes
                .get_id(idx)
                .ok_or(ServiceError::MissingRelation)?
                .clone();

            relations.push(id);
        }

        for id in relations {
            let node = self
                .nodes
                .node_mut(&id)
                .ok_or_else(|| ServiceError::MissingNode(id.to_string()))?;

            node.up(&self.device, &self.client).await?;
        }

        Ok(())
    }
}

/// A node containing the [`State`], [`Id`] of the resource and index of the dependencies.
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

    #[instrument(skip_all)]
    async fn store<D, T>(&mut self, device: &D, inner: T) -> SResult<()>
    where
        D: Debug + Client + Sync,
        T: Into<NodeType> + Debug,
    {
        self.inner.store(&self.id, device, inner).await
    }

    #[instrument(skip_all)]
    async fn create<D>(&mut self, device: &D, client: &Docker) -> SResult<()>
    where
        D: Debug + Client + Sync,
    {
        self.inner.create(&self.id, device, client).await
    }

    #[instrument(skip_all)]
    async fn start<D>(&mut self, device: &D, client: &Docker) -> SResult<()>
    where
        D: Debug + Client + Sync,
    {
        self.inner.start(&self.id, device, client).await
    }

    #[instrument(skip_all)]
    async fn up<D>(&mut self, device: &D, client: &Docker) -> SResult<()>
    where
        D: Debug + Client + Sync,
    {
        self.inner.up(&self.id, device, client).await
    }
}

/// State of the object for the request.
#[derive(Debug, Clone, Default)]
pub(crate) enum State {
    #[default]
    Missing,
    Stored(NodeType),
    Created(NodeType),
    Up(NodeType),
}

impl State {
    #[instrument(skip_all)]
    async fn store<D, T>(&mut self, id: &Id, device: &D, node: T) -> SResult<()>
    where
        D: Debug + Client + Sync,
        T: Into<NodeType> + Debug,
    {
        match self {
            State::Missing => {
                let node = node.into();

                node.store(id, device).await?;

                *self = State::Stored(node);

                debug!("node {id} stored");

                Ok(())
            }
            State::Stored(_) | State::Created(_) | State::Up(_) => {
                Err(ServiceError::Store(id.to_string()))
            }
        }
    }

    #[instrument(skip_all)]
    async fn create<D>(&mut self, id: &Id, device: &D, client: &Docker) -> SResult<()>
    where
        D: Debug + Client + Sync,
    {
        match self {
            State::Missing => return Err(ServiceError::Create(id.to_string())),
            State::Stored(node) => {
                node.create(id, device, client).await?;

                self.map_into(State::Created)?;

                debug!("node {id} created");
            }
            State::Created(_) | State::Up(_) => {
                debug!("node already created");
            }
        }
        Ok(())
    }

    #[instrument(skip_all)]
    async fn start<D>(&mut self, id: &Id, device: &D, client: &Docker) -> SResult<()>
    where
        D: Debug + Client + Sync,
    {
        match self {
            State::Missing | State::Stored(_) => Err(ServiceError::Start(id.to_string())),
            State::Created(node) => {
                node.start(id, device, client).await?;

                self.map_into(State::Up)?;

                debug!("node {id} started");

                Ok(())
            }
            State::Up(_) => {
                debug!("node already up");

                Ok(())
            }
        }
    }

    #[instrument(skip_all)]
    async fn up<D>(&mut self, id: &Id, device: &D, client: &Docker) -> SResult<()>
    where
        D: Debug + Client + Sync,
    {
        match &*self {
            State::Missing => return Err(ServiceError::Missing(id.to_string())),
            State::Stored(_) => {
                self.create(id, device, client).await?;
                self.start(id, device, client).await?;
            }
            State::Created(_) => {
                self.start(id, device, client).await?;
            }
            State::Up(_) => {
                debug!("node already up");
            }
        }

        info!("node {id} up");

        Ok(())
    }

    fn map_into<F>(&mut self, f: F) -> SResult<()>
    where
        F: FnOnce(NodeType) -> State,
    {
        *self = match std::mem::take(self) {
            // It's safe to return the error on missing since the taken one is also missing
            State::Missing => return Err(ServiceError::BugMissing),
            State::Stored(node) | State::Created(node) | State::Up(node) => f(node),
        };

        Ok(())
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
pub(crate) enum NodeType {}

impl NodeType {
    #[instrument(skip_all)]
    async fn store<D>(&mut self, _id: &Id, _device: &D) -> SResult<()>
    where
        D: Debug + Client + Sync,
    {
        unimplemented!()
    }

    #[instrument(skip_all)]
    async fn create<D>(&mut self, _id: &Id, _device: &D, _client: &Docker) -> SResult<()>
    where
        D: Debug + Client + Sync,
    {
        unimplemented!()
    }

    #[instrument(skip_all)]
    async fn start<D>(&mut self, id: &Id, device: &D, client: &Docker) -> SResult<()>
    where
        D: Debug + Client + Sync,
    {
        unimplemented!()
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

/// A resource in the nodes struct.
pub(crate) trait Resource: Into<NodeType> {
    fn dependencies(&self) -> Result<Vec<String>, ServiceError>;
}

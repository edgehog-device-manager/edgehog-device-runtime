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
    event::FromEventError, properties::PropAccess, DeviceEvent, Error as AstarteError, FromEvent,
};
use tracing::{debug, instrument};

use crate::{
    error::DockerError,
    image::Image,
    properties::{Client, PropError},
    requests::{image::CreateImage, CreateRequests, ReqError},
    store::Resource,
    store::{StateStore, StateStoreError},
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
    MissingNode(String),
    /// relation is missing given the index
    MissingRelation,
    /// couldn't process request
    Request(#[from] ReqError),
    /// couldn't create for missing node {0}
    Create(String),
    /// couldn't start for missing node {0}
    Start(String),
    /// couldn't operate on missing node {0}
    Missing(String),
    /// couldn't store the resource state
    StateStore(#[from] StateStoreError),
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
    pub async fn init(client: Docker, store: StateStore, device: D) -> Result<Self> {
        let mut service = Self::new(client, store, device);

        for value in service.store.load().await? {
            let id = Id::new(&value.id);

            match value.resource {
                Some(Resource::Image(state)) => {
                    let image = Image::from(state);

                    service.nodes.add_node_sync(
                        id,
                        |id, node_idx| {
                            Ok(Node::with_state(id, node_idx, State::Stored(image.into())))
                        },
                        &[],
                    )?;
                }
                None => {
                    debug!("addming missing resource");

                    service.nodes.add_node_sync(
                        id,
                        |id, node_idx| Ok(Node::new(id, node_idx)),
                        &[],
                    )?;
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
        let id = Id::new(&req.id);

        let device = &self.device;
        let store = &mut self.store;

        self.nodes
            .add_node(
                id,
                |id, idx| async move {
                    let image = Image::from(req);

                    let mut node = Node::new(id, idx);

                    node.store(store, device, image).await?;

                    Ok(node)
                },
                &[],
            )
            .await?;

        Ok(())
    }

    /// Will start an application
    #[instrument(skip(self))]
    pub async fn start(&mut self, id: &str) -> Result<()>
    where
        D: Debug + Client + Sync + 'static,
    {
        let id = Id::new(id);

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

        self.store.store(&self.nodes).await?;

        Ok(())
    }
}

/// Id of the nodes in the Service graph
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Id(Arc<str>);

impl Id {
    /// Create a new ID
    pub(crate) fn new(id: &str) -> Self {
        Self(Arc::from(id))
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

impl AsRef<str> for Id {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use astarte_device_sdk::store::SqliteStore;
    use astarte_device_sdk_mock::mockall::Sequence;
    use astarte_device_sdk_mock::MockDeviceClient;
    use resource::NodeType;
    use tempfile::TempDir;

    use crate::requests::image::tests::create_image_request_event;

    use super::*;

    #[tokio::test]
    async fn should_add_an_image() {
        let tempdir = TempDir::new().unwrap();

        let id = "5b705c7b-e6c7-4455-ba9b-a081be020c43";

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
        let create_image_req = create_image_request_event(id, reference, "");

        service.on_event(create_image_req).await.unwrap();

        let id = Id::new(id);
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
}

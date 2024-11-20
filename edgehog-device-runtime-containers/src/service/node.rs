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

//! Node in the containers

use std::fmt::Debug;

use petgraph::stable_graph::NodeIndex;
use tracing::{debug, info, instrument, trace, warn};

use crate::{
    properties::{
        container::{AvailableContainers, ContainerStatus},
        deployment::{AvailableDeployments, DeploymentStatus},
        image::AvailableImage,
        network::AvailableNetworks,
        volume::AvailableVolumes,
        AvailableProp,
    },
    service::{resource::State, ServiceError},
    Docker,
};

use super::{
    resource::{NodeResource, NodeType},
    Client, Id, Result,
};

/// A node containing the [`State`], [`Id`] of the resource and index of the dependencies.
///
/// Its a node in the graph of container resources, with the dependencies of other nodes as edges.
#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub(crate) id: Id,
    pub(super) idx: NodeIndex,
    pub(crate) resource: Option<NodeResource>,
}

impl Node {
    pub(crate) fn new(id: Id, idx: NodeIndex, resource: Option<NodeResource>) -> Self {
        Self { id, idx, resource }
    }

    #[instrument(skip_all)]
    pub(crate) async fn publish<D>(&mut self, device: &D) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let resource = self.resource.as_mut().ok_or(ServiceError::Missing {
            id: self.id,
            ctx: "publish",
        })?;

        if resource.state != State::Received {
            warn!(
                "trying to publish resource {} with state {}",
                self.id, resource.state
            );

            return Ok(());
        }

        match resource.value {
            NodeType::Image(_) => {
                AvailableImage::new(&self.id).send(device, false).await;
            }
            NodeType::Volume(_) => AvailableVolumes::new(&self.id).send(device, false).await,
            NodeType::Network(_) => {
                AvailableNetworks::new(&self.id).send(device, false).await;
            }
            NodeType::Container(_) => {
                AvailableContainers::new(&self.id)
                    .send(device, ContainerStatus::Received)
                    .await;
            }
            NodeType::Deployment => {
                AvailableDeployments::new(&self.id)
                    .send(device, DeploymentStatus::Stopped)
                    .await;
            }
        }

        resource.state = State::Published;

        info!("resource {} stored", self.id);

        Ok(())
    }

    // TODO: store before sending the prop
    #[instrument(skip_all)]
    async fn create<D>(&mut self, device: &D, client: &Docker) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let resource = self.resource.as_mut().ok_or(ServiceError::Missing {
            id: self.id,
            ctx: "create",
        })?;

        match &mut resource.value {
            NodeType::Image(image) => {
                image.inspect_or_create(client).await?;

                AvailableImage::new(&self.id).send(device, true).await;
            }
            NodeType::Volume(volume) => {
                volume.inspect_or_create(client).await?;

                AvailableVolumes::new(&self.id).send(device, true).await;
            }
            NodeType::Network(network) => {
                network.inspect_or_create(client).await?;

                AvailableNetworks::new(&self.id).send(device, true).await;
            }
            NodeType::Container(container) => {
                container.inspect_or_create(client).await?;

                AvailableContainers::new(&self.id)
                    .send(device, ContainerStatus::Created)
                    .await;
            }
            NodeType::Deployment => {}
        }

        resource.state = State::Created;

        info!("resource {} created", self.id);

        Ok(())
    }

    // TODO: store before sending the prop
    #[instrument(skip_all)]
    async fn start<D>(&mut self, device: &D, client: &Docker) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let id = &self.id;
        let resource = self.resource.as_mut().ok_or(ServiceError::Missing {
            id: self.id,
            ctx: "start",
        })?;

        match &resource.value {
            NodeType::Image(_) | NodeType::Volume(_) | NodeType::Network(_) => {
                trace!("resource {id} does't need starting");
            }
            NodeType::Container(container) => {
                container.start(client).await?;

                AvailableContainers::new(id)
                    .send(device, ContainerStatus::Running)
                    .await;
            }
            NodeType::Deployment => {
                AvailableDeployments::new(id)
                    .send(device, DeploymentStatus::Started)
                    .await;
            }
        }

        resource.state = State::Up;

        info!("resource {id} started");

        Ok(())
    }

    #[instrument(skip_all)]
    async fn remove<D>(&mut self, device: &D, client: &Docker) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let id = &self.id;
        let resource = self.resource.as_mut().ok_or(ServiceError::Missing {
            id: self.id,
            ctx: "start",
        })?;

        match &resource.value {
            NodeType::Image(image) => {
                image.remove(client).await?;

                AvailableImage::new(id).unset(device).await;
            }
            NodeType::Volume(volume) => {
                volume.remove(client).await?;

                AvailableVolumes::new(id).unset(device).await;
            }
            NodeType::Network(network) => {
                network.remove(client).await?;

                AvailableNetworks::new(id).unset(device).await;
            }
            NodeType::Container(container) => {
                container.remove(client).await?;

                AvailableContainers::new(id).unset(device).await;
            }
            NodeType::Deployment => {
                AvailableDeployments::new(id).unset(device).await;
            }
        }

        self.resource = None;

        info!("resource {id} removed");

        Ok(())
    }

    #[instrument(skip_all)]
    pub(crate) async fn stop<D>(&mut self, device: &D, client: &Docker) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let id = &self.id;
        let resource = self.resource.as_mut().ok_or(ServiceError::Missing {
            id: self.id,
            ctx: "stop",
        })?;

        match resource.state {
            State::Received | State::Published => {
                warn!("stopping resource {id}, but was never created");
            }
            State::Created | State::Up => {
                trace!("stopping ");
            }
        }

        match &resource.value {
            NodeType::Image(_) | NodeType::Volume(_) | NodeType::Network(_) => {
                trace!("nothing to do for {id}");
            }
            NodeType::Container(container) => {
                let exists = container.stop(client).await?;
                debug_assert!(exists.is_some());

                AvailableContainers::new(id)
                    .send(device, ContainerStatus::Stopped)
                    .await;
            }
            NodeType::Deployment => {
                AvailableDeployments::new(id)
                    .send(device, DeploymentStatus::Stopped)
                    .await;
            }
        }

        resource.state = State::Created;

        info!("resource {id} stopped");

        Ok(())
    }

    #[instrument(skip_all)]
    pub(crate) async fn up<D>(&mut self, device: &D, client: &Docker) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let resource = self.resource.as_ref().ok_or(ServiceError::Missing {
            id: self.id,
            ctx: "up",
        })?;

        match &resource.state {
            State::Received | State::Published => {
                if resource.state == State::Received {
                    warn!("starting resource {} which is not stored", self.id);
                }

                self.create(device, client).await?;
                self.start(device, client).await?;
            }
            State::Created => {
                self.start(device, client).await?;
            }
            State::Up => {
                debug!("resource {} is already up", self.id);
            }
        }

        info!("resource {} up", self.id);

        Ok(())
    }

    #[instrument(skip_all)]
    pub(crate) async fn delete<D>(&mut self, device: &D, client: &Docker) -> Result<()>
    where
        D: Client + Sync + 'static,
    {
        let resource = self.resource.as_ref().ok_or(ServiceError::Missing {
            id: self.id,
            ctx: "up",
        })?;

        if resource.state == State::Received {
            warn!("removing resource {} which is not stored", self.id);
        }

        self.stop(device, client).await?;
        self.remove(device, client).await?;

        info!("resource {} deleted", self.id);

        Ok(())
    }

    /// Returns true if the resource is present and the state is up
    pub(crate) fn is_up(&self) -> bool {
        matches!(
            self.resource,
            Some(NodeResource {
                state: State::Up,
                ..
            })
        )
    }
}
